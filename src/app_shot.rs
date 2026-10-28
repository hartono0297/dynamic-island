//! The island's side of the screenshot preview (Run 8): a new clipboard
//! image becomes a candidate, a short-lived worker looks for its file in
//! the Screenshots folder, the big notice shows it, and its buttons copy it
//! again, keep it or move it to the Recycle Bin. A child of `app`, so it
//! works on `App` directly. See docs/memory/screenshot-preview.md.
//!
//! Power: no watcher and no polling. One worker lists the folder when an
//! image arrives, and `TIMER_SHOT_RETRY` (a one-shot) runs at most two more
//! within about 3 s; then nothing runs until the next image.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::{App, WM_APP_EVENT, poster};
use crate::clipboard::image::{self, Thumb};
use crate::clipboard::store::ClipImage;
use crate::clipboard::win as clipboard_win;
use crate::config::ShotIgnore;
use crate::i18n::{self, Key, tr};
use crate::model::{Event, Notice, NoticeKind, ShotAction, View};
use crate::screenshot::win::{self, DeleteJob};
use crate::screenshot::{self, ClipCandidate, FileCandidate, FileOutcome};
use crate::ui::render::ShotView;

/// One-shot: look in the Screenshots folder once more (Snipping Tool
/// writes the file a moment after the clipboard). 20–22 are the
/// clipboard's.
pub(super) const TIMER_SHOT_RETRY: usize = 23;
/// The waits before the second and third look: about 3 s in all.
const RETRY_MS: [u32; 2] = [1000, 1800];
/// The preview's thumbnail, made from the full DIB: its 230 × 136 box at 2×.
const PREVIEW_MAX: (u32, u32) = (460, 272);

/// What the preview draws of a screenshot.
pub(super) struct ShotLook {
    pub id: u64,
    pub thumb: Thumb,
    pub w: u32,
    pub h: u32,
    /// The matched file's size, else the bitmap's.
    pub bytes: u64,
}

/// One screenshot: the clipboard candidate, its file once found.
pub(super) struct ShotState {
    pub look: ShotLook,
    /// Its history card; `None` with the history off.
    pub entry: Option<u64>,
    pub clip: ClipCandidate,
    /// The clipboard sequence number read with it (or after "Salin lagi").
    pub sequence: u32,
    /// The original, for "Salin lagi" (the history's own copy, shared).
    pub dib: Option<Arc<Vec<u8>>>,
    pub file: Option<FileCandidate>,
    /// Folder looks made so far.
    pub checks: u8,
}

/// The screenshots the island knows of.
#[derive(Default)]
pub(super) struct Shots {
    /// The one whose preview was raised (shows or waits its turn).
    pub shown: Option<ShotState>,
    /// A new image without the screenshot flag: it shows only once its file
    /// is found; a browser's "Copy image" never is.
    pub waiting: Option<ShotState>,
    /// The shot whose folder looks still run (the newest only).
    pub checking: Option<u64>,
    pub retry_timer: bool,
    pub next_id: u64,
    /// The Screenshots folder as the last look found it.
    pub dir: Option<PathBuf>,
    /// The last preview's look, kept so its fade-out still draws it.
    pub faded: Option<ShotLook>,
    /// When the image before the newest arrived (unix ms): files before
    /// the newest's arrival may be that one's (`screenshot::claim_from`).
    pub last_at_ms: Option<i64>,
    /// The files matched to screenshots, newest last (`TAKEN_CAP`): never
    /// matched to another.
    pub taken: Vec<PathBuf>,
}

/// How many matched files `Shots::taken` remembers.
const TAKEN_CAP: usize = 16;

impl Shots {
    fn get_mut(&mut self, id: u64) -> Option<&mut ShotState> {
        [self.shown.as_mut(), self.waiting.as_mut()]
            .into_iter()
            .flatten()
            .find(|s| s.look.id == id)
    }
}

/// What the renderer draws for `view`'s preview.
pub(super) fn shot_view<'a>(shots: &'a Shots, view: &View) -> Option<ShotView<'a>> {
    let View::Notice(Notice {
        kind: NoticeKind::Screenshot { shot, .. },
        ..
    }) = view
    else {
        return None;
    };
    let look = shots
        .shown
        .as_ref()
        .map(|s| &s.look)
        .filter(|l| l.id == *shot)
        .or(shots.faded.as_ref().filter(|l| l.id == *shot))?;
    Some(ShotView {
        id: look.id,
        thumb: &look.thumb,
        w: look.w,
        h: look.h,
        bytes: look.bytes,
    })
}

/// The history card that goes with a screenshot's "Hapus": its own card,
/// never one the user pinned (a PrtSc of an unchanged screen can bring an
/// old pinned card to the front).
pub(super) fn card_to_remove(
    history: &crate::clipboard::History,
    entry: Option<u64>,
) -> Option<u64> {
    entry.filter(|&id| history.get(id).is_some_and(|c| !c.pinned))
}

/// The notice after "Simpan": "Disimpan · Pictures\Screenshots" when the
/// screenshot has its file in `dir`; without one (a PrtSc with no auto-save)
/// "Belum ada file · tetap di clipboard", so no one thinks it is on disk.
pub(super) fn kept_notice(has_file: bool, dir: Option<PathBuf>) -> Notice {
    let (title, body) = match (has_file, dir) {
        (true, Some(dir)) => (Key::ShotSaved, screenshot::folder_label(&dir)),
        _ => (Key::ShotNotSaved, tr(Key::ShotOnClipboard).into()),
    };
    Notice {
        key: "shot-saved".into(),
        source: tr(Key::ShotTitle).into(),
        title: tr(title).into(),
        body,
        kind: NoticeKind::ShotSaved,
    }
}

/// A preview that ran its full time untouched: the screenshot to delete,
/// only while "Jika diabaikan" is Hapus now (set back to Simpan meanwhile,
/// it is kept). Simpan, the default, never deletes.
pub(super) fn delete_when_ignored(on_ignore: ShotIgnore, id: u64) -> Option<u64> {
    (on_ignore == ShotIgnore::Delete).then_some(id)
}

/// A change in Settings → Clipboard → Screenshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShotSetting {
    Preview(bool),
    OnIgnore(ShotIgnore),
    Secs(u32),
}

impl App {
    /// A new clipboard image (`entry` its history card, `None` with the
    /// history off). The flag (only bitmap formats) shows the preview at
    /// once; otherwise it shows only if the folder holds its file. Nothing
    /// with the preview off or over a fullscreen app (then it is simply
    /// kept). True when the preview was raised now (the caller refreshes).
    pub(super) fn shot_from_image(
        &mut self,
        entry: Option<u64>,
        img: &ClipImage,
        sequence: u32,
    ) -> bool {
        if !self.config.screenshot.preview || self.state.fullscreen() {
            return false;
        }
        self.shots.next_id += 1;
        let id = self.shots.next_id;
        let at_ms = screenshot::unix_ms(SystemTime::now());
        let dib = img.full.clone();
        let thumb = dib
            .as_deref()
            .and_then(|d| {
                image::dib_info(d)
                    .map(|i| image::thumbnail_fit(d, &i, PREVIEW_MAX.0, PREVIEW_MAX.1))
            })
            .unwrap_or_else(|| img.thumb.clone());
        let state = ShotState {
            look: ShotLook {
                id,
                thumb,
                w: img.width,
                h: img.height,
                bytes: dib.as_ref().map_or(0, |d| d.len() as u64),
            },
            entry,
            clip: ClipCandidate {
                at_ms,
                from_ms: screenshot::claim_from(at_ms, self.shots.last_at_ms.replace(at_ms)),
                w: img.width,
                h: img.height,
                hash: screenshot::thumb_hash(&img.thumb.bgra, img.thumb.w, img.thumb.h),
            },
            sequence,
            dib,
            file: None,
            checks: 0,
        };
        // The shown screenshot's file not found yet: one last look for it
        // now, as the new image takes over the retries (a browser's "Copy
        // image" must not leave the preview without its file).
        if let Some(prev) = self.shots.checking
            && self
                .shots
                .shown
                .as_ref()
                .is_some_and(|s| s.look.id == prev && s.file.is_none())
        {
            self.look_for_shot_file(prev);
        }
        self.stop_shot_retry();
        self.shots.waiting = Some(state);
        self.shots.checking = Some(id);
        self.look_for_shot_file(id);
        if img.screenshot {
            self.show_waiting_shot();
            return true;
        }
        false
    }

    /// The waiting shot's preview goes up; it replaces the one before.
    fn show_waiting_shot(&mut self) {
        let Some(state) = self.shots.waiting.take() else {
            return;
        };
        if let Some(old) = self.shots.shown.replace(state)
            && self.shots.checking == Some(old.look.id)
        {
            self.stop_shot_retry();
        }
        let id = self.shots.shown.as_ref().map_or(0, |s| s.look.id);
        let cfg = self.config.screenshot;
        let notice = Notice {
            key: format!("shot-{id}"),
            source: tr(Key::ShotTitle).into(),
            title: tr(Key::ShotTitle).into(),
            body: String::new(),
            kind: NoticeKind::Screenshot {
                shot: id,
                secs: cfg.show_secs,
                delete_if_ignored: cfg.on_ignore == ShotIgnore::Delete,
            },
        };
        self.apply(Event::Notice(notice), Instant::now());
    }

    /// One look in the Screenshots folder for `id`, on a short-lived thread.
    fn look_for_shot_file(&mut self, id: u64) {
        let Some(state) = self.shots.get_mut(id) else {
            return;
        };
        state.checks += 1;
        let clip = state.clip;
        let taken = self.shots.taken.clone();
        if !win::spawn_check(
            id,
            clip,
            taken,
            self.tx.clone(),
            poster(self.hwnd, WM_APP_EVENT),
        ) {
            self.shots.checking = None;
        }
    }

    fn stop_shot_retry(&mut self) {
        if std::mem::take(&mut self.shots.retry_timer) {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_SHOT_RETRY);
            }
        }
    }

    /// `TIMER_SHOT_RETRY`: the next look, for the shot still being looked for.
    pub(super) fn on_shot_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SHOT_RETRY);
        }
        self.shots.retry_timer = false;
        if let Some(id) = self.shots.checking {
            self.look_for_shot_file(id);
        }
    }

    /// A look came back. A match is this screenshot's file (a waiting one
    /// now shows); none tries again up to twice, then gives up: an image
    /// without the flag and without a file was no screenshot.
    pub(super) fn on_shot_folder(
        &mut self,
        shot: u64,
        dir: Option<PathBuf>,
        found: Option<FileCandidate>,
    ) {
        if dir.is_some() {
            self.shots.dir = dir;
        }
        let looking = self.shots.checking == Some(shot);
        let Some(state) = self.shots.get_mut(shot) else {
            return;
        };
        match found {
            Some(file) => {
                state.look.bytes = file.bytes;
                let path = file.path.clone();
                state.file = Some(file);
                let taken = &mut self.shots.taken;
                if !taken.contains(&path) {
                    taken.push(path);
                    if taken.len() > TAKEN_CAP {
                        taken.remove(0);
                    }
                }
                if looking {
                    self.shots.checking = None;
                    self.stop_shot_retry();
                }
                let waiting = self
                    .shots
                    .waiting
                    .as_ref()
                    .is_some_and(|s| s.look.id == shot);
                if waiting {
                    if self.config.screenshot.preview && !self.state.fullscreen() {
                        self.show_waiting_shot();
                    } else {
                        self.shots.waiting = None;
                    }
                }
                // The drain refreshes the island: the size line now says
                // the file's.
            }
            None => {
                let checks = state.checks;
                if looking && let Some(&ms) = RETRY_MS.get(usize::from(checks) - 1) {
                    unsafe {
                        SetTimer(Some(self.hwnd), TIMER_SHOT_RETRY, ms, None);
                    }
                    self.shots.retry_timer = true;
                    return;
                }
                if looking {
                    self.shots.checking = None;
                }
                if self
                    .shots
                    .waiting
                    .as_ref()
                    .is_some_and(|s| s.look.id == shot)
                {
                    self.shots.waiting = None;
                }
            }
        }
    }

    /// After the reducer: a preview that is gone (expired, replaced, a
    /// button, a game) forgets its screenshot; its look stays for the fade.
    pub(super) fn follow_shot(&mut self) {
        let Some(id) = self.shots.shown.as_ref().map(|s| s.look.id) else {
            return;
        };
        if self.state.has_shot(id) {
            return;
        }
        if let Some(old) = self.shots.shown.take() {
            self.shots.faded = Some(old.look);
        }
        if self.shots.checking == Some(id) {
            self.shots.checking = None;
            self.stop_shot_retry();
        }
    }

    /// A button or the thumbnail, from the reducer (which already closed
    /// the preview for the buttons).
    pub(super) fn perform_shot(&mut self, action: ShotAction) -> Option<Event> {
        let lang = i18n::lang();
        match action {
            ShotAction::Copy(id) => {
                let state = self.shots.shown.as_mut().filter(|s| s.look.id == id)?;
                let dib = state
                    .dib
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| image::dib_from_thumb(&state.look.thumb));
                let (w, h, entry) = (state.look.w, state.look.h, state.entry);
                let sequence = clipboard_win::write(self.hwnd, &clipboard_win::image_formats(dib))?;
                state.sequence = sequence;
                self.clip.own_sequence = Some(sequence);
                if let Some(entry) = entry
                    && self
                        .clip
                        .history
                        .bump(entry, chrono::Local::now().timestamp())
                {
                    self.schedule_clip_save();
                }
                Some(Event::Notice(Notice {
                    key: "clip-copied".into(),
                    source: tr(Key::ClipboardTitle).into(),
                    title: tr(Key::ClipCopied).into(),
                    body: i18n::clip_image_meta(lang, true, w, h),
                    kind: NoticeKind::Copied,
                }))
            }
            ShotAction::Keep(id) => {
                let state = self.shots.shown.as_ref().filter(|s| s.look.id == id)?;
                let dir = state
                    .file
                    .as_ref()
                    .and_then(|f| f.path.parent().map(PathBuf::from))
                    .or_else(|| self.shots.dir.clone());
                Some(Event::Notice(kept_notice(state.file.is_some(), dir)))
            }
            ShotAction::Open(id) => {
                let state = self.shots.shown.as_ref().filter(|s| s.look.id == id)?;
                // Without a file there is nothing to open.
                if let Some(file) = &state.file {
                    win::open_file(&file.path);
                }
                None
            }
            ShotAction::Delete(id) => self.delete_shot(id),
            ShotAction::DeleteIgnored(id) => {
                let id = delete_when_ignored(self.config.screenshot.on_ignore, id)?;
                self.delete_shot(id)
            }
        }
    }

    /// "Hapus": the clipboard is emptied only while it still holds this
    /// image; its history card goes; the file goes to the Recycle Bin on a
    /// worker (one last look first if none was matched). The notice follows
    /// in `on_shot_recycled`.
    fn delete_shot(&mut self, id: u64) -> Option<Event> {
        if self.shots.shown.as_ref().is_none_or(|s| s.look.id != id) {
            return None;
        }
        let state = self.shots.shown.take()?;
        self.shots.faded = Some(ShotLook {
            thumb: state.look.thumb.clone(),
            ..state.look
        });
        if self.shots.checking == Some(id) {
            self.shots.checking = None;
            self.stop_shot_retry();
        }
        let emptied = match clipboard_win::empty_if_unchanged(self.hwnd, state.sequence) {
            Some(sequence) => {
                self.clip.own_sequence = Some(sequence);
                true
            }
            None => false,
        };
        if let Some(entry) = card_to_remove(&self.clip.history, state.entry)
            && self.clip.history.delete(entry)
        {
            if self.clip.selected == Some(entry) {
                self.clip.selected = None;
            }
            self.clamp_clip_scroll();
            self.schedule_clip_save();
            self.redraw_clip();
        }
        let had_file = state.file.is_some();
        let job = DeleteJob {
            shot: id,
            file: state.file.map(|f| f.path),
            clip: state.clip,
            taken: self.shots.taken.clone(),
            emptied,
        };
        if win::spawn_delete(job, self.tx.clone(), poster(self.hwnd, WM_APP_EVENT)) {
            return None;
        }
        Some(Event::ShotRecycled {
            shot: id,
            file: if had_file {
                FileOutcome::Failed
            } else {
                FileOutcome::NoFile
            },
            emptied,
        })
    }

    /// The delete worker is done: "Dihapus" (Bata at rest) says what went.
    pub(super) fn on_shot_recycled(&mut self, file: FileOutcome, emptied: bool, now: Instant) {
        let (title, body) = i18n::shot_deleted(i18n::lang(), file, emptied);
        let notice = Notice {
            key: "shot-deleted".into(),
            source: tr(Key::ShotTitle).into(),
            title: title.into(),
            body,
            kind: NoticeKind::ShotDeleted,
        };
        self.apply(Event::Notice(notice), now);
    }

    /// Settings → Clipboard → Screenshot. The listener runs while the
    /// history or the preview is on.
    pub(super) fn on_shot_setting(&mut self, change: ShotSetting) {
        let cfg = &mut self.config.screenshot;
        match change {
            ShotSetting::Preview(on) => cfg.preview = on,
            ShotSetting::OnIgnore(v) => cfg.on_ignore = v,
            ShotSetting::Secs(n) => {
                let (min, max, _) = crate::config::SHOT_SECS_RANGE;
                cfg.show_secs = n.clamp(min, max);
            }
        }
        self.save_config();
        if matches!(change, ShotSetting::Preview(_)) {
            self.sync_clip_listener();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{card_to_remove, delete_when_ignored, kept_notice};
    use crate::clipboard::History;
    use crate::config::{Config, ShotIgnore};

    #[test]
    fn an_ignored_preview_deletes_only_with_hapus_set_now() {
        // The default is Simpan: an ignored preview keeps its file.
        let default = Config::default().screenshot.on_ignore;
        assert_eq!(default, ShotIgnore::Keep);
        assert_eq!(delete_when_ignored(default, 3), None);
        assert_eq!(delete_when_ignored(ShotIgnore::Delete, 3), Some(3));
    }
    use crate::clipboard::image::tests::dib24;
    use crate::clipboard::store::{ClipSnapshot, Ingested, ingest};
    use crate::screenshot::{ClipCandidate, FileCandidate, pick_match, thumb_hash};

    #[test]
    fn one_screenshot_by_clipboard_and_file_is_one_card() {
        let mut h = History::new(50);
        let snap = ClipSnapshot {
            dib: Some(dib24(1280, 720)),
            only_bitmaps: true,
            sequence: 7,
            ..ClipSnapshot::default()
        };
        let Ingested::Added { id, image: true } = ingest(&mut h, snap.clone(), 100) else {
            panic!("an image entry");
        };
        // Its file in the folder matches (the same picture's hash), and
        // adds nothing to the history.
        let thumb = &h.items()[0].image().unwrap().thumb;
        let hash = thumb_hash(&thumb.bgra, thumb.w, thumb.h);
        assert!(hash.is_some());
        let clip = ClipCandidate {
            at_ms: 100_000,
            from_ms: 100_000 - crate::screenshot::BEFORE_MS,
            w: 1280,
            h: 720,
            hash,
        };
        let file = FileCandidate {
            path: "Screenshot 2026-10-28 101500.png".into(),
            at_ms: 100_700,
            bytes: 250_880,
            w: 1280,
            h: 720,
            hash,
        };
        assert!(pick_match(&clip, std::slice::from_ref(&file)).is_some());
        assert_eq!(h.len(), 1);
        // The same screenshot again (PrtSc twice): the card moves up, no copy.
        assert_eq!(ingest(&mut h, snap, 101), Ingested::Bumped { id });
        assert_eq!(h.len(), 1);
        assert!(h.items()[0].image().is_some_and(|i| i.screenshot));
    }

    #[test]
    fn hapus_keeps_a_pinned_card() {
        use crate::clipboard::store::tests::image;
        use crate::clipboard::store::{Ingested, ingest};
        let mut h = History::new(50);
        let Ingested::Added { id, .. } = ingest(&mut h, image(1280, 720), 0) else {
            panic!("added");
        };
        assert_eq!(card_to_remove(&h, Some(id)), Some(id));
        h.set_pinned(id, true);
        assert_eq!(card_to_remove(&h, Some(id)), None, "pinned on purpose");
        assert_eq!(card_to_remove(&h, None), None);
    }

    #[test]
    fn simpan_without_a_file_does_not_say_saved() {
        let dir = PathBuf::from(r"C:\Users\x\Pictures\Screenshots");
        let saved = kept_notice(true, Some(dir.clone()));
        assert_eq!(
            (saved.title.as_str(), saved.body.as_str()),
            ("Disimpan", r"Pictures\Screenshots")
        );
        // A PrtSc without auto-save left no file: it is only on the clipboard.
        for dir in [None, Some(dir)] {
            let n = kept_notice(false, dir);
            assert_eq!(
                (n.title.as_str(), n.body.as_str()),
                ("Belum ada file", "tetap di clipboard")
            );
        }
    }
}
