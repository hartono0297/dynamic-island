//! The Win32 side of the screenshot preview: where the Screenshots folder
//! is, the short-lived workers that list it and move a file to the Recycle
//! Bin, and opening a file. Only the running island calls these; no demo
//! does, and the one test here writes a PNG into a folder it makes under the
//! system temp dir (the others would read the owner's folder or delete a
//! file).
//!
//! No watcher and no polling: the check worker lists the folder once and
//! ends; the Simpan and Hapus workers keep looking (a few times a second)
//! only until the screenshot's match window has closed, then end.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Screenshots, KF_FLAG_DEFAULT, SHCreateMemStream, SHGetKnownFolderPath, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use super::{
    Arrivals, ClipCandidate, FileOutcome, SETTLE_MS, confirmed, delete_outcome, find_confirmed_in,
    find_in, thumb_hash, unix_ms, wait_for,
};
use crate::clipboard::image::{self, thumb_size};
use crate::model::Event;

/// `thumb_hash` of an image file, decoded through WIC at the clipboard
/// thumbnail's size (`thumb_size`), so both sides hash alike. `None` when it
/// cannot be decoded (still being written, an unknown format). Needs COM on
/// the calling thread; the file is closed again before this returns.
fn file_hash(path: &Path) -> Option<u64> {
    unsafe {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let name = HSTRING::from(path.as_os_str());
        let decoder = wic
            .CreateDecoderFromFilename(&name, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        let (tw, th) = thumb_size(w.max(1), h.max(1));
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(&frame, tw, th, WICBitmapInterpolationModeFant)
            .ok()?;
        let converter = wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
        let mut pixels = vec![0u8; (tw * th * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), tw * 4, &mut pixels)
            .ok()?;
        thumb_hash(&pixels, tw, th)
    }
}

/// The Screenshots folder from the Known Folder (it follows OneDrive's
/// redirection); never a guessed path. `None` when Windows cannot name it.
pub fn screenshots_dir() -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path.filter(|p| p.is_absolute())
    }
}

/// Lists the Screenshots folder once on a short-lived thread and posts
/// `Event::ShotFolder` with the file of `clip`, if one matches. False when
/// the thread could not start.
pub fn spawn_check(
    shot: u64,
    clip: ClipCandidate,
    taken: Vec<PathBuf>,
    tx: Sender<Event>,
    wake: impl Fn() + Send + 'static,
) -> bool {
    std::thread::Builder::new()
        .name("shot-check".into())
        .spawn(move || {
            // For WIC (the files' hashes).
            let inited = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let dir = screenshots_dir();
            let found = dir
                .as_deref()
                .and_then(|d| find_in(d, &clip, &taken, file_hash));
            if inited {
                unsafe { CoUninitialize() };
            }
            if tx.send(Event::ShotFolder { shot, dir, found }).is_ok() {
                wake();
            }
        })
        .is_ok()
}

/// `bgra` (top-down, `w` x `h`, opaque) as PNG bytes, encoded by WIC into a
/// memory stream: no file is touched. Needs COM on the calling thread.
fn encode_png(w: u32, h: u32, bgra: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let encoder = wic
            .CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())
            .ok()?;
        let stream = SHCreateMemStream(None)?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;
        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options).ok()?;
        let frame = frame?;
        frame.Initialize(options.as_ref()).ok()?;
        frame.SetSize(w, h).ok()?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format).ok()?;
        frame.WritePixels(h, w * 4, bgra).ok()?;
        frame.Commit().ok()?;
        encoder.Commit().ok()?;
        let mut len = 0u64;
        stream.Seek(0, STREAM_SEEK_END, Some(&mut len)).ok()?;
        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut bytes = vec![0u8; usize::try_from(len).ok()?];
        let mut read = 0u32;
        stream
            .Read(
                bytes.as_mut_ptr().cast(),
                u32::try_from(len).ok()?,
                Some(&mut read),
            )
            .ok()
            .ok()?;
        (read as usize == bytes.len()).then_some(bytes)
    }
}

/// Writes `bytes` to a new file at `path` (create new: an existing file is
/// never opened). A file this made and could not fill is removed.
fn write_at(path: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .ok()?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if written.is_ok() {
        Some(path.to_path_buf())
    } else {
        let _ = std::fs::remove_file(path);
        None
    }
}

/// The pixels of a clipboard DIB at full size, top-down BGRA (the
/// thumbnail sampler at scale 1 is the exact pixel).
fn full_pixels(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let info = image::dib_info(dib)?;
    let full = image::thumbnail_fit(dib, &info, info.w, info.h);
    Some((full.w, full.h, full.bgra))
}

/// Looks for `clip`'s file until its window has closed (`wait_for`): Simpan
/// and Hapus may be clicked before Snipping Tool has written it. Only a file
/// whose hash is confirmed counts. Needs COM on the calling thread (WIC).
fn wait_for_file(clip: ClipCandidate, taken: &[PathBuf], arrivals: &Arrivals) -> Option<PathBuf> {
    let dir = screenshots_dir()?;
    let end = || arrivals.window_end(clip.at_ms);
    wait_for(
        || end() + SETTLE_MS,
        || unix_ms(SystemTime::now()),
        || {
            let clip = ClipCandidate {
                to_ms: end(),
                ..clip
            };
            find_confirmed_in(&dir, &clip, taken, file_hash).map(|f| f.path)
        },
        |ms| std::thread::sleep(Duration::from_millis(ms)),
    )
}

/// What "Simpan" hands the worker.
pub struct SaveJob {
    pub shot: u64,
    pub dib: Arc<Vec<u8>>,
    /// Where a new file goes, named on the UI thread (and already in `taken`
    /// there, so the next screenshot cannot claim it).
    pub path: PathBuf,
    pub clip: ClipCandidate,
    /// Files other screenshots matched (this job's own `path` among them).
    pub taken: Vec<PathBuf>,
    pub arrivals: Arrivals,
}

/// "Simpan" for a screenshot with no file yet, on a short-lived thread:
/// first it waits for Snipping Tool's own file (its window); found, that is
/// the saved file. Else it writes the picture as a PNG at `job.path` (the
/// folder made if it is missing). Posts `Event::ShotSaveDone` with the path,
/// `None` when anything failed. False when the thread could not start.
pub fn spawn_save(job: SaveJob, tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("shot-save".into())
        .spawn(move || {
            let inited = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let SaveJob {
                shot,
                dib,
                path,
                clip,
                mut taken,
                arrivals,
            } = job;
            taken.retain(|p| *p != path);
            let path = wait_for_file(clip, &taken, &arrivals).or_else(|| {
                let dir = path.parent()?;
                std::fs::create_dir_all(dir).ok()?;
                let (w, h, bgra) = full_pixels(&dib)?;
                let png = encode_png(w, h, &bgra)?;
                // The UI thread named it; a file that took the name since
                // is never opened (create new), the save then fails.
                write_at(&path, &png)
            });
            if inited {
                unsafe { CoUninitialize() };
            }
            if tx.send(Event::ShotSaveDone { shot, path }).is_ok() {
                wake();
            }
        })
        .is_ok()
}

/// What "Hapus" hands the worker.
pub struct DeleteJob {
    pub shot: u64,
    /// The file matched so far (its hash is checked again before it goes);
    /// `None`, or a hash that does not confirm, waits for the file first.
    pub file: Option<PathBuf>,
    pub clip: ClipCandidate,
    /// Files other screenshots matched: never this one's.
    pub taken: Vec<PathBuf>,
    pub arrivals: Arrivals,
    /// The clipboard was emptied (on the UI thread, before this).
    pub emptied: bool,
}

/// Moves the screenshot's file to the Recycle Bin on a short-lived thread
/// (COM single-threaded there, as `IFileOperation` wants) through the
/// cleaner's `recycle`: never a permanent delete. Only a file whose hash
/// confirms the clipboard's goes: the matched one if it does, else the one
/// that turns up before its window closes (`wait_for_file`). A file gone by
/// then is no file. Posts `Event::ShotRecycled`. False when the thread could
/// not start.
pub fn spawn_delete(job: DeleteJob, tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("shot-delete".into())
        .spawn(move || {
            let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            // The screenshot's own file is in `taken` (it was matched): left
            // there, the wait could never find it again if its hash was
            // unreadable a moment ago.
            let mut taken = job.taken;
            if let Some(own) = &job.file {
                taken.retain(|p| p != own);
            }
            let file = job
                .file
                .filter(|p| confirmed(job.clip.hash, file_hash(p)))
                .or_else(|| wait_for_file(job.clip, &taken, &job.arrivals));
            let outcome = match file {
                None => FileOutcome::NoFile,
                Some(path) => delete_outcome(path.exists(), || {
                    crate::clean::recycle(std::slice::from_ref(&path))
                }),
            };
            if inited {
                unsafe { CoUninitialize() };
            }
            let event = Event::ShotRecycled {
                shot: job.shot,
                file: outcome,
                emptied: job.emptied,
            };
            if tx.send(event).is_ok() {
                wake();
            }
        })
        .is_ok()
}

/// Opens `path` in its default app (Snipping Tool or Photos). The island
/// never takes the foreground for it.
pub fn open_file(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    unsafe {
        ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::image::tests::dib24;

    #[test]
    fn a_picture_round_trips_through_a_png_in_a_temp_folder() {
        let dir = std::env::temp_dir().join(format!("vale-shot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (w, h) = (40u32, 24u32);
        let mut bgra = Vec::new();
        for y in 0..h {
            for x in 0..w {
                bgra.extend([(x * 6) as u8, (y * 10) as u8, 200, 255]);
            }
        }
        let inited = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let png = encode_png(w, h, &bgra);
        if inited {
            unsafe { CoUninitialize() };
        }
        let png = png.expect("WIC encoded it");
        assert_eq!(crate::screenshot::png_size(&png), Some((w, h)));
        let path = write_at(&dir.join("t.png"), &png).expect("written");
        assert_eq!(std::fs::read(&path).expect("the file"), png);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_never_opens_a_file_that_is_there() {
        let dir = std::env::temp_dir().join(format!("vale-shot-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let first = dir.join("Screenshot 2026-10-28 143012.png");
        std::fs::write(&first, b"theirs").expect("a file already there");
        // The UI thread named it, a file took the name since: it is never
        // opened, and the save fails instead.
        assert_eq!(write_at(&first, b"ours"), None);
        assert_eq!(
            std::fs::read(&first).expect("theirs"),
            b"theirs",
            "untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dib_becomes_full_size_pixels() {
        let (w, h, bgra) = full_pixels(&dib24(30, 20)).expect("a 24 bpp DIB");
        assert_eq!((w, h), (30, 20), "never scaled");
        assert_eq!(bgra.len(), 30 * 20 * 4);
        assert!(bgra.chunks(4).all(|p| p[3] == 255), "opaque");
        assert!(full_pixels(&[0u8; 8]).is_none(), "not a DIB");
    }
}
