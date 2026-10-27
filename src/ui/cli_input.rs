//! The CLI page's input: one hidden child EDIT of the island. It is never
//! drawn (a layered window updated with `UpdateLayeredWindow` shows only its
//! own bitmap, not its children); the panel draws its text, caret and
//! selection. The EDIT is there for what a real control gives for free:
//! IME, paste, undo, word moves and selection.
//!
//! Keys the page handles itself come back through the callback given to
//! `create` (`Key`); every other change posts `WM_APP_CLI_REDRAW` to the
//! island, once until it is handled, so the panel redraws.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{EM_GETSEL, EM_LIMITTEXT, EM_SETSEL};
use windows::Win32::UI::Input::Ime::{
    CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_A, VK_C, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_PRIOR,
    VK_RETURN, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DLGC_WANTALLKEYS, ES_AUTOHSCROLL, GetWindowTextLengthW, GetWindowTextW, HMENU,
    PostMessageW, SendMessageW, SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CHAR,
    WM_CLEAR, WM_CUT, WM_GETDLGCODE, WM_IME_COMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
    WM_KILLFOCUS, WM_NCDESTROY, WM_PASTE, WM_SETFOCUS, WM_UNDO, WS_CHILD, WS_VISIBLE,
};
use windows::core::{HSTRING, Result, w};

/// Posted to the island when the input's text or selection changed.
/// WM_APP + 1..10 are taken.
pub const WM_APP_CLI_REDRAW: u32 = WM_APP + 11;

/// Keys the CLI page acts on instead of the EDIT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Up,
    Down,
    /// Take the highlighted suggestion.
    Tab,
    /// Ctrl+C without a selection: stop the running command.
    Stop,
    /// Let go of the keyboard.
    Escape,
    PageUp,
    PageDown,
    /// The EDIT lost the keyboard.
    Blur,
}

const SUBCLASS_ID: usize = 1;
const ID_INPUT: isize = 201;

thread_local! {
    static NOTIFY: Cell<Option<fn(Key)>> = const { Cell::new(None) };
    /// The island (where redraws are posted) and whether one is pending.
    static PARENT: Cell<isize> = const { Cell::new(0) };
    static REDRAW_POSTED: Cell<bool> = const { Cell::new(false) };
    /// Where the composition window goes, in the EDIT's client pixels.
    static IME_AT: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

pub struct CliInput {
    hwnd: HWND,
}

impl CliInput {
    /// A 1 × 1 EDIT in the island's top-left corner (a transparent pixel):
    /// its client coordinates are the island's, which the IME position needs.
    pub fn create(parent: HWND, notify: fn(Key)) -> Result<Self> {
        let instance = unsafe { GetModuleHandleW(None)? };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                1,
                1,
                Some(parent),
                Some(HMENU(ID_INPUT as *mut _)),
                Some(instance.into()),
                None,
            )?
        };
        NOTIFY.set(Some(notify));
        PARENT.set(parent.0 as isize);
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(input_proc), SUBCLASS_ID, 0);
            // A command line, not a document: 8K characters is plenty.
            SendMessageW(hwnd, EM_LIMITTEXT, Some(WPARAM(8192)), Some(LPARAM(0)));
        }
        Ok(Self { hwnd })
    }

    pub fn text(&self) -> String {
        let len = unsafe { GetWindowTextLengthW(self.hwnd) }.max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = unsafe { GetWindowTextW(self.hwnd, &mut buf) }.max(0) as usize;
        String::from_utf16_lossy(&buf[..n.min(len)])
    }

    /// Replaces the text; the caret goes to its end.
    pub fn set_text(&self, text: &str) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(text));
        }
        let end = text.encode_utf16().count();
        unsafe {
            SendMessageW(
                self.hwnd,
                EM_SETSEL,
                Some(WPARAM(end)),
                Some(LPARAM(end as isize)),
            );
        }
        request_redraw();
    }

    /// Start and end of the selection in UTF-16 units (equal: the caret).
    pub fn selection(&self) -> (usize, usize) {
        selection(self.hwnd)
    }

    pub fn has_focus(&self) -> bool {
        unsafe { GetFocus() == self.hwnd }
    }

    /// Gives the EDIT the keyboard; the island must be active already.
    pub fn focus(&self) -> bool {
        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
        self.has_focus()
    }

    /// Puts the IME's composition window at the caret the panel drew
    /// (client pixels of the island, which are the EDIT's).
    pub fn set_ime_caret(&self, caret: RECT) {
        IME_AT.set(Some((caret.left, caret.top)));
        place_ime(self.hwnd);
    }
}

fn selection(hwnd: HWND) -> (usize, usize) {
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(
            hwnd,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
    }
    (start.min(end) as usize, start.max(end) as usize)
}

fn place_ime(hwnd: HWND) {
    let Some((x, y)) = IME_AT.get() else { return };
    unsafe {
        let imc = ImmGetContext(hwnd);
        if imc.is_invalid() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x, y },
            rcArea: RECT::default(),
        };
        let _ = ImmSetCompositionWindow(imc, &form);
        let _ = ImmReleaseContext(hwnd, imc);
    }
}

/// The island handled the last redraw request.
pub fn redraw_done() {
    REDRAW_POSTED.set(false);
}

/// Asks the island to redraw the input, once until `redraw_done`.
pub fn request_redraw() {
    if REDRAW_POSTED.replace(true) {
        return;
    }
    let parent = HWND(PARENT.get() as *mut _);
    if unsafe { PostMessageW(Some(parent), WM_APP_CLI_REDRAW, WPARAM(0), LPARAM(0)) }.is_err() {
        REDRAW_POSTED.set(false);
    }
}

fn notify(key: Key) {
    if let Some(f) = NOTIFY.get() {
        f(key);
    }
}

fn ctrl_down() -> bool {
    let state = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) };
    state < 0
}

unsafe extern "system" fn input_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // Tab, Enter and Esc come to the EDIT, never to dialog navigation.
        WM_GETDLGCODE => {
            let code = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            return LRESULT(code.0 | DLGC_WANTALLKEYS as isize);
        }
        WM_KEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as u16;
            let copy_free = key == VK_C.0 && ctrl_down() && {
                let (a, b) = selection(hwnd);
                a == b
            };
            let ours = match key {
                k if k == VK_RETURN.0 => Some(Key::Enter),
                k if k == VK_UP.0 => Some(Key::Up),
                k if k == VK_DOWN.0 => Some(Key::Down),
                k if k == VK_TAB.0 => Some(Key::Tab),
                k if k == VK_ESCAPE.0 => Some(Key::Escape),
                k if k == VK_PRIOR.0 => Some(Key::PageUp),
                k if k == VK_NEXT.0 => Some(Key::PageDown),
                // Ctrl+C copies a selection; without one it stops the run.
                _ if copy_free => Some(Key::Stop),
                _ => None,
            };
            if let Some(k) = ours {
                notify(k);
                return LRESULT(0);
            }
            if key == VK_A.0 && ctrl_down() {
                unsafe {
                    SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                }
                request_redraw();
                return LRESULT(0);
            }
            // Arrows, Home, End, Delete: the caret or the text moves.
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            request_redraw();
            return result;
        }
        WM_CHAR => {
            let ch = wparam.0;
            // Enter, Esc, Tab, Ctrl+A and Ctrl+C (handled above) must not beep.
            let no_selection = || {
                let (a, b) = selection(hwnd);
                a == b
            };
            if matches!(ch, 0x0D | 0x0A | 0x1B | 0x09 | 0x01) || (ch == 0x03 && no_selection()) {
                return LRESULT(0);
            }
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            request_redraw();
            return result;
        }
        WM_PASTE | WM_CUT | WM_CLEAR | WM_UNDO => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            request_redraw();
            return result;
        }
        // The EDIT places the composition window at its own (hidden) caret;
        // it goes where the panel draws the caret instead.
        WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            place_ime(hwnd);
            request_redraw();
            return result;
        }
        WM_SETFOCUS => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            request_redraw();
            return result;
        }
        WM_KILLFOCUS => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            notify(Key::Blur);
            return result;
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(input_proc), SUBCLASS_ID);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
