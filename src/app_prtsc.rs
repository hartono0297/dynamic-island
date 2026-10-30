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
//! when the config is read, the tray menu opens or Settings opens
//! (`retry_hotkeys`), like the other hotkeys. The registry is read once per
//! failed try and kept, never on a Settings redraw.
//!
//! Windows usually picks up a direct edit of its value only after a sign-out
//! (Vale sends no `WM_SETTINGCHANGE`; nothing documents one that frees the
//! key). So once Vale has written 0 and the key is still taken, the row says
//! to sign out rather than blaming another app.

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
/// Where "Matikan di Windows" ends up when the registry write did not free
/// the key: Windows' own keyboard page, for the owner to switch it off.
const WINDOWS_KEYBOARD_PAGE: &str = "ms-settings:easeofaccess-keyboard";

/// What the PrtSc key holds: whether Vale has the key, whether the last
/// registration failed, Windows' value as read then, and whether Vale wrote
/// that value this session.
#[derive(Default)]
pub(super) struct PrtscState {
    pub registered: bool,
    pub failed: bool,
    /// `PrintScreenKeyForSnippingEnabled` read at the last failed try
    /// (`None`: absent). Not read while the key is Vale's.
    windows_value: Option<u32>,
    /// The button's write went through this session: a 0 read back is then
    /// Vale's own, not yet in effect, rather than the sign of another app.
    wrote_off: bool,
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
        prtsc_row(on, self.failed, self.windows_value, self.wrote_off)
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
    /// On, still failing after Vale itself wrote 0: Windows has not picked
    /// the change up yet. "Applies after signing out", no button.
    SignOutToApply,
}

/// The row's state from the toggle, whether registration failed, the
/// registry value (`None`: absent) and whether Vale wrote that value.
pub(super) fn prtsc_row(
    on: bool,
    failed: bool,
    windows_value: Option<u32>,
    wrote_off: bool,
) -> RowState {
    if !on {
        RowState::Off
    } else if !failed {
        RowState::Active
    } else if windows_value != Some(0) {
        RowState::TakenByWindows
    } else if wrote_off {
        RowState::SignOutToApply
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
        RowState::SignOutToApply => PrtscRow::SignOutToApply,
    }
}

/// What the button does after its registry write and the retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// The key is Vale's now.
    Registered,
    /// Written but still taken: Windows applies it after a sign-out. Its
    /// keyboard page would already show the setting off, so nothing opens;
    /// the row says to sign out.
    SignOut,
    /// The write failed: open Windows' keyboard settings for the owner.
    OpenSettings,
}

pub(super) fn after_disable_click(retry_ok: bool, wrote: bool) -> Next {
    if retry_ok {
        Next::Registered
    } else if wrote {
        Next::SignOut
    } else {
        Next::OpenSettings
    }
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

    /// The Settings toggle: kept, saved, and the key follows.
    pub(super) fn set_print_screen(&mut self, on: bool) {
        self.config.screenshot.print_screen = on;
        self.save_config();
        self.sync_prtsc_hotkey();
    }

    /// The row's state for Settings, from the value kept at the last failed
    /// try (no registry read: this runs on every refresh while Settings is
    /// open).
    pub(super) fn prtsc_model(&self) -> PrtscRow {
        ui_row(self.prtsc.row(self.config.screenshot.print_screen))
    }

    /// "Matikan di Windows", on click only and only while the row offers it:
    /// Windows' own setting is switched off (HKCU), then the registration is
    /// tried again (which reads the value back if it fails). Written but
    /// still taken, the row says to sign out; a failed write opens Windows'
    /// keyboard settings for the owner.
    pub(super) fn turn_off_windows_print_screen(&mut self) {
        if !self.prtsc.click_writes(self.config.screenshot.print_screen) {
            return;
        }
        let wrote = write_windows_value_off();
        self.prtsc.wrote_off |= wrote;
        self.sync_prtsc_hotkey();
        if after_disable_click(self.prtsc.registered, wrote) == Next::OpenSettings {
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
        for wrote in [false, true] {
            for value in [None, Some(0), Some(1), Some(7)] {
                assert_eq!(prtsc_row(false, false, value, wrote), RowState::Off);
                // Off wins even when a stale failure is left.
                assert_eq!(prtsc_row(false, true, value, wrote), RowState::Off);
                assert_eq!(prtsc_row(true, false, value, wrote), RowState::Active);
            }
            // Taken: Windows' setting on (1) or absent is Windows'.
            assert_eq!(prtsc_row(true, true, None, wrote), RowState::TakenByWindows);
            assert_eq!(
                prtsc_row(true, true, Some(1), wrote),
                RowState::TakenByWindows
            );
        }
        // An explicit 0 Vale did not write means another app has the key.
        assert_eq!(prtsc_row(true, true, Some(0), false), RowState::TakenByApp);
    }

    #[test]
    fn written_by_vale_and_still_taken_says_sign_out_not_another_app() {
        // The default Windows 11 case: the write lands, Windows has not
        // picked it up, the retry fails and the value reads back as 0.
        assert_eq!(after_disable_click(false, true), Next::SignOut);
        assert_eq!(
            prtsc_row(true, true, Some(0), true),
            RowState::SignOutToApply
        );
        assert_eq!(ui_row(RowState::SignOutToApply), PrtscRow::SignOutToApply);
        // Through the kept state: the failed retry after the click.
        let mut s = PrtscState::default();
        s.tried(false, || None);
        assert_eq!(s.row(true), RowState::TakenByWindows);
        s.wrote_off = true;
        s.tried(false, || Some(0));
        assert_eq!(s.row(true), RowState::SignOutToApply);
        assert!(!s.click_writes(true), "no button, no second write");
    }

    #[test]
    fn the_button_ends_registered_signed_out_or_opens_windows_settings() {
        assert_eq!(after_disable_click(true, true), Next::Registered);
        assert_eq!(after_disable_click(true, false), Next::Registered);
        // Written: the row says to sign out, nothing opens.
        assert_eq!(after_disable_click(false, true), Next::SignOut);
        // Not written: Windows' own page, for the owner to switch it.
        assert_eq!(after_disable_click(false, false), Next::OpenSettings);
        assert!(WINDOWS_KEYBOARD_PAGE.starts_with("ms-settings:"));
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
