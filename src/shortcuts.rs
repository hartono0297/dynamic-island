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

/// How a shortcut is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `ShellExecute` "open": a URI or a program on the PATH.
    Open(&'static str),
    /// The Windows key with this key.
    WinKey(VIRTUAL_KEY),
    /// `LockWorkStation`.
    Lock,
}

pub fn shortcut_action(s: Shortcut) -> Action {
    match s {
        Shortcut::Screenshot => Action::Open("ms-screenclip:"),
        Shortcut::Clipboard => Action::WinKey(VK_V),
        Shortcut::Emoji => Action::WinKey(VK_OEM_PERIOD),
        Shortcut::Explorer => Action::Open("explorer.exe"),
        Shortcut::Calculator => Action::Open("calc.exe"),
        Shortcut::TaskManager => Action::Open("taskmgr.exe"),
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

/// The tile's icon in Segoe Fluent Icons / Segoe MDL2 Assets.
pub fn glyph(s: Shortcut) -> &'static str {
    match s {
        // Camera.
        Shortcut::Screenshot => "\u{E722}",
        // Paste: a clipboard.
        Shortcut::Clipboard => "\u{E77F}",
        Shortcut::Emoji => "\u{E76E}",
        // FileExplorer.
        Shortcut::Explorer => "\u{EC50}",
        Shortcut::Calculator => "\u{E8EF}",
        // Diagnostic: an activity line.
        Shortcut::TaskManager => "\u{E9D9}",
        Shortcut::WindowsSettings => "\u{E713}",
        Shortcut::Lock => "\u{E72E}",
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
                (Shortcut::Explorer, Action::Open("explorer.exe")),
                (Shortcut::Calculator, Action::Open("calc.exe")),
                (Shortcut::TaskManager, Action::Open("taskmgr.exe")),
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
                assert_ne!(glyph(*a), glyph(*b));
            }
            assert_eq!(glyph(*a).chars().count(), 1);
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
    }
}
