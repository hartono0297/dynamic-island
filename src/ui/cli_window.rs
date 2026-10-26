//! The CLI window: a small window under the panel, styled like the editors
//! (dark or light chrome per the window mode) around a terminal that is dark
//! in both modes. A read-only output field over a `>` input line; under the
//! terminal, the shell and folder and a Stop button.
//!
//! The window only shows and asks: commands, output and history live in the
//! island (`App`), which pushes output here (`append`, `reset`,
//! `set_status`) and gets `CliAction`s back through `WM_APP_CLI_ACTION`.
//! Closing it (Esc) leaves a running command running.
//!
//! Keys in the input line: Enter runs, Up/Down walk the history, Ctrl+C
//! stops a running command (else copies), PageUp/PageDown scroll the
//! output, Esc closes. Typing in the output goes to the input line.
//!
//! One window under the panel at a time: opening the CLI while an editor is
//! open brings the editor forward instead (it may hold unsaved text), and
//! opening an editor closes the CLI window (nothing in it can be lost).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DeleteObject, EndPaint, FW_NORMAL, FillRect, GetDC, GetTextFaceW, HBRUSH, HDC,
    HFONT, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, ReleaseDC,
    RoundRect, SelectObject, SetBkColor, SetTextColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    EM_GETSEL, EM_LIMITTEXT, EM_REPLACESEL, EM_SCROLLCARET, EM_SETSEL, SetWindowTheme,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, SetFocus, VK_A, VK_C, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_PRIOR,
    VK_RETURN, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL, ES_READONLY, GetClientRect, GetDlgCtrlID,
    GetForegroundWindow, GetWindowTextLengthW, HMENU, IDC_ARROW, IsWindow, LoadCursorW, MoveWindow,
    PostMessageW, RegisterClassExW, SB_PAGEDOWN, SB_PAGEUP, SW_SHOW, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
    SetWindowTextW, ShowWindow, SwitchToThisWindow, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_NCDESTROY,
    WM_PAINT, WM_SETFONT, WM_SIZE, WM_VSCROLL, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

use super::app_icon;
use super::editor::{
    apply_mode, blend, brushes, colorref, create_font, editor_rect, init_common_controls, key_down,
    monitor_at, send, window_text,
};
use crate::cli::{History, HistoryCursor, Shell};
use crate::i18n::{Key, tr};
use crate::theme::{self, TERMINAL_BG, TERMINAL_TEXT};

/// Posted by the CLI window to its owner: `lparam` is a `Box<CliAction>`
/// raw pointer the owner frees (`take_action`). WM_APP + 1..10 are taken.
pub const WM_APP_CLI_ACTION: u32 = WM_APP + 11;

/// What the window asks of the island.
#[derive(Clone, Debug, PartialEq)]
pub enum CliAction {
    /// Enter in the input line (never blank).
    Run(String),
    /// Ctrl+C or the Stop button.
    Stop,
    /// The window became active (true) or stopped being active.
    Focus(bool),
    /// The window is gone.
    Closed,
}

/// Takes ownership of a `WM_APP_CLI_ACTION`'s `lparam`.
///
/// # Safety
/// `lparam` must come from a `WM_APP_CLI_ACTION` message, taken once.
pub unsafe fn take_action(lparam: LPARAM) -> Option<CliAction> {
    let ptr = lparam.0 as *mut CliAction;
    (!ptr.is_null()).then(|| *unsafe { Box::from_raw(ptr) })
}

/// What a new window starts with.
pub struct CliInit {
    /// The whole output so far (`\r\n` line breaks).
    pub text: String,
    pub cwd: String,
    pub shell: Shell,
    pub running: bool,
    /// For Up/Down; oldest first.
    pub history: History,
}

/// Logical outer size.
const SIZE_LOGICAL: (f32, f32) = (560.0, 320.0);
const ID_OUTPUT: i32 = 101;
const ID_INPUT: i32 = 102;
const ID_PROMPT: i32 = 103;
const ID_STATUS: i32 = 104;
const ID_STOP: i32 = 105;
const SS_CENTERIMAGE: u32 = 0x0200;
const SS_PATHELLIPSIS: u32 = 0x8000;
const SUBCLASS_ID: usize = 1;
/// Between the output and the input line inside the terminal.
const TERMINAL_LINE: u32 = 0x2A2A2E;

/// Where everything goes in a client area of `cw` × `ch` logical px: the
/// terminal (output over the input row) and, under it, the status and Stop.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    terminal: [f32; 4],
    output: [f32; 4],
    /// The line between output and input.
    rule_y: f32,
    prompt: [f32; 4],
    input: [f32; 4],
    status: [f32; 4],
    stop: [f32; 4],
}

fn layout(cw: f32, ch: f32) -> Layout {
    let m = 10.0;
    let pad = 8.0;
    let (bar_h, stop_w) = (28.0, 96.0);
    let bar_y = ch - m - bar_h;
    let terminal = [m, m, cw - 2.0 * m, (bar_y - 8.0 - m).max(60.0)];
    let [tx, ty, tw, th] = terminal;
    let row_h = 22.0;
    let row_y = ty + th - pad - row_h;
    let rule_y = row_y - 6.0;
    Layout {
        terminal,
        output: [
            tx + pad,
            ty + pad,
            tw - pad,
            (rule_y - 6.0 - ty - pad).max(20.0),
        ],
        rule_y,
        prompt: [tx + pad, row_y, 14.0, row_h],
        input: [tx + pad + 16.0, row_y, tw - 2.0 * pad - 16.0, row_h],
        status: [m + 2.0, bar_y, cw - 2.0 * m - stop_w - 10.0, bar_h],
        stop: [cw - m - stop_w, bar_y, stop_w, bar_h],
    }
}

/// "PowerShell · C:\Users\x".
fn status_text(shell: Shell, cwd: &str) -> String {
    format!("{} \u{00B7} {cwd}", shell.name())
}

struct Cli {
    hwnd: HWND,
    owner: HWND,
    /// Foreground window before the window opened; gets the focus back on Esc.
    previous: HWND,
    scale: f32,
    fonts: Fonts,
    controls: Vec<(i32, HWND)>,
    history: History,
    cursor: HistoryCursor,
    shell: Shell,
    cwd: String,
    running: bool,
    /// `Closed` was posted.
    finished: bool,
}

struct Fonts {
    ui: HFONT,
    mono: HFONT,
}

impl Fonts {
    fn new(scale: f32) -> Self {
        Fonts {
            ui: create_font(scale, CLEARTYPE_QUALITY),
            mono: mono_font(scale),
        }
    }

    fn apply(&self, control: HWND, id: i32) {
        let font = if matches!(id, ID_OUTPUT | ID_INPUT | ID_PROMPT) {
            self.mono
        } else {
            self.ui
        };
        send(control, WM_SETFONT, font.0 as usize, 1);
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in [self.ui, self.mono] {
            if !font.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
            }
        }
    }
}

fn font_named(scale: f32, face: PCWSTR) -> HFONT {
    unsafe {
        CreateFontW(
            -(13.0 * scale).round() as i32,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            face,
        )
    }
}

/// Cascadia Mono, or Consolas where it is not installed (GDI would quietly
/// substitute some other face).
fn mono_font(scale: f32) -> HFONT {
    let font = font_named(scale, w!("Cascadia Mono"));
    if font_face(font).eq_ignore_ascii_case("Cascadia Mono") {
        return font;
    }
    unsafe {
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
    font_named(scale, w!("Consolas"))
}

/// The face GDI actually picked for `font`.
fn font_face(font: HFONT) -> String {
    unsafe {
        let dc = GetDC(None);
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let mut name = [0u16; 64];
        let n = GetTextFaceW(dc, Some(&mut name)).max(1) as usize - 1;
        SelectObject(dc, old);
        ReleaseDC(None, dc);
        String::from_utf16_lossy(&name[..n.min(name.len())])
    }
}

thread_local! {
    static CLI: RefCell<Option<Cli>> = const { RefCell::new(None) };
    /// The terminal background, created once.
    static TERMINAL_BRUSH: Cell<Option<HBRUSH>> = const { Cell::new(None) };
}

fn terminal_brush() -> HBRUSH {
    TERMINAL_BRUSH.with(|b| {
        if let Some(brush) = b.get() {
            return brush;
        }
        let brush = unsafe { CreateSolidBrush(colorref(TERMINAL_BG)) };
        b.set(Some(brush));
        brush
    })
}

fn with_cli<R>(f: impl FnOnce(&mut Cli) -> R) -> Option<R> {
    CLI.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        slot.as_mut().map(f)
    })
}

fn open_hwnd() -> Option<HWND> {
    CLI.with(|cell| cell.try_borrow().ok()?.as_ref().map(|c| c.hwnd))
        .filter(|h| unsafe { IsWindow(Some(*h)) }.as_bool())
}

pub fn is_open() -> bool {
    open_hwnd().is_some()
}

impl Cli {
    fn control(&self, id: i32) -> Option<HWND> {
        self.controls
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, h)| *h)
    }

    fn layout(&self) {
        let mut rc = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rc) }.is_err() {
            return;
        }
        let s = self.scale.max(0.5);
        let l = layout(
            (rc.right - rc.left) as f32 / s,
            (rc.bottom - rc.top) as f32 / s,
        );
        let px = |v: f32| (v * s).round() as i32;
        for (id, [x, y, w, h]) in [
            (ID_OUTPUT, l.output),
            (ID_PROMPT, l.prompt),
            (ID_INPUT, l.input),
            (ID_STATUS, l.status),
            (ID_STOP, l.stop),
        ] {
            if let Some(c) = self.control(id) {
                unsafe {
                    let _ = MoveWindow(c, px(x), px(y), px(w).max(0), px(h).max(0), true);
                }
            }
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }

    fn show_status(&self) {
        if let Some(label) = self.control(ID_STATUS) {
            let text = status_text(self.shell, &self.cwd);
            unsafe {
                let _ = SetWindowTextW(label, &HSTRING::from(text));
            }
        }
        if let Some(stop) = self.control(ID_STOP) {
            unsafe {
                let _ = EnableWindow(stop, self.running);
            }
        }
    }

    fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        let fonts = Fonts::new(scale);
        for (id, c) in &self.controls {
            fonts.apply(*c, *id);
        }
        self.fonts = fonts;
    }
}

fn post(owner: HWND, action: CliAction) {
    let ptr = Box::into_raw(Box::new(action));
    let posted = unsafe {
        PostMessageW(
            Some(owner),
            WM_APP_CLI_ACTION,
            WPARAM(0),
            LPARAM(ptr as isize),
        )
    };
    if posted.is_err() {
        drop(unsafe { Box::from_raw(ptr) });
    }
}

const CLASS_NAME: PCWSTR = w!("DynamicIslandCli");

fn register_class(instance: HINSTANCE) -> Result<()> {
    let (icon, icon_small) = app_icon::class_icons(instance);
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hIcon: icon,
            hIconSm: icon_small,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            let error = Error::from_thread();
            if error.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Opens the window owned by `owner`, centred under `anchor_bottom_center`
/// (screen px), or brings the open one forward.
pub fn open(owner: HWND, anchor_bottom_center: POINT, init: CliInit) -> Result<HWND> {
    if let Some(hwnd) = open_hwnd() {
        bring_to_front(hwnd);
        return Ok(hwnd);
    }
    CLI.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = None;
        }
    });
    init_common_controls();
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    register_class(instance)?;
    let (scale, work) = monitor_at(anchor_bottom_center);
    let size = SIZE {
        cx: (SIZE_LOGICAL.0 * scale).round() as i32,
        cy: (SIZE_LOGICAL.1 * scale).round() as i32,
    };
    let rc = editor_rect(anchor_bottom_center, size, work);
    let previous = unsafe { GetForegroundWindow() };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS_NAME,
            &HSTRING::from(tr(Key::Cli)),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            rc.left,
            rc.top,
            size.cx,
            size.cy,
            Some(owner),
            None,
            Some(instance),
            None,
        )?
    };
    let fonts = Fonts::new(scale);
    let controls = match create_controls(hwnd, instance, &fonts, &init) {
        Ok(c) => c,
        Err(error) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
    };
    let cli = Cli {
        hwnd,
        owner,
        previous,
        scale,
        fonts,
        controls,
        history: init.history,
        cursor: HistoryCursor::default(),
        shell: init.shell,
        cwd: init.cwd,
        running: init.running,
        finished: false,
    };
    cli.layout();
    cli.show_status();
    restyle_window(&cli);
    let output = cli.control(ID_OUTPUT);
    let stored = CLI.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => {
            *slot = Some(cli);
            None
        }
        Err(_) => Some(cli),
    });
    if let Some(cli) = stored {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        drop(cli);
        return Err(Error::empty());
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    if let Some(output) = output {
        scroll_to_end(output);
    }
    bring_to_front(hwnd);
    Ok(hwnd)
}

fn create_controls(
    parent: HWND,
    instance: HINSTANCE,
    fonts: &Fonts,
    init: &CliInit,
) -> Result<Vec<(i32, HWND)>> {
    let read_only = (ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_NOHIDESEL) as u32;
    let specs: [(i32, PCWSTR, String, u32, bool); 5] = [
        (
            ID_OUTPUT,
            w!("EDIT"),
            init.text.clone(),
            read_only | WS_VSCROLL.0,
            true,
        ),
        (ID_PROMPT, w!("STATIC"), ">".into(), SS_CENTERIMAGE, false),
        (
            ID_INPUT,
            w!("EDIT"),
            String::new(),
            ES_AUTOHSCROLL as u32 | WS_TABSTOP.0,
            true,
        ),
        (
            ID_STATUS,
            w!("STATIC"),
            String::new(),
            SS_CENTERIMAGE | SS_PATHELLIPSIS,
            false,
        ),
        (
            ID_STOP,
            w!("BUTTON"),
            tr(Key::CliStop).into(),
            BS_PUSHBUTTON as u32 | WS_TABSTOP.0,
            false,
        ),
    ];
    let mut out = Vec::new();
    for (id, class, text, style, subclass) in specs {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                &HSTRING::from(text),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(id as isize as *mut c_void)),
                Some(instance),
                None,
            )?
        };
        fonts.apply(hwnd, id);
        if subclass {
            unsafe {
                let _ = SetWindowSubclass(hwnd, Some(control_proc), SUBCLASS_ID, id as usize);
            }
        }
        out.push((id, hwnd));
    }
    if let Some((_, output)) = out.iter().find(|(id, _)| *id == ID_OUTPUT) {
        // As much text as the buffer holds (the default stops at 32K chars).
        send(*output, EM_LIMITTEXT, 0, 0);
    }
    Ok(out)
}

fn bring_to_front(hwnd: HWND) {
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() != hwnd {
            SwitchToThisWindow(hwnd, true);
        }
    }
    focus_input(hwnd);
}

fn focus_input(hwnd: HWND) {
    let input = with_cli(|c| (c.hwnd == hwnd).then(|| c.control(ID_INPUT)).flatten()).flatten();
    if let Some(input) = input {
        unsafe {
            let _ = SetFocus(Some(input));
        }
    }
}

fn scroll_to_end(edit: HWND) {
    let end = unsafe { GetWindowTextLengthW(edit) }.max(0) as usize;
    send(edit, EM_SETSEL, end, end as isize);
    send(edit, EM_SCROLLCARET, 0, 0);
}

/// Adds lines at the end of the output and scrolls to them.
pub fn append(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(output) = with_cli(|c| c.control(ID_OUTPUT)).flatten() else {
        return;
    };
    let len = unsafe { GetWindowTextLengthW(output) }.max(0) as usize;
    let mut text = if len > 0 {
        String::from("\r\n")
    } else {
        String::new()
    };
    text.push_str(&lines.join("\r\n"));
    let wide = HSTRING::from(text);
    send(output, EM_SETSEL, len, len as isize);
    send(output, EM_REPLACESEL, 0, wide.as_ptr() as isize);
    send(output, EM_SCROLLCARET, 0, 0);
}

/// Replaces the whole output (after `cls`, or once old lines scrolled away).
pub fn reset(text: &str) {
    let Some(output) = with_cli(|c| c.control(ID_OUTPUT)).flatten() else {
        return;
    };
    unsafe {
        let _ = SetWindowTextW(output, &HSTRING::from(text));
    }
    scroll_to_end(output);
}

/// The folder, shell and whether a command runs (Stop is on only then).
pub fn set_status(cwd: &str, shell: Shell, running: bool) {
    with_cli(|c| {
        if (c.cwd.as_str(), c.shell, c.running) == (cwd, shell, running) {
            return;
        }
        c.cwd = cwd.to_owned();
        c.shell = shell;
        c.running = running;
        c.show_status();
    });
}

/// Closes the window; a running command goes on.
pub fn close() {
    if let Some(hwnd) = open_hwnd() {
        finish(hwnd);
    }
}

/// Posts `Closed` once, hands the foreground back when the window has it,
/// and destroys the window.
fn finish(hwnd: HWND) {
    let Some((owner, previous, first)) = with_cli(|c| {
        let first = !c.finished;
        c.finished = true;
        (c.owner, c.previous, first)
    }) else {
        return;
    };
    unsafe {
        if GetForegroundWindow() == hwnd
            && !previous.is_invalid()
            && IsWindow(Some(previous)).as_bool()
        {
            let _ = SetForegroundWindow(previous);
        }
        let _ = DestroyWindow(hwnd);
    }
    if first {
        post(owner, CliAction::Closed);
    }
}

/// Window title and Stop in the language now in effect.
pub fn relabel() {
    let Some((hwnd, stop)) = with_cli(|c| (c.hwnd, c.control(ID_STOP))) else {
        return;
    };
    unsafe {
        let _ = SetWindowTextW(hwnd, &HSTRING::from(tr(Key::Cli)));
        if let Some(stop) = stop {
            let _ = SetWindowTextW(stop, &HSTRING::from(tr(Key::CliStop)));
        }
    }
}

/// Restyles for the window mode now in effect.
pub fn restyle() {
    CLI.with(|cell| {
        if let Ok(slot) = cell.try_borrow()
            && let Some(cli) = slot.as_ref()
        {
            restyle_window(cli);
        }
    });
}

fn restyle_window(cli: &Cli) {
    let p = theme::current_window();
    let themed: Vec<HWND> = cli.control(ID_STOP).into_iter().collect();
    apply_mode(cli.hwnd, &themed, &p);
    unsafe {
        // The terminal is dark in both modes, its scroll bar too.
        if let Some(output) = cli.control(ID_OUTPUT) {
            let _ = SetWindowTheme(output, w!("DarkMode_Explorer"), None);
        }
        let _ = SetWindowPos(
            cli.hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let _ = windows::Win32::Graphics::Gdi::RedrawWindow(
            Some(cli.hwnd),
            None,
            None,
            windows::Win32::Graphics::Gdi::RDW_INVALIDATE
                | windows::Win32::Graphics::Gdi::RDW_ERASE
                | windows::Win32::Graphics::Gdi::RDW_FRAME
                | windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN,
        );
    }
}

/// Enter in the input line: the command goes to the island, the line empties.
fn submit(input: HWND) {
    let text = window_text(input);
    let cmd = text.trim();
    let Some(owner) = with_cli(|c| {
        c.cursor.reset();
        if !cmd.is_empty() {
            c.history.push(cmd);
        }
        c.owner
    }) else {
        return;
    };
    unsafe {
        let _ = SetWindowTextW(input, &HSTRING::new());
    }
    if !cmd.is_empty() {
        post(owner, CliAction::Run(cmd.to_owned()));
    }
}

/// Up (`older`) or Down through the history into the input line.
fn browse(input: HWND, older: bool) {
    let typed = window_text(input);
    let next = with_cli(|c| {
        let items = c.history.items().to_vec();
        if older {
            c.cursor.up(&items, &typed)
        } else {
            c.cursor.down(&items)
        }
    })
    .flatten();
    if let Some(text) = next {
        unsafe {
            let _ = SetWindowTextW(input, &HSTRING::from(text.as_str()));
        }
        let end = text.encode_utf16().count();
        send(input, EM_SETSEL, end, end as isize);
    }
}

fn has_selection(edit: HWND) -> bool {
    let (mut start, mut end) = (0u32, 0u32);
    send(
        edit,
        EM_GETSEL,
        &mut start as *mut u32 as usize,
        &mut end as *mut u32 as isize,
    );
    start != end
}

fn on_command(hwnd: HWND, id: i32, code: u32) {
    if id == ID_STOP && code == BN_CLICKED {
        if let Some(owner) = with_cli(|c| c.owner) {
            post(owner, CliAction::Stop);
        }
        focus_input(hwnd);
    }
}

fn color_control(wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hdc = HDC(wparam.0 as *mut c_void);
    let control = HWND(lparam.0 as *mut c_void);
    let p = theme::current_window();
    let id = unsafe { GetDlgCtrlID(control) };
    let (text, bg, brush) = match id {
        ID_OUTPUT | ID_INPUT => (TERMINAL_TEXT, TERMINAL_BG, terminal_brush()),
        ID_PROMPT => (p.accent, TERMINAL_BG, terminal_brush()),
        ID_STATUS => (p.secondary, p.bg, brushes(&p).0),
        _ => (p.text, p.bg, brushes(&p).0),
    };
    unsafe {
        SetTextColor(hdc, colorref(text));
        SetBkColor(hdc, colorref(bg));
    }
    LRESULT(brush.0 as isize)
}

/// The window background, the rounded terminal behind the output and input,
/// and the rule between them.
fn paint(hwnd: HWND) {
    let p = theme::current_window();
    let scale = with_cli(|c| c.scale).unwrap_or(1.0);
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        if !hdc.is_invalid() && GetClientRect(hwnd, &mut rc).is_ok() {
            FillRect(hdc, &rc, brushes(&p).0);
            let l = layout(
                (rc.right - rc.left) as f32 / scale,
                (rc.bottom - rc.top) as f32 / scale,
            );
            let px = |v: f32| (v * scale).round() as i32;
            let [x, y, w, h] = l.terminal;
            // A hairline in light mode, where the dark block needs no edge
            // but reads as part of the window with one.
            let edge = if p.light {
                blend(p.bg, p.border >> 8, (p.border & 0xFF) as f32 / 255.0)
            } else {
                TERMINAL_BG
            };
            let brush = CreateSolidBrush(colorref(TERMINAL_BG));
            let pen = CreatePen(PS_SOLID, 1, colorref(edge));
            let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
            let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
            let radius = px(16.0);
            let _ = RoundRect(hdc, px(x), px(y), px(x + w), px(y + h), radius, radius);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(HGDIOBJ(pen.0));
            let _ = DeleteObject(HGDIOBJ(brush.0));
            let rule = CreateSolidBrush(colorref(TERMINAL_LINE));
            let line = RECT {
                left: px(x + 8.0),
                top: px(l.rule_y),
                right: px(x + w - 8.0),
                bottom: px(l.rule_y) + 1,
            };
            FillRect(hdc, &line, rule);
            let _ = DeleteObject(HGDIOBJ(rule.0));
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn on_dpi_changed(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let dpi = (wparam.0 & 0xFFFF) as f32;
    let scale = if dpi > 0.0 { dpi / 96.0 } else { 1.0 };
    with_cli(|c| c.set_scale(scale));
    let suggested = lparam.0 as *const RECT;
    if let Some(rc) = unsafe { suggested.as_ref() } {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    with_cli(|c| c.layout());
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => color_control(wparam, lparam),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            on_command(hwnd, id, code);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            let active = (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE;
            let owner = with_cli(|c| (!c.finished).then_some(c.owner)).flatten();
            if let Some(owner) = owner {
                post(owner, CliAction::Focus(active));
            }
            if active {
                focus_input(hwnd);
            }
            result
        }
        WM_CLOSE => {
            finish(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            with_cli(|c| c.layout());
            LRESULT(0)
        }
        WM_DPICHANGED => {
            on_dpi_changed(hwnd, wparam, lparam);
            LRESULT(0)
        }
        WM_DESTROY => {
            // Gone without `finish` (its owner was destroyed): still say so.
            let owner =
                with_cli(|c| (!std::mem::replace(&mut c.finished, true)).then_some(c.owner))
                    .flatten();
            if let Some(owner) = owner {
                post(owner, CliAction::Closed);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // The fonts go once the controls are gone.
            CLI.with(|cell| {
                if let Ok(mut slot) = cell.try_borrow_mut()
                    && slot.as_ref().is_some_and(|c| c.hwnd == hwnd)
                {
                    *slot = None;
                }
            });
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Keys of the input line and the output (`data` is the control id).
unsafe extern "system" fn control_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let id = data as i32;
    let window = with_cli(|c| c.hwnd);
    match msg {
        WM_KEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as u16;
            let ctrl = key_down(VK_CONTROL.0);
            if key == VK_ESCAPE.0 {
                if let Some(window) = window {
                    unsafe {
                        let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(0);
            }
            if key == VK_A.0 && ctrl {
                send(hwnd, EM_SETSEL, 0, -1);
                return LRESULT(0);
            }
            if id == ID_INPUT {
                match key {
                    k if k == VK_RETURN.0 => {
                        submit(hwnd);
                        return LRESULT(0);
                    }
                    k if k == VK_UP.0 || k == VK_DOWN.0 => {
                        browse(hwnd, k == VK_UP.0);
                        return LRESULT(0);
                    }
                    k if k == VK_PRIOR.0 || k == VK_NEXT.0 => {
                        let output = with_cli(|c| c.control(ID_OUTPUT)).flatten();
                        if let Some(output) = output {
                            let page = if k == VK_PRIOR.0 {
                                SB_PAGEUP
                            } else {
                                SB_PAGEDOWN
                            };
                            send(output, WM_VSCROLL, page.0 as usize, 0);
                        }
                        return LRESULT(0);
                    }
                    k if k == VK_C.0 && ctrl && !has_selection(hwnd) => {
                        let stop = with_cli(|c| c.running.then_some(c.owner)).flatten();
                        if let Some(owner) = stop {
                            post(owner, CliAction::Stop);
                        }
                        return LRESULT(0);
                    }
                    k if k == VK_TAB.0 => return LRESULT(0),
                    _ => {}
                }
            }
        }
        WM_CHAR => {
            let ch = wparam.0;
            // Enter, Esc, Tab, Ctrl+A and Ctrl+C (handled above) must not beep.
            if matches!(ch, 0x0D | 0x1B | 0x09 | 0x01)
                || (ch == 0x03 && id == ID_INPUT && !has_selection(hwnd))
            {
                return LRESULT(0);
            }
            // Typing in the output goes to the input line.
            if id == ID_OUTPUT && ch >= 0x20 {
                let input = with_cli(|c| c.control(ID_INPUT)).flatten();
                if let Some(input) = input {
                    unsafe {
                        let _ = SetFocus(Some(input));
                    }
                    send(input, WM_CHAR, wparam.0, lparam.0);
                    return LRESULT(0);
                }
            }
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(control_proc), SUBCLASS_ID);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// `--demo-cli` (debug builds only): the CLI window on its own, under where
/// the panel would be, with a hidden owner that runs what is typed for real
/// in the user's home folder (builtins included) and prints each action.
/// `DI_WINDOW=dark|light|system` picks the window mode, `DI_SHELL=cmd` the
/// shell, and `DI_CLI_RUN=command` enters that command at once. Ends when
/// the window closes; a running command is stopped then.
#[cfg(debug_assertions)]
pub fn run_demo() {
    use std::path::PathBuf;
    use std::sync::Arc;

    use windows::Win32::Graphics::Gdi::MonitorFromPoint;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage,
    };

    use crate::cli::{self, Builtin, CliEvent, RunHandle, Session, SyncStep, WindowSync};
    use crate::theme::{Theme, WindowTheme};

    const WM_DEMO_EVENT: u32 = WM_APP + 12;

    struct Demo {
        session: Session,
        shown: WindowSync,
        run: Option<RunHandle>,
        runs: u64,
        cwd: PathBuf,
        shell: Shell,
    }

    thread_local! {
        static DEMO: RefCell<Option<Demo>> = const { RefCell::new(None) };
    }

    fn sync(d: &mut Demo) {
        match d.shown.next(&d.session.output) {
            SyncStep::Nothing => {}
            SyncStep::Append(lines) => append(&lines),
            SyncStep::Reset => reset(&d.session.output.text()),
        }
        set_status(
            &d.cwd.to_string_lossy(),
            d.shell,
            d.session.running().is_some(),
        );
    }

    fn run_command(owner: HWND, d: &mut Demo, cmd: &str) {
        match cli::builtin(cmd) {
            Some(Builtin::Clear) => {
                d.session.clear();
                d.shown.reset(&d.session.output);
                reset("");
            }
            Some(Builtin::Cd(arg)) => {
                d.session.begin(cmd);
                match cli::resolve_cd(&d.cwd, arg.as_deref(), &cli::home()) {
                    Ok(dir) => d.cwd = dir,
                    Err(arg) => d.session.note(cli::cd_error(&arg)),
                }
            }
            None if d.session.running().is_some() => {
                d.session.output.push(cli::echo_line(cmd));
                d.session.output.push(tr(Key::CliBusy).into());
            }
            None => {
                d.session.begin(cmd);
                d.runs += 1;
                let raw = owner.0 as isize;
                let sink: cli::Sink = Arc::new(move |event: CliEvent| {
                    let ptr = Box::into_raw(Box::new(event));
                    let owner = HWND(raw as *mut c_void);
                    let posted = unsafe {
                        PostMessageW(Some(owner), WM_DEMO_EVENT, WPARAM(0), LPARAM(ptr as isize))
                    };
                    if posted.is_err() {
                        drop(unsafe { Box::from_raw(ptr) });
                    }
                });
                match cli::spawn(d.runs, cmd, &d.cwd, d.shell, sink) {
                    Ok(run) => {
                        d.session.started(d.runs);
                        d.run = Some(run);
                    }
                    Err(e) => d.session.note(e.to_string()),
                }
            }
        }
        sync(d);
    }

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_APP_CLI_ACTION => {
                let Some(action) = (unsafe { take_action(lparam) }) else {
                    return LRESULT(0);
                };
                println!("demo-cli: {action:?}");
                DEMO.with_borrow_mut(|slot| {
                    let Some(d) = slot.as_mut() else { return };
                    match action {
                        CliAction::Run(cmd) => run_command(hwnd, d, &cmd),
                        CliAction::Stop => {
                            if let Some(run) = &d.run {
                                run.kill();
                            }
                        }
                        CliAction::Focus(_) => {}
                        CliAction::Closed => {
                            if let Some(run) = d.run.take() {
                                run.kill();
                            }
                            unsafe { PostQuitMessage(0) };
                        }
                    }
                });
                LRESULT(0)
            }
            WM_DEMO_EVENT => {
                let ptr = lparam.0 as *mut CliEvent;
                if ptr.is_null() {
                    return LRESULT(0);
                }
                let event = *unsafe { Box::from_raw(ptr) };
                DEMO.with_borrow_mut(|slot| {
                    let Some(d) = slot.as_mut() else { return };
                    match event {
                        CliEvent::Output { run, lines } => {
                            d.session.lines(run, lines);
                        }
                        CliEvent::Exit { run, code, killed } => {
                            if d.session.exit(run, code, killed) {
                                d.run = None;
                            }
                        }
                    }
                    sync(d);
                });
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    let mode = std::env::var("DI_WINDOW").unwrap_or_default();
    theme::set_system_light(theme::read_system_light());
    theme::set_current(Theme {
        window: WindowTheme::from_name(&mode).unwrap_or_default(),
        ..Theme::default()
    });
    let shell = std::env::var("DI_SHELL")
        .ok()
        .and_then(|s| Shell::from_name(&s))
        .unwrap_or_default();
    let cwd = cli::home();
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let instance: HINSTANCE = module.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(owner_proc),
            hInstance: instance,
            lpszClassName: w!("DynamicIslandCliDemoOwner"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let Ok(owner) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("DynamicIslandCliDemoOwner"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            eprintln!("demo-cli: cannot create the owner window");
            return;
        };
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (scale, _) = monitor_at(POINT {
            x: info.rcMonitor.left,
            y: info.rcMonitor.top,
        });
        let anchor = POINT {
            x: (info.rcMonitor.left + info.rcMonitor.right) / 2,
            y: info.rcMonitor.top + (super::geometry::WINDOW_H * scale).round() as i32,
        };
        DEMO.with_borrow_mut(|slot| {
            *slot = Some(Demo {
                session: Session::default(),
                shown: WindowSync::default(),
                run: None,
                runs: 0,
                cwd: cwd.clone(),
                shell,
            })
        });
        let init = CliInit {
            text: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            shell,
            running: false,
            history: History::default(),
        };
        match open(owner, anchor, init) {
            Ok(hwnd) => println!("demo-cli: hwnd {:#x}", hwnd.0 as usize),
            Err(error) => {
                eprintln!("demo-cli: open failed: {error}");
                return;
            }
        }
        if let Ok(cmd) = std::env::var("DI_CLI_RUN") {
            post(owner, CliAction::Run(cmd));
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyWindow(owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
        let [x, y, w, h] = inner;
        let [ox, oy, ow, oh] = outer;
        x >= ox && y >= oy && x + w <= ox + ow && y + h <= oy + oh
    }

    fn disjoint(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1]
    }

    #[test]
    fn terminal_holds_output_and_input_above_status_and_stop() {
        // The client area of the 560 × 320 window, and a larger one.
        for (cw, ch) in [(544.0, 281.0), (800.0, 500.0)] {
            let l = layout(cw, ch);
            let client = [0.0, 0.0, cw, ch];
            for r in [l.terminal, l.status, l.stop] {
                assert!(inside(r, client), "{r:?}");
            }
            for r in [l.output, l.prompt, l.input] {
                assert!(inside(r, l.terminal), "{r:?} in the terminal");
            }
            assert!(l.output[1] + l.output[3] <= l.rule_y);
            assert!(l.rule_y < l.input[1]);
            assert!(
                l.prompt[0] + l.prompt[2] <= l.input[0],
                "prompt left of input"
            );
            assert_eq!(l.prompt[1], l.input[1]);
            assert!(disjoint(l.status, l.stop));
            assert!(l.terminal[1] + l.terminal[3] <= l.status[1]);
            assert!(l.output[3] >= 150.0, "room for the output: {}", l.output[3]);
        }
    }

    #[test]
    fn status_names_shell_and_folder() {
        assert_eq!(
            status_text(Shell::PowerShell, "C:\\Users\\x"),
            "PowerShell \u{00B7} C:\\Users\\x"
        );
        assert_eq!(status_text(Shell::Cmd, "D:\\"), "cmd \u{00B7} D:\\");
    }

    #[test]
    fn actions_round_trip_through_lparam() {
        let action = CliAction::Run("dir".into());
        let lparam = LPARAM(Box::into_raw(Box::new(action.clone())) as isize);
        assert_eq!(unsafe { take_action(lparam) }, Some(action));
        assert_eq!(unsafe { take_action(LPARAM(0)) }, None);
    }
}
