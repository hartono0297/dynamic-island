//! Notification-area icon, its menu, and the autostart registry value.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, HICON, IDI_APPLICATION,
    LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterWindowMessageW,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
    WM_NULL,
};
use windows::core::{Error, HSTRING, Result, w};

use crate::config::{Config, Style};
use crate::i18n::{Key, text};
use crate::ui::app_icon;

/// Sent by the shell for mouse activity on the tray icon; `lparam` holds the
/// mouse message (legacy notify-icon version).
pub const WM_APP_TRAY: u32 = WM_APP + 2;
const TRAY_ID: u32 = 1;
const TOOLTIP: &str = "Dynamic Island";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "DynamicIsland";

/// The logo icon the tray currently shows, if loaded (null otherwise). Kept
/// alive while the icon is in the notification area and replaced on re-add.
static TRAY_ICON: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    Settings,
    StylePill,
    StyleNotch,
    Exit,
}

const COMMANDS: [MenuCmd; 4] = [
    MenuCmd::Settings,
    MenuCmd::StylePill,
    MenuCmd::StyleNotch,
    MenuCmd::Exit,
];

impl MenuCmd {
    /// Menu item id; 0 is what `TrackPopupMenu` returns for "nothing chosen".
    fn id(self) -> usize {
        COMMANDS.iter().position(|&c| c == self).unwrap_or(0) + 1
    }

    fn from_id(id: usize) -> Option<Self> {
        id.checked_sub(1).and_then(|i| COMMANDS.get(i).copied())
    }
}

#[derive(Debug, PartialEq)]
enum MenuItem {
    Entry {
        cmd: MenuCmd,
        label: &'static str,
        checked: bool,
    },
    Separator,
}

/// The menu in `cfg.language`, built each time it opens, so it always
/// shows the current language. Everything else lives in the settings window.
fn menu_items(cfg: &Config) -> Vec<MenuItem> {
    let entry = |cmd, key, checked| MenuItem::Entry {
        cmd,
        label: text(cfg.language, key),
        checked,
    };
    vec![
        entry(MenuCmd::Settings, Key::MenuSettings, false),
        MenuItem::Separator,
        entry(
            MenuCmd::StylePill,
            Key::MenuStylePill,
            cfg.style == Style::Pill,
        ),
        entry(
            MenuCmd::StyleNotch,
            Key::MenuStyleNotch,
            cfg.style == Style::Notch,
        ),
        MenuItem::Separator,
        entry(MenuCmd::Exit, Key::MenuExit, false),
    ]
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

/// Adds the tray icon. Call again when `is_taskbar_created` says so: the
/// icon is lost when Explorer restarts or was not up yet at logon.
pub fn add_tray(hwnd: HWND) -> Result<()> {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_APP_TRAY;
    // Small-icon size for the system DPI: 16 px at 100 %, 20 at 125 %, ...
    let logo = app_icon::small_icon_for_dpi(unsafe { GetDpiForSystem() }).ok();
    data.hIcon = match logo {
        Some(icon) => icon,
        None => unsafe { LoadIconW(None, IDI_APPLICATION)? },
    };
    for (dst, src) in data.szTip.iter_mut().zip(TOOLTIP.encode_utf16()) {
        *dst = src;
    }
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.ok();
    let new = logo.map_or(std::ptr::null_mut(), |icon| icon.0);
    destroy_icon(TRAY_ICON.swap(new, Ordering::Relaxed));
    added
}

pub fn remove_tray(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
    }
    destroy_icon(TRAY_ICON.swap(std::ptr::null_mut(), Ordering::Relaxed));
}

fn destroy_icon(raw: *mut core::ffi::c_void) {
    if !raw.is_null() {
        unsafe {
            let _ = DestroyIcon(HICON(raw));
        }
    }
}

/// True for the message Explorer broadcasts when the taskbar is (re)created.
pub fn is_taskbar_created(msg: u32) -> bool {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    let registered =
        *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    // 0 means registration failed; never mistake WM_NULL for it.
    registered != 0 && msg == registered
}

/// True when a `WM_APP_TRAY` message is a click that should open the menu.
pub fn is_menu_click(lparam: LPARAM) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP};
    matches!(lparam.0 as u32, WM_LBUTTONUP | WM_RBUTTONUP)
}

/// Shows the menu at the cursor and waits for a choice. Runs a modal message
/// loop, so the caller must not hold any state borrow across it.
pub fn show_menu(hwnd: HWND, cfg: &Config) -> Option<MenuCmd> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for item in menu_items(cfg) {
            let _ = match item {
                MenuItem::Entry {
                    cmd,
                    label,
                    checked,
                } => {
                    let flags = if checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    AppendMenuW(menu, flags, cmd.id(), &HSTRING::from(label))
                }
                MenuItem::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, None),
            };
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Without the foreground the menu would not close on a click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        // Lets the menu dismiss properly the next time it opens.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        MenuCmd::from_id(chosen.0 as usize)
    }
}

/// `"<exe>"`, the command the Run value holds.
fn run_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Writes or deletes `HKCU\...\Run\DynamicIsland` for the current exe. Debug
/// builds leave the registry alone, so `cargo run` never points autostart at
/// `target\debug`.
pub fn set_autostart(enabled: bool) -> Result<()> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let result = if enabled {
            write_run_value(key)
        } else {
            match RegDeleteValueW(key, &HSTRING::from(RUN_VALUE)) {
                ERROR_FILE_NOT_FOUND => Ok(()),
                status => status.ok(),
            }
        };
        let _ = RegCloseKey(key);
        result
    }
}

unsafe fn write_run_value(key: HKEY) -> Result<()> {
    let exe = std::env::current_exe().map_err(Error::from)?;
    let bytes: Vec<u8> = run_command(&exe)
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    unsafe { RegSetValueExW(key, &HSTRING::from(RUN_VALUE), None, REG_SZ, Some(&bytes)) }.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    fn labels(items: &[MenuItem]) -> Vec<&'static str> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Entry { label, .. } => *label,
                MenuItem::Separator => "-",
            })
            .collect()
    }

    fn checked(items: &[MenuItem]) -> Vec<MenuCmd> {
        items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Entry {
                    cmd, checked: true, ..
                } => Some(*cmd),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn menu_labels_in_order() {
        assert_eq!(
            labels(&menu_items(&Config::default())),
            vec![
                "Pengaturan…",
                "-",
                "Gaya: Pill",
                "Gaya: Notch",
                "-",
                "Keluar"
            ]
        );
    }

    #[test]
    fn menu_labels_in_english() {
        let en = Config {
            language: Lang::En,
            ..Config::default()
        };
        assert_eq!(
            labels(&menu_items(&en)),
            vec!["Settings…", "-", "Style: Pill", "Style: Notch", "-", "Quit"]
        );
    }

    #[test]
    fn settings_comes_first() {
        let items = menu_items(&Config::default());
        assert!(matches!(
            items.first(),
            Some(MenuItem::Entry {
                cmd: MenuCmd::Settings,
                checked: false,
                ..
            })
        ));
    }

    #[test]
    fn checks_follow_config() {
        let pill = Config::default();
        assert_eq!(checked(&menu_items(&pill)), vec![MenuCmd::StylePill]);
        let notch = Config {
            style: Style::Notch,
            start_with_windows: false,
            ..Config::default()
        };
        assert_eq!(checked(&menu_items(&notch)), vec![MenuCmd::StyleNotch]);
    }

    #[test]
    fn ids_round_trip_and_zero_is_nothing() {
        for cmd in COMMANDS {
            assert_ne!(cmd.id(), 0);
            assert_eq!(MenuCmd::from_id(cmd.id()), Some(cmd));
        }
        assert_eq!(MenuCmd::from_id(0), None);
        assert_eq!(MenuCmd::from_id(COMMANDS.len() + 1), None);
    }

    #[test]
    fn every_command_is_in_the_menu() {
        let items = menu_items(&Config::default());
        for cmd in COMMANDS {
            assert!(
                items
                    .iter()
                    .any(|item| matches!(item, MenuItem::Entry { cmd: c, .. } if *c == cmd)),
                "{cmd:?}"
            );
        }
    }

    #[test]
    fn run_command_quotes_the_exe() {
        assert_eq!(
            run_command(Path::new(r"D:\Program Files\dynamic-island.exe")),
            r#""D:\Program Files\dynamic-island.exe""#
        );
    }
}
