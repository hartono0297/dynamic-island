//! What the Pintasan page's tiles do. The island collapses first, then the
//! action runs a moment later, once the window the user was in is in front
//! again: Win+V and Win+. open at its caret, and a screen clip must not
//! catch the panel.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_LWIN, VK_OEM_PERIOD, VK_V,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

use crate::i18n::Key;
use crate::model::Shortcut;
use crate::ui::geometry::ANIM_MS;
use crate::ui::icons::Icon;

/// How a shortcut is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `ShellExecute` "open" on a URI.
    Open(&'static str),
    /// `ShellExecute` "open" on a program, by its path under `%SystemRoot%`,
    /// so a same-named exe in the working directory is never picked.
    System(&'static str),
    /// The Windows key with this key.
    WinKey(VIRTUAL_KEY),
    /// `LockWorkStation`.
    Lock,
    /// Explorer on a drive's root ("C:\"), from the System page.
    Drive(char),
}

/// A drive's root as Explorer takes it: "C:\".
pub fn drive_root(letter: char) -> String {
    format!("{letter}:\\")
}

pub fn shortcut_action(s: Shortcut) -> Action {
    match s {
        Shortcut::Screenshot => Action::Open("ms-screenclip:"),
        Shortcut::Clipboard => Action::WinKey(VK_V),
        Shortcut::Emoji => Action::WinKey(VK_OEM_PERIOD),
        Shortcut::Explorer => Action::System("explorer.exe"),
        Shortcut::Calculator => Action::System(r"System32\calc.exe"),
        Shortcut::TaskManager => Action::System(r"System32\taskmgr.exe"),
        Shortcut::WindowsSettings => Action::Open("ms-settings:"),
        Shortcut::Lock => Action::Lock,
    }
}

/// The tile's label.
pub fn label(s: Shortcut) -> Key {
    match s {
        Shortcut::Screenshot => Key::ShortcutScreenshot,
        Shortcut::Clipboard => Key::ShortcutClipboard,
        Shortcut::Emoji => Key::ShortcutEmoji,
        Shortcut::Explorer => Key::ShortcutExplorer,
        Shortcut::Calculator => Key::ShortcutCalculator,
        Shortcut::TaskManager => Key::ShortcutTaskManager,
        Shortcut::WindowsSettings => Key::ShortcutSettings,
        Shortcut::Lock => Key::ShortcutLock,
    }
}

/// The tile's Studio icon.
pub fn icon(s: Shortcut) -> Icon {
    match s {
        Shortcut::Screenshot => Icon::Camera,
        Shortcut::Clipboard => Icon::Clipboard,
        Shortcut::Emoji => Icon::Emoji,
        Shortcut::Explorer => Icon::Folder,
        Shortcut::Calculator => Icon::Calc,
        Shortcut::TaskManager => Icon::Activity,
        Shortcut::WindowsSettings => Icon::Gear,
        Shortcut::Lock => Icon::Lock,
    }
}

/// Milliseconds between the click and the action: long enough for the
/// foreground window to have the keyboard again; a screen clip waits for
/// the panel's collapse to finish so it is not in the picture.
pub fn delay_ms(action: Action) -> u32 {
    match action {
        Action::Open("ms-screenclip:") => ANIM_MS as u32 + 60,
        _ => 120,
    }
}

/// Carries `action` out. May pump messages (the shell), so the caller must
/// not hold the app state.
/// `rel` under `%SystemRoot%`.
fn system_path(rel: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    std::path::Path::new(&root).join(rel)
}

pub fn run(action: Action) {
    match action {
        Action::Open(target) => unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(target),
                None,
                None,
                SW_SHOWNORMAL,
            );
        },
        Action::System(rel) => unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(system_path(rel).as_os_str()),
                None,
                None,
                SW_SHOWNORMAL,
            );
        },
        Action::Drive(letter) => unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(system_path("explorer.exe").as_os_str()),
                &HSTRING::from(drive_root(letter)),
                None,
                SW_SHOWNORMAL,
            );
        },
        Action::WinKey(key) => {
            let input = |vk: VIRTUAL_KEY, up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: if up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let inputs = [
                input(VK_LWIN, false),
                input(key, false),
                input(key, true),
                input(VK_LWIN, true),
            ];
            unsafe {
                SendInput(&inputs, size_of::<INPUT>() as i32);
            }
        }
        Action::Lock => unsafe {
            let _ = windows::Win32::System::Shutdown::LockWorkStation();
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tile_maps_to_its_pinned_action() {
        let got: Vec<(Shortcut, Action)> = Shortcut::ALL
            .into_iter()
            .map(|s| (s, shortcut_action(s)))
            .collect();
        assert_eq!(
            got,
            [
                (Shortcut::Screenshot, Action::Open("ms-screenclip:")),
                (Shortcut::Clipboard, Action::WinKey(VK_V)),
                (Shortcut::Emoji, Action::WinKey(VK_OEM_PERIOD)),
                (Shortcut::Explorer, Action::System("explorer.exe")),
                (Shortcut::Calculator, Action::System(r"System32\calc.exe")),
                (
                    Shortcut::TaskManager,
                    Action::System(r"System32\taskmgr.exe")
                ),
                (Shortcut::WindowsSettings, Action::Open("ms-settings:")),
                (Shortcut::Lock, Action::Lock),
            ]
        );
    }

    #[test]
    fn tiles_have_distinct_labels_and_icons() {
        for (i, a) in Shortcut::ALL.iter().enumerate() {
            for b in &Shortcut::ALL[i + 1..] {
                assert_ne!(label(*a), label(*b));
                assert_ne!(icon(*a), icon(*b));
            }
        }
    }

    #[test]
    fn actions_wait_for_the_collapse() {
        let screenshot = shortcut_action(Shortcut::Screenshot);
        assert!(u64::from(delay_ms(screenshot)) > ANIM_MS, "after the morph");
        for s in Shortcut::ALL {
            let ms = delay_ms(shortcut_action(s));
            assert!((100..=400).contains(&ms), "{s:?}: {ms}");
        }
        assert_eq!(delay_ms(Action::Drive('C')), 120);
        assert_eq!(drive_root('D'), r"D:\");
    }
}
