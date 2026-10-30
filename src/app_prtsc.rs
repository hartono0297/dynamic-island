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
//! (`retry_hotkeys`), like the other hotkeys.

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

/// What the PrtSc key holds: whether Vale has the key, and whether the last
/// registration failed.
#[derive(Default)]
pub(super) struct PrtscState {
    pub registered: bool,
    pub failed: bool,
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
    } else if windows_value == Some(0) {
        RowState::TakenByApp
    } else {
        RowState::TakenByWindows
    }
}

/// What the button does after its registry write and the retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// The key is Vale's now.
    Registered,
    /// Still taken: open Windows' keyboard settings for the owner.
    OpenSettings,
}

pub(super) fn after_disable_click(retry_ok: bool) -> Next {
    if retry_ok {
        Next::Registered
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
                self.prtsc.registered = ok;
                self.prtsc.failed = !ok;
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

    /// The row's state for Settings, with the registry read only while the
    /// key is wanted and taken (`windows` for the model: Windows' setting
    /// still gives the key to Snipping Tool).
    pub(super) fn prtsc_model(&self) -> (bool, bool) {
        let on = self.config.screenshot.print_screen;
        let value = (on && self.prtsc.failed).then(read_windows_value).flatten();
        let row = prtsc_row(on, self.prtsc.failed, value);
        (
            matches!(row, RowState::TakenByWindows | RowState::TakenByApp),
            row == RowState::TakenByWindows,
        )
    }

    /// "Matikan di Windows", on click only: Windows' own setting is switched
    /// off (HKCU), then the registration is tried again. If it still fails
    /// Windows' keyboard settings open for the owner.
    pub(super) fn turn_off_windows_print_screen(&mut self) {
        if !self.config.screenshot.print_screen {
            return;
        }
        let _ = write_windows_value_off();
        self.prtsc.failed = false;
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
        // Taken: Windows' setting on (1) or absent is Windows'...
        assert_eq!(prtsc_row(true, true, None), RowState::TakenByWindows);
        assert_eq!(prtsc_row(true, true, Some(1)), RowState::TakenByWindows);
        // ...an explicit 0 means another app has the key.
        assert_eq!(prtsc_row(true, true, Some(0)), RowState::TakenByApp);
    }

    #[test]
    fn the_button_ends_registered_or_opens_windows_settings() {
        assert_eq!(after_disable_click(true), Next::Registered);
        assert_eq!(after_disable_click(false), Next::OpenSettings);
        assert!(WINDOWS_KEYBOARD_PAGE.starts_with("ms-settings:"));
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
