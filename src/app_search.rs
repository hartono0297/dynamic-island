//! The island's side of the quick search: its hotkey, the Start Menu index
//! on a background thread, the field's keys, the results and running one.
//! A child of `app`, so it works on `App` directly. What a line finds and
//! how results rank is `search.rs`; the reducer only knows the search is
//! open and which row was clicked.
//!
//! Power: nothing runs while the search is closed. The Start Menu is read
//! once, on the first time it opens, on its own thread.

use std::time::Instant;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

use super::{App, WM_APP_EVENT, poster};
use crate::clipboard::win;
use crate::config::SearchHotkey;
use crate::i18n::{self, Key, tr};
use crate::model::{Event, Hit, Notice, NoticeKind, Page, View};
use crate::search::{self, Action, AppEntry, Labels, SearchResult};
use crate::shortcuts;
use crate::ui::cli_input::{CliInput, Key as InputKey};
use crate::ui::editor;

/// `RegisterHotKey` id of the quick search (the Clipboard's is 1).
pub(super) const HOTKEY_SEARCH: i32 = 2;
/// Characters the field takes.
pub(super) const QUERY_LIMIT: usize = search::QUERY_MAX;

/// The search's state that is not the reducer's.
#[derive(Default)]
pub(super) struct SearchState {
    /// The Start Menu's apps; `None` until the index thread answered.
    pub apps: Option<Vec<AppEntry>>,
    /// The index thread was started.
    pub indexing: bool,
    /// The field's text as last searched.
    pub query: String,
    pub results: Vec<SearchResult>,
    /// The highlighted row.
    pub selected: usize,
    /// The hotkey in effect, and whether registering the chosen one failed.
    pub hotkey: Option<SearchHotkey>,
    pub hotkey_failed: bool,
}

/// `CF_UNICODETEXT`'s bytes for `text`.
fn text_formats(text: &str) -> Vec<(u32, Vec<u8>)> {
    vec![(13, win::utf16z(text))]
}

/// The word under a result row's kind, in the current language.
fn with_labels<T>(f: impl FnOnce(&Labels) -> T) -> T {
    let lang = i18n::lang();
    let web_title = move |q: &str| i18n::search_web_title(lang, q);
    f(&Labels {
        app: tr(Key::SearchKindApp),
        setting: tr(Key::SearchKindSetting),
        copy: tr(Key::SearchCopy),
        web: tr(Key::SearchKindWeb),
        web_title: &web_title,
    })
}

/// The highlighted row after an Up or a Down among `n` rows (it stops at
/// both ends).
pub(super) fn move_selection(at: usize, n: usize, up: bool) -> usize {
    if n == 0 {
        0
    } else if up {
        at.saturating_sub(1)
    } else {
        (at + 1).min(n - 1)
    }
}

impl App {
    /// Registers the chosen hotkey (none by default). A failure (another app
    /// has it) shows in Settings; nothing else happens.
    pub(super) fn sync_search_hotkey(&mut self) {
        let want = self.config.search_hotkey;
        if self.search.hotkey == Some(want) {
            return;
        }
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SEARCH);
        }
        let reg = match want {
            SearchHotkey::Off => None,
            SearchHotkey::CtrlAltSpace => Some((MOD_CONTROL | MOD_ALT, u32::from(VK_SPACE.0))),
            SearchHotkey::WinAltS => Some((MOD_WIN | MOD_ALT, u32::from(b'S'))),
        };
        self.search.hotkey_failed = match reg {
            Some((mods, key)) => unsafe {
                RegisterHotKey(Some(self.hwnd), HOTKEY_SEARCH, mods | MOD_NOREPEAT, key)
            }
            .is_err(),
            None => false,
        };
        self.search.hotkey = Some(want);
    }

    /// `WM_DESTROY`: the hotkey goes.
    pub(super) fn search_shutdown(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SEARCH);
        }
    }

    /// `WM_HOTKEY`: the search opens (or closes). The Start Menu is read the
    /// first time.
    pub(super) fn on_search_hotkey(&mut self) {
        self.start_search_index();
        self.apply_now(Event::SearchHotkey);
    }

    /// Reads the Start Menu on its own thread, once.
    fn start_search_index(&mut self) {
        if self.search.apps.is_some() || self.search.indexing {
            return;
        }
        self.search.indexing = true;
        let tx = self.tx.clone();
        let wake = poster(self.hwnd, WM_APP_EVENT);
        let spawned = std::thread::Builder::new()
            .name("vale-search-index".into())
            .spawn(move || {
                let apps = search::index_start_menu();
                if tx.send(Event::SearchIndex(apps)).is_ok() {
                    wake();
                }
            });
        if spawned.is_err() {
            self.search.indexing = false;
        }
    }

    /// The index arrived: what is typed is searched again.
    pub(super) fn on_search_index(&mut self, apps: Vec<AppEntry>) {
        self.search.apps = Some(apps);
        self.search.indexing = false;
        self.search_again();
        self.redraw_search();
    }

    /// The reducer opened the search: a clean field takes the keyboard (the
    /// island activates). When it cannot have it, the search closes again.
    pub(super) fn focus_search(&mut self) -> Option<Event> {
        // Every way in (hotkey, tray, tile) reads the Start Menu once.
        self.start_search_index();
        if editor::focus_open_editor() {
            return Some(Event::SearchClose);
        }
        let input = self.search_input.as_ref()?;
        input.set_text("");
        self.search.query.clear();
        self.search.results.clear();
        self.search.selected = 0;
        unsafe {
            let fg = GetForegroundWindow();
            if fg != self.hwnd {
                self.cli_prev = (!fg.is_invalid()).then_some(fg);
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        let ok = input.focus() && unsafe { GetForegroundWindow() } == self.hwnd;
        if ok {
            None
        } else {
            self.return_keyboard();
            Some(Event::SearchClose)
        }
    }

    /// The search closed: the keyboard goes back and the field is cleared.
    pub(super) fn release_search(&mut self) {
        self.return_keyboard();
        if let Some(input) = &self.search_input
            && !input.text().is_empty()
        {
            input.set_text("");
        }
        self.search.query.clear();
        self.search.results.clear();
        self.search.selected = 0;
    }

    /// The field has the keyboard for real.
    fn search_has_keyboard(&self) -> bool {
        self.search_input.as_ref().is_some_and(CliInput::has_focus)
            && unsafe { GetForegroundWindow() } == self.hwnd
    }

    /// A key the field hands over.
    pub(super) fn on_search_key(&mut self, key: InputKey) {
        match key {
            // Enter is a click on the highlighted row: the reducer closes the
            // search and asks for it to be run.
            InputKey::Enter => {
                let at = self.search.selected;
                if self.search.results.get(at).is_some() {
                    self.apply_now(Event::Click(Hit::SearchResult(at)));
                }
            }
            InputKey::Up | InputKey::Down => {
                let to = move_selection(
                    self.search.selected,
                    self.search.results.len(),
                    key == InputKey::Up,
                );
                if to != self.search.selected {
                    self.search.selected = to;
                    self.redraw_search();
                }
            }
            InputKey::Escape => self.apply_now(Event::SearchClose),
            // Another window took the keyboard (a click outside, Alt+Tab):
            // no foreground to hand back.
            InputKey::Blur => {
                if self.state.search_open() && !self.search_has_keyboard() {
                    self.cli_prev = None;
                    self.apply_now(Event::SearchClose);
                }
            }
            InputKey::Tab | InputKey::Stop | InputKey::PageUp | InputKey::PageDown => {}
        }
    }

    /// The field's text or selection changed.
    pub(super) fn on_search_changed(&mut self) {
        let typed = self
            .search_input
            .as_ref()
            .map(CliInput::text)
            .unwrap_or_default();
        if typed != self.search.query {
            self.search.query = typed;
            self.search_again();
        }
        self.redraw_search();
    }

    /// Searches again for what is typed; the highlight goes to the first row.
    fn search_again(&mut self) {
        let lang = i18n::lang();
        let apps = self.search.apps.as_deref().unwrap_or(&[]);
        self.search.results = with_labels(|labels| search::search(&self.search.query, apps, lang, labels));
        self.search.selected = 0;
    }

    fn redraw_search(&mut self) {
        if self.state.view() == View::Panel(Page::Search) && !self.animating() {
            self.draw(Instant::now());
        }
    }

    /// Runs result `i` (the search is closing already). `Some(event)` is
    /// what to show after (the "Disalin" notice of a copied result).
    pub(super) fn run_search(&mut self, i: usize) -> Option<Event> {
        let result = self.search.results.get(i)?.clone();
        match result.action {
            Action::Launch(path) => {
                self.queue_shortcut(shortcuts::Action::Shell(path));
                None
            }
            Action::OpenSetting(uri) => {
                self.queue_shortcut(shortcuts::Action::Shell(uri.to_owned()));
                None
            }
            Action::OpenWeb(url) => {
                if search::is_web_url(&url) {
                    self.queue_shortcut(shortcuts::Action::Shell(url));
                }
                None
            }
            Action::Copy(text) => {
                let sequence = win::write(self.hwnd, &text_formats(&text))?;
                // Written here: not a copy for the history to take again.
                self.clip.own_sequence = Some(sequence);
                Some(Event::Notice(Notice {
                    key: "search-copied".into(),
                    source: tr(Key::SearchTitle).into(),
                    title: tr(Key::ClipCopied).into(),
                    body: text,
                    kind: NoticeKind::Copied,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highlight_stops_at_both_ends() {
        assert_eq!(move_selection(0, 4, true), 0);
        assert_eq!(move_selection(0, 4, false), 1);
        assert_eq!(move_selection(3, 4, false), 3);
        assert_eq!(move_selection(3, 4, true), 2);
        assert_eq!(move_selection(0, 0, false), 0);
        assert_eq!(move_selection(5, 0, true), 0);
    }

    #[test]
    fn text_goes_to_the_clipboard_as_unicode() {
        let formats = text_formats("12,5");
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].0, 13);
        // UTF-16LE, NUL-terminated.
        assert_eq!(formats[0].1, [b'1', 0, b'2', 0, b',', 0, b'5', 0, 0, 0]);
    }

    #[test]
    fn labels_name_the_kinds_in_both_languages() {
        for lang in [i18n::Lang::Id, i18n::Lang::En] {
            i18n::set_lang(lang);
            let apps = [AppEntry {
                name: "Kalkulator".into(),
                path: "x.lnk".into(),
            }];
            let found = with_labels(|l| search::search("kal", &apps, lang, l));
            assert_eq!(found[0].subtitle, tr(Key::SearchKindApp));
            assert_eq!(found.last().unwrap().subtitle, tr(Key::SearchKindWeb));
            assert!(found.last().unwrap().title.contains("kal"));
        }
        i18n::set_lang(i18n::Lang::Id);
    }
}
