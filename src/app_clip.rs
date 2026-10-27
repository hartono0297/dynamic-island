//! The island's side of the clipboard history: the listener, reading a
//! change into the history, copying an entry back, the encrypted file, the
//! hotkey and the Clipboard page's search field and selection. A child of
//! `app`, so it works on `App` directly.
//!
//! Power: no repeating timer. A clipboard change while the page is hidden
//! redraws nothing (unless a notice shows); saves are one one-shot after a
//! change; a busy clipboard is retried with a short one-shot.

use std::time::Instant;

use chrono::Local;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, KillTimer, SetForegroundWindow, SetTimer,
};

use super::App;
use crate::clipboard::persist::{self, Write};
use crate::clipboard::store::{self, ClipSnapshot, Ingested, ingest};
use crate::clipboard::win::{self, Read};
use crate::clipboard::{Clip, ClipData, History};
use crate::config::ClipHotkey;
use crate::i18n::{self, Key, tr};
use crate::model::{ClipAction, Event, Hit, Notice, NoticeKind, Page, View};
use crate::ui::cli_input::{CliInput, Key as InputKey};
use crate::ui::editor;
use crate::ui::geometry::{ShapeKind, shape_for};
use crate::ui::panel_layout::{clip_max_scroll, clip_rows, clip_scroll_to};
use crate::ui::render::ClipView;

/// One-shot: the clipboard was busy; read it again.
pub(super) const TIMER_CLIP_RETRY: usize = 17;
/// One-shot: write the history file once changes settle.
pub(super) const TIMER_CLIP_SAVE: usize = 18;
/// One-shot: the hotkey's panel was not visited; it closes.
pub(super) const TIMER_CLIP_HOLD: usize = 19;
const RETRY_MS: u32 = 60;
const RETRIES: u8 = 4;
const SAVE_MS: u32 = 1500;
const HOLD_MS: u32 = 8000;
/// `RegisterHotKey` id of the Clipboard page's hotkey.
pub(super) const HOTKEY_CLIP: i32 = 1;
/// Characters the search field takes.
pub(super) const SEARCH_LIMIT: usize = 256;
/// The "Disalin" preview's length before the renderer fits it.
const PREVIEW_CHARS: usize = 120;

/// The history and the page's state that is not the reducer's.
#[derive(Default)]
pub(super) struct ClipState {
    pub history: History,
    /// The sequence number after the island's own last write.
    pub own_sequence: Option<u32>,
    /// `AddClipboardFormatListener` is on.
    pub listening: bool,
    /// Reads of a busy clipboard tried so far for this change.
    pub retries: u8,
    /// `TIMER_CLIP_SAVE` is set: a write is due.
    pub save_due: bool,
    /// The chosen card (an entry id); `None`: the first one shown.
    pub selected: Option<u64>,
    /// Rows of cards scrolled.
    pub scroll: usize,
    /// The search text as last seen.
    pub query: String,
    /// The hotkey in effect, and whether registering the chosen one failed.
    pub hotkey: Option<ClipHotkey>,
    pub hotkey_failed: bool,
}

/// The entries the page lists for `query`, newest first.
fn shown<'a>(history: &'a History, query: &str) -> Vec<&'a Clip> {
    history.search(query, |k| tr(i18n::clip_kind_key(k)))
}

/// The selected card among `items`: the chosen one while it is listed,
/// else the first.
pub(super) fn effective(items: &[&Clip], chosen: Option<u64>) -> Option<u64> {
    chosen
        .filter(|id| items.iter().any(|c| c.id == *id))
        .or_else(|| items.first().map(|c| c.id))
}

/// What the Clipboard page draws, from the parts of `App` it needs (the
/// renderer is borrowed mutably meanwhile).
pub(super) fn clip_view<'a>(
    clip: &'a ClipState,
    enabled: bool,
    query: &'a str,
    selection: (usize, usize),
    focused: bool,
) -> ClipView<'a> {
    let items = shown(&clip.history, query);
    let selected = effective(&items, clip.selected);
    ClipView {
        enabled,
        total: clip.history.len(),
        items,
        query,
        selection,
        focused,
        selected,
        scroll: clip.scroll,
        now: Local::now().naive_local(),
    }
}

/// The "Disalin" notice for `clip`: its one-line preview.
fn copied_notice(clip: &Clip) -> Notice {
    let lang = i18n::lang();
    let body = match &clip.data {
        ClipData::Text(t) => store::preview(t, PREVIEW_CHARS),
        ClipData::Files(paths) => {
            let first = paths
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            i18n::clip_files(lang, paths.len(), &first)
        }
        ClipData::Image(img) => i18n::clip_image_meta(lang, img.screenshot, img.width, img.height),
    };
    Notice {
        key: "clip-copied".into(),
        source: tr(Key::ClipboardTitle).into(),
        title: tr(Key::ClipCopied).into(),
        body,
        kind: NoticeKind::Copied,
    }
}

impl App {
    /// At startup: the saved history (when kept), the listener and the hotkey.
    pub(super) fn clip_start(&mut self) {
        let cfg = self.config.clipboard;
        let items = if cfg.persist {
            persist::load_from(&persist::path())
        } else {
            Vec::new()
        };
        self.clip.history = History::from_items(cfg.max_items, items);
        self.sync_clip_listener();
        self.sync_clip_hotkey();
    }

    /// The listener runs while "Riwayat clipboard" is on.
    fn sync_clip_listener(&mut self) {
        let want = self.config.clipboard.enabled;
        if want && !self.clip.listening {
            self.clip.listening = win::add_listener(self.hwnd);
        } else if !want && self.clip.listening {
            win::remove_listener(self.hwnd);
            self.clip.listening = false;
        }
    }

    /// Registers the chosen hotkey (none by default). A failure (another app
    /// has it) shows in Settings; nothing else happens.
    fn sync_clip_hotkey(&mut self) {
        let want = self.config.clipboard.hotkey;
        if self.clip.hotkey == Some(want) {
            return;
        }
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_CLIP);
        }
        let mods = match want {
            ClipHotkey::Off => None,
            ClipHotkey::CtrlAltV => Some(MOD_CONTROL | MOD_ALT),
            ClipHotkey::WinAltV => Some(MOD_WIN | MOD_ALT),
        };
        self.clip.hotkey_failed = match mods {
            Some(m) => unsafe {
                RegisterHotKey(
                    Some(self.hwnd),
                    HOTKEY_CLIP,
                    m | MOD_NOREPEAT,
                    u32::from(b'V'),
                )
            }
            .is_err(),
            None => false,
        };
        self.clip.hotkey = Some(want);
    }

    /// `WM_DESTROY`: the listener and hotkey go; a pending save is written now.
    pub(super) fn clip_shutdown(&mut self) {
        if self.clip.listening {
            win::remove_listener(self.hwnd);
            self.clip.listening = false;
        }
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_CLIP);
        }
        if std::mem::take(&mut self.clip.save_due) && self.config.clipboard.persist {
            persist::write_now(
                &persist::path(),
                Write::Save(self.clip.history.items().to_vec()),
            );
        }
    }

    /// `WM_CLIPBOARDUPDATE`: the change goes into the history. The island's
    /// own copies add nothing; a skipped (password manager) change stores and
    /// says nothing. Only a visible page or a notice redraws.
    pub(super) fn on_clipboard_update(&mut self) {
        self.clip.retries = 0;
        self.read_clipboard();
    }

    fn read_clipboard(&mut self) {
        if !self.config.clipboard.enabled {
            return;
        }
        let snap: ClipSnapshot = match win::read(self.hwnd, self.clip.own_sequence) {
            Read::Own => return,
            Read::Busy => {
                if self.clip.retries < RETRIES {
                    self.clip.retries += 1;
                    unsafe {
                        SetTimer(Some(self.hwnd), TIMER_CLIP_RETRY, RETRY_MS, None);
                    }
                }
                return;
            }
            Read::Snapshot(snap) => snap,
        };
        let at = Local::now().timestamp();
        let outcome = ingest(&mut self.clip.history, snap, at);
        let id = match outcome {
            Ingested::Skipped | Ingested::Nothing => return,
            Ingested::Added { id, image } => {
                if image {
                    self.clip_image_arrived(id);
                }
                id
            }
            Ingested::Bumped { id } => id,
        };
        self.schedule_clip_save();
        let notice = self
            .config
            .clipboard
            .copied_feedback_all
            .then(|| self.clip.history.get(id).map(copied_notice))
            .flatten();
        match notice {
            Some(n) => self.apply_now(Event::Notice(n)),
            None => self.redraw_clip(),
        }
    }

    /// A new image entry arrived (a screenshot, a copied picture). Run 8's
    /// screenshot preview hooks in here; nothing else happens yet.
    fn clip_image_arrived(&mut self, _id: u64) {}

    /// Redraws the page if it is on screen.
    fn redraw_clip(&mut self) {
        if self.state.view() == View::Panel(Page::Clipboard) && !self.animating() {
            self.draw(Instant::now());
        }
    }

    /// The file follows a change once, `SAVE_MS` later (a burst of copies
    /// writes once). Nothing with "hanya di memori".
    fn schedule_clip_save(&mut self) {
        if !self.config.clipboard.persist {
            return;
        }
        self.clip.save_due = true;
        // Replaces a pending one: the last change sets the time.
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_CLIP_SAVE, SAVE_MS, None);
        }
    }

    pub(super) fn on_clip_timer(&mut self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
        match id {
            TIMER_CLIP_RETRY => self.read_clipboard(),
            TIMER_CLIP_SAVE => {
                if std::mem::take(&mut self.clip.save_due) && self.config.clipboard.persist {
                    let items = self.clip.history.items().to_vec();
                    persist::write_in_background(persist::path(), Write::Save(items));
                }
            }
            TIMER_CLIP_HOLD => {
                if self.state.clip_held() {
                    self.apply_now(Event::ClipHoldEnd);
                }
            }
            _ => {}
        }
    }

    /// `WM_HOTKEY`: the page opens (or closes) without taking the foreground.
    pub(super) fn on_clip_hotkey(&mut self) {
        self.apply_now(Event::ClipHotkey);
        if self.state.clip_held() {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_CLIP_HOLD, HOLD_MS, None);
            }
        }
    }

    /// The screen locked: with "Hapus saat layar dikunci", unpinned entries
    /// go from memory and the file. Pins stay.
    pub(super) fn clip_on_lock(&mut self) {
        if self.config.clipboard.clear_on_lock && self.clip.history.clear_unpinned() {
            self.clip.selected = None;
            self.clip.scroll = 0;
            self.write_clip_file_now();
        }
    }

    /// The file (or its removal) now, in the background: after a clear.
    fn write_clip_file_now(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CLIP_SAVE);
        }
        self.clip.save_due = false;
        if self.config.clipboard.persist {
            let items = self.clip.history.items().to_vec();
            persist::write_in_background(persist::path(), Write::Save(items));
        }
    }

    /// A card's click or button, from the reducer.
    pub(super) fn perform_clip(&mut self, action: ClipAction) -> Option<Event> {
        match action {
            ClipAction::Copy(id) => return self.copy_clip(id, false),
            ClipAction::CopyPlain(id) => return self.copy_clip(id, true),
            ClipAction::TogglePin(id) => {
                let pinned = self.clip.history.get(id).is_some_and(|c| c.pinned);
                self.clip.selected = Some(id);
                if self.clip.history.set_pinned(id, !pinned) {
                    self.schedule_clip_save();
                }
            }
            ClipAction::Delete(id) => {
                // The selection moves to the card that takes its place.
                let items = shown(&self.clip.history, &self.clip.query);
                let next = items
                    .iter()
                    .position(|c| c.id == id)
                    .and_then(|i| {
                        items
                            .get(i + 1)
                            .or_else(|| i.checked_sub(1).and_then(|j| items.get(j)))
                    })
                    .map(|c| c.id);
                if self.clip.history.delete(id) {
                    self.clip.selected = next;
                    self.clamp_clip_scroll();
                    self.schedule_clip_save();
                }
            }
        }
        self.redraw_clip();
        None
    }

    /// Puts entry `id` on the clipboard (as text only with `plain`); it moves
    /// to the front, no duplicate, and "Disalin" shows.
    fn copy_clip(&mut self, id: u64, plain: bool) -> Option<Event> {
        let clip = self.clip.history.get(id)?;
        let formats = win::formats_for(clip, plain);
        let sequence = win::write(self.hwnd, &formats)?;
        self.clip.own_sequence = Some(sequence);
        let notice = copied_notice(clip);
        self.clip.history.bump(id, Local::now().timestamp());
        self.clip.selected = Some(id);
        self.clip.scroll = 0;
        self.schedule_clip_save();
        Some(Event::Notice(notice))
    }

    fn clamp_clip_scroll(&mut self) {
        let n = shown(&self.clip.history, &self.clip.query).len();
        let max = clip_max_scroll(n, self.clip_rows());
        self.clip.scroll = self.clip.scroll.min(max);
    }

    fn clip_rows(&self) -> usize {
        let s = shape_for(self.config.style, ShapeKind::Panel);
        clip_rows(s, self.state.waiting_strip().is_some())
    }

    /// The wheel over the cards: a row per notch.
    pub(super) fn clip_wheel(&mut self, delta: i32) {
        let rows = (-delta / 120).clamp(-20, 20);
        let rows = if rows == 0 { -delta.signum() } else { rows };
        let n = shown(&self.clip.history, &self.clip.query).len();
        let max = clip_max_scroll(n, self.clip_rows()) as i64;
        let to = (self.clip.scroll as i64 + i64::from(rows)).clamp(0, max);
        if to as usize != self.clip.scroll {
            self.clip.scroll = to as usize;
            self.redraw_clip();
        }
    }

    /// A right-click on a card selects it (its buttons show); true when it
    /// was one, so the tray menu stays shut.
    pub(super) fn clip_right_click(&mut self, hit: Option<&Hit>) -> bool {
        if self.state.view() != View::Panel(Page::Clipboard) {
            return false;
        }
        match hit {
            Some(Hit::ClipCard(id)) => {
                self.clip.selected = Some(*id);
                self.redraw_clip();
                true
            }
            _ => false,
        }
    }

    /// After the reducer: a closed panel forgets the search, the selection
    /// and the scroll.
    pub(super) fn follow_clip_page(&mut self, was_open: bool) {
        if was_open && !self.state.panel_is_open() {
            self.clip.selected = None;
            self.clip.scroll = 0;
            self.clip.query.clear();
            if let Some(input) = &self.clip_input
                && !input.text().is_empty()
            {
                input.set_text("");
            }
        }
    }

    /// The search field has the keyboard for real.
    pub(super) fn clip_has_keyboard(&self) -> bool {
        self.clip_input.as_ref().is_some_and(CliInput::has_focus)
            && unsafe { GetForegroundWindow() } == self.hwnd
    }

    /// A click on the search field: the island activates (this process got
    /// the click) and the hidden EDIT takes the keyboard, as for the CLI.
    pub(super) fn focus_clip_search(&mut self) -> Option<Event> {
        if editor::focus_open_editor() {
            return None;
        }
        let input = self.clip_input.as_ref()?;
        unsafe {
            let fg = GetForegroundWindow();
            if fg != self.hwnd {
                self.cli_prev = (!fg.is_invalid()).then_some(fg);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        let ok = input.focus() && unsafe { GetForegroundWindow() } == self.hwnd;
        if !ok {
            self.return_keyboard();
            return None;
        }
        (!self.state.clip_focused()).then_some(Event::ClipFocus(true))
    }

    /// Esc on an empty field: the panel unlocks and the keyboard goes back.
    fn release_clip(&mut self) {
        self.return_keyboard();
        if self.state.clip_focused() {
            self.apply_now(Event::ClipFocus(false));
        }
    }

    /// A key the search field hands over.
    pub(super) fn on_clip_key(&mut self, key: InputKey) {
        match key {
            InputKey::Enter => {
                let items = shown(&self.clip.history, &self.clip.query);
                let Some(id) = effective(&items, self.clip.selected) else {
                    return;
                };
                let notice = self.copy_clip(id, false);
                // The panel closes and the keyboard goes back (the reducer
                // lets go), then "Disalin".
                self.apply_now(Event::ClipCommit);
                if let Some(event) = notice {
                    self.apply_now(event);
                }
            }
            InputKey::Up | InputKey::Down => {
                let items = shown(&self.clip.history, &self.clip.query);
                let Some(id) = effective(&items, self.clip.selected) else {
                    return;
                };
                let at = items.iter().position(|c| c.id == id).unwrap_or(0);
                let to = if key == InputKey::Up {
                    at.saturating_sub(1)
                } else {
                    (at + 1).min(items.len() - 1)
                };
                let next = items[to].id;
                let rows = self.clip_rows();
                self.clip.selected = Some(next);
                self.clip.scroll = clip_scroll_to(to, self.clip.scroll, rows);
                self.redraw_clip();
            }
            InputKey::PageUp | InputKey::PageDown => {
                let rows = self.clip_rows() as i32 * 120;
                self.clip_wheel(if key == InputKey::PageUp { rows } else { -rows });
            }
            // Text first; the next Esc lets go.
            InputKey::Escape => match &self.clip_input {
                Some(input) if !input.text().is_empty() => input.set_text(""),
                _ => self.release_clip(),
            },
            InputKey::Blur => {
                if self.state.clip_focused() && !self.clip_has_keyboard() {
                    self.cli_prev = None;
                    self.apply_now(Event::ClipFocus(false));
                }
            }
            InputKey::Tab | InputKey::Stop => {}
        }
    }

    /// The field's text or selection changed: the list filters again from
    /// its first card.
    pub(super) fn on_clip_changed(&mut self) {
        let typed = self
            .clip_input
            .as_ref()
            .map(CliInput::text)
            .unwrap_or_default();
        if typed != self.clip.query {
            self.clip.query = typed;
            self.clip.selected = None;
            self.clip.scroll = 0;
        }
        self.redraw_clip();
    }

    /// Settings → Clipboard.
    pub(super) fn on_clip_setting(&mut self, change: ClipSetting) {
        let cfg = &mut self.config.clipboard;
        match change {
            ClipSetting::Enabled(on) => cfg.enabled = on,
            ClipSetting::Items(n) => cfg.max_items = store::clamp_items(n),
            ClipSetting::Persist(on) => cfg.persist = on,
            ClipSetting::ClearOnLock(on) => cfg.clear_on_lock = on,
            ClipSetting::Hotkey(h) => cfg.hotkey = h,
            ClipSetting::FeedbackAll(on) => cfg.copied_feedback_all = on,
            ClipSetting::Clear => {
                if self.clip.history.clear_unpinned() {
                    self.clip.selected = None;
                    self.clip.scroll = 0;
                    self.write_clip_file_now();
                }
                self.redraw_clip();
                return;
            }
        }
        self.save_config();
        match change {
            ClipSetting::Enabled(_) => {
                self.sync_clip_listener();
                self.redraw_clip();
            }
            ClipSetting::Items(_) => {
                self.clip.history.set_cap(self.config.clipboard.max_items);
                self.clamp_clip_scroll();
                self.schedule_clip_save();
                self.redraw_clip();
            }
            // Off: the file goes. On: it is written now.
            ClipSetting::Persist(false) => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_CLIP_SAVE);
                }
                self.clip.save_due = false;
                persist::write_in_background(persist::path(), Write::Remove);
            }
            ClipSetting::Persist(true) => self.write_clip_file_now(),
            ClipSetting::Hotkey(_) => self.sync_clip_hotkey(),
            _ => {}
        }
    }

    /// For Settings: the hotkey chosen could not be registered.
    pub(super) fn clip_hotkey_failed(&self) -> bool {
        self.clip.hotkey_failed
    }
}

/// A change in Settings → Clipboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipSetting {
    Enabled(bool),
    Items(u32),
    Persist(bool),
    ClearOnLock(bool),
    Hotkey(ClipHotkey),
    FeedbackAll(bool),
    /// "Hapus riwayat": everything not pinned.
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::store::tests::text;

    #[test]
    fn the_selection_falls_back_to_the_first_card_shown() {
        let mut h = History::new(50);
        for s in ["a", "b", "c"] {
            ingest(&mut h, text(s), 0);
        }
        let items = shown(&h, "");
        let ids: Vec<u64> = items.iter().map(|c| c.id).collect();
        assert_eq!(effective(&items, None), Some(ids[0]));
        assert_eq!(effective(&items, Some(ids[2])), Some(ids[2]));
        // Filtered away: the first match.
        let only_a = shown(&h, "a");
        assert_eq!(effective(&only_a, Some(ids[0])), Some(ids[2]));
        assert_eq!(effective(&[], Some(1)), None);
    }

    #[test]
    fn copied_previews_one_line() {
        let mut h = History::new(50);
        ingest(&mut h, text("Denah lantai 2\n  sudah saya revisi"), 0);
        let n = copied_notice(&h.items()[0]);
        assert_eq!(n.kind, NoticeKind::Copied);
        assert_eq!(n.title, "Disalin");
        assert_eq!(n.body, "Denah lantai 2 sudah saya revisi");
    }
}
