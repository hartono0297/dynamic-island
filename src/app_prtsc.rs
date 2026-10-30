//! The PrtSc key opens Vale's own screenshot (`capture` and the editor after
//! it): the hotkey, its registration, what Settings shows when Windows or
//! another app already has the key, and the one button that asks Windows to
//! let go of it. A child of `app`, so it works on `App` directly.
//!
//! The key is registered bare (`MOD_NOREPEAT`, no other modifier): Alt+PrtSc,
//! Win+PrtSc, Ctrl+PrtSc and Shift+PrtSc stay Windows'. Windows 11 gives the
//! bare key to Snipping Tool (`PrintScreenKeyForSnippingEnabled`), in which
//! case `RegisterHotKey` fails; the Settings row then offers to turn that off.
//!
//! Power: no timer, no polling. A failed registration is tried again only
//! when the tray menu opens or Settings opens (`retry_hotkeys`). A config
//! save or read registers or lets go only when the setting itself moved
//! (`follows_config`), so a toggle click is one try and an unrelated save
//! none. The registry is read once per failed try and kept, never on a
//! Settings redraw.

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_SNAPSHOT,
};
use windows::core::w;

use super::App;
use crate::model::Event;
use crate::shortcuts;
use crate::ui::settings_layout::PrtscRow;

/// `RegisterHotKey` id of the PrtSc key (Clipboard 1, Search 2, Devices 3).
pub(super) const HOTKEY_PRTSC: i32 = 4;

/// Where Windows keeps "Use the Print screen button to open screen
/// capture": under HKCU, a DWORD. 0 = off; 1 or absent = on (Windows 11
/// treats an absent value as on).
const KEYBOARD_KEY: windows::core::PCWSTR = w!("Control Panel\\Keyboard");
const SNIPPING_VALUE: windows::core::PCWSTR = w!("PrintScreenKeyForSnippingEnabled");
/// Where "Matikan di Windows" ends up when the retry after the registry write
/// still fails: Windows' own keyboard page, for the owner to switch it off.
const WINDOWS_KEYBOARD_PAGE: &str = "ms-settings:easeofaccess-keyboard";

/// What the PrtSc key holds: whether Vale has the key, whether the last
/// registration failed, and Windows' value as read then.
#[derive(Default)]
pub(super) struct PrtscState {
    pub registered: bool,
    pub failed: bool,
    /// `PrintScreenKeyForSnippingEnabled` read at the last failed try
    /// (`None`: absent). Not read while the key is Vale's.
    windows_value: Option<u32>,
}

impl PrtscState {
    /// Keeps the outcome of a `RegisterHotKey` try. Only a failure reads
    /// Windows' value (`read`), once, for the Settings row.
    fn tried(&mut self, ok: bool, read: impl FnOnce() -> Option<u32>) {
        self.registered = ok;
        self.failed = !ok;
        if !ok {
            self.windows_value = read();
        }
    }

    /// The Settings row, from what is kept (no registry read).
    fn row(&self, on: bool) -> RowState {
        prtsc_row(on, self.failed, self.windows_value)
    }

    /// The button writes only while its row offers it: a late click (the
    /// key freed meanwhile from the tray or Settings) leaves Windows alone.
    fn click_writes(&self, on: bool) -> bool {
        self.row(on) == RowState::TakenByWindows
    }
}

/// What the Settings row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowState {
    /// The toggle is off: nothing registered, nothing to say.
    Off,
    /// On and Vale has the key.
    Active,
    /// On, registration failed, and Windows' own setting still gives the key
    /// to Snipping Tool (value 1 or absent): "Dipakai Windows" and a button.
    TakenByWindows,
    /// On, registration failed, and Windows' setting is already off: another
    /// app holds the key. The usual "taken" text, no button.
    TakenByApp,
}

/// The row's state from the toggle, whether registration failed and the
/// registry value (`None`: absent).
pub(super) fn prtsc_row(on: bool, failed: bool, windows_value: Option<u32>) -> RowState {
    if !on {
        RowState::Off
    } else if !failed {
        RowState::Active
    } else if windows_value != Some(0) {
        RowState::TakenByWindows
    } else {
        RowState::TakenByApp
    }
}

/// What Settings draws for a row state.
pub(super) fn ui_row(row: RowState) -> PrtscRow {
    match row {
        RowState::Off | RowState::Active => PrtscRow::Plain,
        RowState::TakenByWindows => PrtscRow::TakenByWindows,
        RowState::TakenByApp => PrtscRow::TakenByApp,
    }
}

/// What the button does after its registry write and the retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// The key is Vale's now.
    Registered,
    /// Still taken, whether or not the write went through (Windows may not
    /// have picked it up yet): open Windows' keyboard settings for the owner.
    OpenSettings,
}

pub(super) fn after_disable_click(retry_ok: bool) -> Next {
    if retry_ok {
        Next::Registered
    } else {
        Next::OpenSettings
    }
}

/// Whether a config save or read has to touch the registration: only when
/// the setting no longer matches what is held or was last tried (turned on
/// and never tried, turned off while held or failed). A failed key that is
/// still wanted waits for `retry_hotkeys`.
pub(super) fn follows_config(want: bool, registered: bool, failed: bool) -> bool {
    want != (registered || failed)
}

/// What a sync has to do to the registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    Nothing,
    Register,
    Unregister,
}

/// The step from the setting, whether Vale holds the key and whether the
/// last try failed. A failed key is tried again (`Register`) only while it
/// is wanted; one that is held is left alone.
pub(super) fn plan(want: bool, registered: bool) -> Plan {
    match (want, registered) {
        (true, false) => Plan::Register,
        (false, true) => Plan::Unregister,
        _ => Plan::Nothing,
    }
}

/// `PrintScreenKeyForSnippingEnabled` as Windows has it. Read only.
pub(super) fn read_windows_value() -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEYBOARD_KEY,
            SNIPPING_VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}

/// Writes `PrintScreenKeyForSnippingEnabled` = 0 (REG_DWORD, HKCU only). The
/// only place Vale writes it, and only from the button's click.
fn write_windows_value_off() -> bool {
    let zero: u32 = 0;
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            KEYBOARD_KEY,
            SNIPPING_VALUE,
            REG_DWORD.0,
            Some(&zero as *const u32 as *const _),
            size_of::<u32>() as u32,
        )
    }
    .is_ok()
}

impl App {
    /// Registers or lets go of the bare PrtSc key as the setting says. A
    /// failure (Windows or another app has it) shows in Settings and is tried
    /// again on the next sync, tray menu or Settings opening.
    pub(super) fn sync_prtsc_hotkey(&mut self) {
        let want = self.config.screenshot.print_screen;
        match plan(want, self.prtsc.registered) {
            Plan::Nothing => {}
            Plan::Unregister => unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PRTSC);
                self.prtsc.registered = false;
            },
            Plan::Register => {
                let ok = unsafe {
                    RegisterHotKey(
                        Some(self.hwnd),
                        HOTKEY_PRTSC,
                        MOD_NOREPEAT,
                        u32::from(VK_SNAPSHOT.0),
                    )
                }
                .is_ok();
                self.prtsc.tried(ok, read_windows_value);
            }
        }
        if !want {
            self.prtsc.failed = false;
        }
    }

    /// `WM_HOTKEY`: Vale's own capture, whatever "Alat screenshot" says (that
    /// choice is for the tile). Starts through the shortcut timer like the
    /// tile, after an open panel or the search has collapsed, so it is not in
    /// the picture. One overlay at a time: a press while it is up (or while
    /// it is starting) does nothing.
    pub(super) fn on_prtsc_hotkey(&mut self) {
        if !self.config.screenshot.print_screen || crate::capture::overlay::active() {
            return;
        }
        self.apply_now(Event::PrintScreen);
    }

    /// The Settings toggle: kept and saved; the key follows from
    /// `adopt_config` (`prtsc_follow_config`), one try per click.
    pub(super) fn set_print_screen(&mut self, on: bool) {
        self.config.screenshot.print_screen = on;
        self.save_config();
    }

    /// A config save or read (the toggle, a hand edit): the key follows a
    /// changed setting. Not a retry of a failed key (`retry_hotkeys` is).
    pub(super) fn prtsc_follow_config(&mut self) {
        let want = self.config.screenshot.print_screen;
        if follows_config(want, self.prtsc.registered, self.prtsc.failed) {
            self.sync_prtsc_hotkey();
        }
    }

    /// The row's state for Settings, from the value kept at the last failed
    /// try (no registry read: this runs on every refresh while Settings is
    /// open).
    pub(super) fn prtsc_model(&self) -> PrtscRow {
        ui_row(self.prtsc.row(self.config.screenshot.print_screen))
    }

    /// "Matikan di Windows", on click only and only while the row offers it:
    /// Windows' own setting is switched off (HKCU), then the registration is
    /// tried again (which reads the value back if it fails). Still taken,
    /// written or not, Windows' keyboard settings open for the owner.
    pub(super) fn turn_off_windows_print_screen(&mut self) {
        if !self.prtsc.click_writes(self.config.screenshot.print_screen) {
            return;
        }
        let _ = write_windows_value_off();
        self.sync_prtsc_hotkey();
        if after_disable_click(self.prtsc.registered) == Next::OpenSettings {
            self.queue_shortcut(shortcuts::Action::Open(WINDOWS_KEYBOARD_PAGE));
        }
    }

    /// `WM_DESTROY`: the key goes back.
    pub(super) fn prtsc_shutdown(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PRTSC);
        }
        self.prtsc.registered = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_the_next_free_one() {
        assert_eq!(HOTKEY_PRTSC, 4);
        assert_eq!(super::super::clip::HOTKEY_CLIP, 1);
        assert_eq!(super::super::HOTKEY_SEARCH, 2);
        assert_eq!(super::super::HOTKEY_DEVICES, 3);
    }

    #[test]
    fn the_row_follows_the_toggle_the_registration_and_windows() {
        for value in [None, Some(0), Some(1), Some(7)] {
            assert_eq!(prtsc_row(false, false, value), RowState::Off);
            // Off wins even when a stale failure is left.
            assert_eq!(prtsc_row(false, true, value), RowState::Off);
            assert_eq!(prtsc_row(true, false, value), RowState::Active);
        }
        // Taken: Windows' setting on (1) or absent is Windows'.
        assert_eq!(prtsc_row(true, true, None), RowState::TakenByWindows);
        assert_eq!(prtsc_row(true, true, Some(1)), RowState::TakenByWindows);
        // An explicit 0 means another app has the key.
        assert_eq!(prtsc_row(true, true, Some(0)), RowState::TakenByApp);
        assert_eq!(ui_row(RowState::Off), PrtscRow::Plain);
        assert_eq!(ui_row(RowState::Active), PrtscRow::Plain);
        assert_eq!(ui_row(RowState::TakenByWindows), PrtscRow::TakenByWindows);
        assert_eq!(ui_row(RowState::TakenByApp), PrtscRow::TakenByApp);
    }

    #[test]
    fn a_failed_retry_after_the_click_opens_windows_keyboard_settings() {
        assert_eq!(after_disable_click(true), Next::Registered);
        // The usual Windows 11 case (the write landed but Windows has not let
        // go of the key yet) and a failed write alike: Windows' own page.
        assert_eq!(after_disable_click(false), Next::OpenSettings);
        assert_eq!(WINDOWS_KEYBOARD_PAGE, "ms-settings:easeofaccess-keyboard");
    }

    #[test]
    fn a_config_save_touches_the_key_only_when_the_setting_moved() {
        // Wanted and held, or wanted and failed: an unrelated save (or the
        // toggle's own save, already followed) neither registers again nor
        // reads the registry. A failed key waits for `retry_hotkeys`.
        assert!(!follows_config(true, true, false));
        assert!(!follows_config(true, false, true));
        // Turned on (never tried, or let go): one try.
        assert!(follows_config(true, false, false));
        // Turned off while held: let go; while failed: the failure clears.
        assert!(follows_config(false, true, false));
        assert!(follows_config(false, false, true));
        // Off and nothing held: nothing.
        assert!(!follows_config(false, false, false));
        // After the one try the toggle's save made, the same setting no
        // longer moves anything, held or failed.
        let mut s = PrtscState::default();
        s.tried(false, || Some(1));
        assert!(!follows_config(true, s.registered, s.failed));
        s.tried(true, || None);
        assert!(!follows_config(true, s.registered, s.failed));
    }

    #[test]
    fn the_click_writes_only_while_the_row_offers_the_button() {
        let mut s = PrtscState::default();
        // Never failed: nothing to turn off.
        assert!(!s.click_writes(true));
        s.tried(false, || Some(1));
        assert!(s.click_writes(true));
        // The toggle switched off meanwhile.
        assert!(!s.click_writes(false));
        // A retry from the tray or Settings got the key first: a late click
        // on the button still on screen writes nothing.
        s.tried(true, || panic!("a success reads nothing"));
        assert!(!s.click_writes(true));
        // Another app (an explicit 0 Vale did not write): no button either.
        s.tried(false, || Some(0));
        assert!(!s.click_writes(true));
    }

    #[test]
    fn the_registry_is_read_once_per_failed_try_never_for_the_row() {
        let reads = std::cell::Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            None
        };
        let mut s = PrtscState::default();
        s.tried(true, read);
        assert_eq!(reads.get(), 0, "a success reads nothing");
        s.tried(false, read);
        assert_eq!(reads.get(), 1);
        // Settings redraws use what was kept.
        for _ in 0..10 {
            assert_eq!(s.row(true), RowState::TakenByWindows);
        }
        assert_eq!(reads.get(), 1);
        // The next failed try (a retry, or the one after the click)
        // refreshes it.
        s.tried(false, || Some(0));
        assert_eq!(s.row(true), RowState::TakenByApp);
    }

    #[test]
    fn registration_is_planned_from_the_setting_and_what_is_held() {
        // Off: nothing held, nothing to do; held: let go.
        assert_eq!(plan(false, false), Plan::Nothing);
        assert_eq!(plan(false, true), Plan::Unregister);
        // On: not held (never tried, failed, or let go): try; held: leave.
        assert_eq!(plan(true, false), Plan::Register);
        assert_eq!(plan(true, true), Plan::Nothing);
    }
}
