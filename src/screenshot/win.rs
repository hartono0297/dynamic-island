//! The Win32 side of the screenshot preview: where the Screenshots folder
//! is, the short-lived workers that list it and move a file to the Recycle
//! Bin, and opening a file. Only the running island calls these; no test and
//! no demo does (they would read the owner's folder or delete a file).
//!
//! No watcher and no polling: each worker lists the folder once and ends.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Screenshots, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use super::{ClipCandidate, FileOutcome, find_in, thumb_hash};
use crate::clipboard::image::thumb_size;
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

/// What "Hapus" hands the worker.
pub struct DeleteJob {
    pub shot: u64,
    /// The file matched so far; `None` runs one last check first.
    pub file: Option<PathBuf>,
    pub clip: ClipCandidate,
    /// Files other screenshots matched: never this one's.
    pub taken: Vec<PathBuf>,
    /// The clipboard was emptied (on the UI thread, before this).
    pub emptied: bool,
}

/// Moves the screenshot's file to the Recycle Bin on a short-lived thread
/// (COM single-threaded there, as `IFileOperation` wants) through the
/// cleaner's `recycle`: never a permanent delete. Without a matched file it
/// looks once more, right now, and acts only on what that finds. Posts
/// `Event::ShotRecycled`. False when the thread could not start.
pub fn spawn_delete(job: DeleteJob, tx: Sender<Event>, wake: impl Fn() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("shot-delete".into())
        .spawn(move || {
            let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let file = job.file.or_else(|| {
                screenshots_dir()
                    .and_then(|d| find_in(&d, &job.clip, &job.taken, file_hash))
                    .map(|f| f.path)
            });
            let outcome = match file {
                None => FileOutcome::NoFile,
                Some(path) if crate::clean::recycle(std::slice::from_ref(&path)) == 0 => {
                    FileOutcome::Recycled
                }
                Some(_) => FileOutcome::Failed,
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
