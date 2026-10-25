// Not referenced by the app yet: wired in Task 10.
#![allow(dead_code)]

//! The note and event editors: small dark Win32 windows under the panel,
//! because the layered island cannot host native text controls. At most one
//! editor exists at a time; it lives on the island's UI thread and reports to
//! the island with `WM_APP_EDITOR_RESULT`.
//!
//! Every editor's last result is `NoteSaved`, `NoteDeleted` or `Closed`
//! (`EditorResult::closes_editor`), posted as the window goes away.
//! `EventSave` is not last: the event editor waits for `WM_APP_EDITOR_STATUS`.
//!
//! Keyboard handling lives in a subclass on each control (Esc, Ctrl+Enter,
//! Tab), so the editors need no `IsDialogMessage` in the island's loop.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::Once;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE,
    SYSTEMTIME, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    CreateSolidBrush, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint,
    FONT_QUALITY, FW_NORMAL, GdiFlush, GetMonitorInfoW, HBRUSH, HDC, HFONT, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SRCCOPY, SelectObject, SetBkColor, SetTextColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, DATETIMEPICK_CLASSW, DTM_GETMONTHCAL, DTM_GETSYSTEMTIME, DTM_SETFORMATW,
    DTM_SETMCFONT, DTM_SETSYSTEMTIME, DTN_DROPDOWN, DTS_SHORTDATEFORMAT, DTS_TIMEFORMAT,
    EM_SETMARGINS, EM_SETSEL, GDT_VALID, ICC_DATE_CLASSES, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR, SetWindowTheme,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetFocus, VK_A, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON,
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, EC_LEFTMARGIN, EC_RIGHTMARGIN,
    ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, GW_CHILD,
    GetClientRect, GetDlgCtrlID, GetForegroundWindow, GetNextDlgTabItem, GetParent, GetWindow,
    GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, IDCANCEL, IDOK, IsChild, IsWindow,
    LoadCursorW, MoveWindow, PRF_CLIENT, PRF_ERASEBKGND, PostMessageW, RegisterClassExW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowPos, SetWindowTextW,
    ShowWindow, SwitchToThisWindow, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE,
    WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_NCDESTROY, WM_NOTIFY, WM_PAINT,
    WM_PRINTCLIENT, WM_SETFOCUS, WM_SETFONT, WM_SIZE, WNDCLASSEXW, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

use crate::calendar::EventDraft;
/// Sent to the event editor after a save (defined by the sync thread, which
/// posts it): `wparam` 1 = saved, the editor closes; 0 = failed, `lparam` is a
/// `Box<String>` raw pointer (or 0) the editor frees and shows in red.
pub use crate::calendar::sync::WM_APP_EDITOR_STATUS;

/// Posted by an editor to its owner: `lparam` is a `Box<EditorResult>` raw
/// pointer the owner frees (`take_result`). WM_APP + 1..5 are taken.
pub const WM_APP_EDITOR_RESULT: u32 = WM_APP + 6;
/// Posted by the note editor to itself on deactivation: it must not destroy
/// itself inside `WM_ACTIVATE`. Only editor windows receive it.
const WM_APP_EDITOR_CLOSE: u32 = WM_APP + 7;

#[derive(Clone, Debug, PartialEq)]
pub enum EditorKind {
    /// `id` is `None` for a new note.
    Note { id: Option<String>, text: String },
    /// `event_id` is `None` for a new event. `read_only` (calendar not
    /// writable, or the event does not fit a draft) shows the fields
    /// disabled and only "Tutup".
    Event {
        event_id: Option<String>,
        calendar_id: Option<String>,
        draft: EventDraft,
        read_only: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorResult {
    /// Empty text means "delete" to the notes store.
    NoteSaved {
        id: Option<String>,
        text: String,
    },
    NoteDeleted {
        id: String,
    },
    /// The editor stays open, showing "Menyimpan…", until `WM_APP_EDITOR_STATUS`.
    EventSave {
        event_id: Option<String>,
        calendar_id: Option<String>,
        draft: EventDraft,
    },
    Closed,
}

impl EditorResult {
    /// True for the last result an editor posts: the window is gone by the
    /// time the owner reads it.
    pub fn closes_editor(&self) -> bool {
        !matches!(self, EditorResult::EventSave { .. })
    }
}

/// Takes ownership of a `WM_APP_EDITOR_RESULT`'s `lparam`.
///
/// # Safety
/// `lparam` must come from a `WM_APP_EDITOR_RESULT` message, and each message
/// may be taken only once.
pub unsafe fn take_result(lparam: LPARAM) -> Option<EditorResult> {
    let ptr = lparam.0 as *mut EditorResult;
    (!ptr.is_null()).then(|| *unsafe { Box::from_raw(ptr) })
}

/// Logical sizes (the window's outer size, scaled by the monitor's DPI).
const NOTE_SIZE: (f32, f32) = (400.0, 220.0);
const EVENT_SIZE: (f32, f32) = (420.0, 260.0);
/// Gap between the anchor (the panel's bottom edge) and the editor, in px.
const GAP: i32 = 8;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}
const BG: COLORREF = rgb(0x20, 0x20, 0x22);
/// Text fields sit a step lighter than the window so they read as fields.
const FIELD: COLORREF = rgb(0x2B, 0x2B, 0x2E);
const TEXT: COLORREF = rgb(0xEE, 0xEE, 0xEE);
const ERROR_TEXT: COLORREF = rgb(0xFF, 0x6B, 0x6B);

const STR_SAVING: &str = "Menyimpan…";
const STR_SAVE_FAILED: &str = "Gagal simpan";

/// Centered under `anchor` with a `GAP`, then moved inside `work` (or pinned
/// to its top-left corner when it is bigger than `work`).
pub fn editor_rect(anchor: POINT, size: SIZE, work: RECT) -> RECT {
    let clamp = |pos: i32, len: i32, lo: i32, hi: i32| pos.min(hi - len).max(lo);
    let left = clamp(anchor.x - size.cx / 2, size.cx, work.left, work.right);
    let top = clamp(anchor.y + GAP, size.cy, work.top, work.bottom);
    RECT {
        left,
        top,
        right: left + size.cx,
        bottom: top + size.cy,
    }
}

/// The checks behind "Simpan": a title, and for a timed event an end after
/// its start (Google also allows zero-length events; the island does not).
pub fn validate_event(d: &EventDraft) -> std::result::Result<(), &'static str> {
    if d.title.trim().is_empty() {
        return Err("Judul wajib diisi");
    }
    if !d.all_day && d.end <= d.start {
        return Err("Jam selesai harus setelah jam mulai");
    }
    Ok(())
}

/// Text for an EDIT control, which needs `\r\n` line breaks.
fn to_edit_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Text from an EDIT control, with `\n` line breaks.
fn from_edit_text(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn system_time(date: NaiveDate, time: NaiveTime) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: date.year().clamp(1601, 30827) as u16,
        wMonth: date.month() as u16,
        wDayOfWeek: date.weekday().num_days_from_sunday() as u16,
        wDay: date.day() as u16,
        wHour: time.hour() as u16,
        wMinute: time.minute() as u16,
        wSecond: 0,
        wMilliseconds: 0,
    }
}

fn date_of(st: &SYSTEMTIME) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(st.wYear.into(), st.wMonth.into(), st.wDay.into())
}

/// Minutes only: the pickers show `HH:mm`.
fn time_of(st: &SYSTEMTIME) -> Option<NaiveTime> {
    NaiveTime::from_hms_opt(st.wHour.into(), st.wMinute.into(), 0)
}

// Control ids. IDOK / IDCANCEL double as the keyboard commands (Ctrl+Enter, Esc).
const ID_OK: i32 = IDOK.0;
const ID_CANCEL: i32 = IDCANCEL.0;
const ID_TEXT: i32 = 101;
const ID_DELETE: i32 = 102;
const ID_TITLE: i32 = 201;
const ID_DATE: i32 = 202;
const ID_ALLDAY: i32 = 203;
const ID_ALLDAY_LABEL: i32 = 204;
const ID_START: i32 = 205;
const ID_END: i32 = 206;
const ID_NOTES: i32 = 207;
const ID_STATUS: i32 = 208;
const ID_LBL_TITLE: i32 = 301;
const ID_LBL_DATE: i32 = 302;
const ID_LBL_START: i32 = 303;
const ID_LBL_END: i32 = 304;
const ID_LBL_NOTES: i32 = 305;

// Static styles (System_SystemServices in `windows`; values fixed by Win32).
const SS_NOTIFY: u32 = 0x0100;
const SS_CENTERIMAGE: u32 = 0x0200;
const SS_ENDELLIPSIS: u32 = 0x4000;

/// What the keyboard subclass does with a control (its `dwRefData`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Multi-line EDIT: Enter is a newline, Ctrl+Enter saves.
    Multi = 1,
    /// Single-line EDIT: Enter saves.
    Line = 2,
    /// Push button or checkbox: Enter clicks it.
    Button = 3,
    /// Date/time picker: Enter saves; Esc closes its drop-down first.
    /// Painted dark by `paint_dark`.
    Picker = 4,
    /// A picker's spin buttons or drop-down calendar: only painted dark.
    Paint = 5,
}

impl Role {
    fn from_ref(data: usize) -> Option<Role> {
        [
            Role::Multi,
            Role::Line,
            Role::Button,
            Role::Picker,
            Role::Paint,
        ]
        .into_iter()
        .find(|r| *r as usize == data)
    }
}

/// One child control to create.
struct Spec {
    id: i32,
    class: PCWSTR,
    text: String,
    style: u32,
    role: Option<Role>,
}

fn specs(kind: &EditorKind) -> Vec<Spec> {
    let spec = |id, class, text: &str, style: u32, role| Spec {
        id,
        class,
        text: text.to_owned(),
        style,
        role,
    };
    let multi = (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32 | WS_VSCROLL.0;
    let label = |id, text| spec(id, w!("STATIC"), text, SS_CENTERIMAGE, None);
    match kind {
        EditorKind::Note { id, text } => {
            let mut v = vec![spec(
                ID_TEXT,
                w!("EDIT"),
                &to_edit_text(text),
                multi,
                Some(Role::Multi),
            )];
            if id.is_some() {
                let style = BS_PUSHBUTTON as u32;
                v.push(spec(
                    ID_DELETE,
                    w!("BUTTON"),
                    "Hapus",
                    style,
                    Some(Role::Button),
                ));
            }
            v
        }
        EditorKind::Event {
            draft, read_only, ..
        } => {
            let ro = if *read_only { ES_READONLY as u32 } else { 0 };
            let mut v = vec![
                label(ID_LBL_TITLE, "Judul"),
                spec(
                    ID_TITLE,
                    w!("EDIT"),
                    &draft.title,
                    ES_AUTOHSCROLL as u32 | ro,
                    Some(Role::Line),
                ),
                label(ID_LBL_DATE, "Tanggal"),
                spec(
                    ID_DATE,
                    DATETIMEPICK_CLASSW,
                    "",
                    DTS_SHORTDATEFORMAT,
                    Some(Role::Picker),
                ),
                spec(
                    ID_ALLDAY,
                    w!("BUTTON"),
                    "",
                    BS_AUTOCHECKBOX as u32,
                    Some(Role::Button),
                ),
                spec(
                    ID_ALLDAY_LABEL,
                    w!("STATIC"),
                    "Seharian",
                    SS_CENTERIMAGE | SS_NOTIFY,
                    None,
                ),
                label(ID_LBL_START, "Mulai"),
                spec(
                    ID_START,
                    DATETIMEPICK_CLASSW,
                    "",
                    DTS_TIMEFORMAT,
                    Some(Role::Picker),
                ),
                label(ID_LBL_END, "Selesai"),
                spec(
                    ID_END,
                    DATETIMEPICK_CLASSW,
                    "",
                    DTS_TIMEFORMAT,
                    Some(Role::Picker),
                ),
                label(ID_LBL_NOTES, "Catatan"),
                spec(
                    ID_NOTES,
                    w!("EDIT"),
                    &to_edit_text(&draft.description),
                    multi | ro,
                    Some(Role::Multi),
                ),
                spec(
                    ID_STATUS,
                    w!("STATIC"),
                    "",
                    SS_CENTERIMAGE | SS_ENDELLIPSIS,
                    None,
                ),
            ];
            if *read_only {
                let style = BS_DEFPUSHBUTTON as u32;
                v.push(spec(
                    ID_CANCEL,
                    w!("BUTTON"),
                    "Tutup",
                    style,
                    Some(Role::Button),
                ));
            } else {
                let def = BS_DEFPUSHBUTTON as u32;
                let push = BS_PUSHBUTTON as u32;
                v.push(spec(ID_OK, w!("BUTTON"), "Simpan", def, Some(Role::Button)));
                v.push(spec(
                    ID_CANCEL,
                    w!("BUTTON"),
                    "Batal",
                    push,
                    Some(Role::Button),
                ));
            }
            v
        }
    }
}

/// Logical `[x, y, w, h]` of each control for a client area of `cw` × `ch`
/// logical px.
fn layout(note: bool, ids: &[i32], cw: f32, ch: f32) -> Vec<(i32, [f32; 4])> {
    let has = |id| ids.contains(&id);
    let mut out = Vec::new();
    if note {
        let m = 10.0;
        let row = if has(ID_DELETE) { 28.0 + 8.0 } else { 0.0 };
        out.push((ID_TEXT, [m, m, cw - 2.0 * m, ch - 2.0 * m - row]));
        out.push((ID_DELETE, [cw - m - 80.0, ch - m - 28.0, 80.0, 28.0]));
        return out;
    }
    let (m, label_w, row_h) = (12.0, 56.0, 24.0);
    let fx = m + label_w + 8.0;
    let field_w = cw - fx - m;
    let rows = [12.0, 44.0, 76.0, 108.0];
    for (id, y) in [ID_LBL_TITLE, ID_LBL_DATE, ID_LBL_START, ID_LBL_NOTES]
        .into_iter()
        .zip(rows)
    {
        out.push((id, [m, y, label_w, row_h]));
    }
    out.push((ID_TITLE, [fx, rows[0], field_w, row_h]));
    out.push((ID_DATE, [fx, rows[1], 150.0, row_h]));
    out.push((ID_ALLDAY, [fx + 166.0, rows[1] + 3.0, 18.0, 18.0]));
    out.push((ID_ALLDAY_LABEL, [fx + 188.0, rows[1], 80.0, row_h]));
    out.push((ID_START, [fx, rows[2], 80.0, row_h]));
    out.push((ID_LBL_END, [fx + 96.0, rows[2], 52.0, row_h]));
    out.push((ID_END, [fx + 150.0, rows[2], 80.0, row_h]));
    let (btn_w, btn_h) = (80.0, 28.0);
    let by = ch - m - btn_h;
    let right = cw - m - btn_w;
    out.push((ID_CANCEL, [right, by, btn_w, btn_h]));
    let left_of_buttons = if has(ID_OK) {
        out.push((ID_OK, [right - 8.0 - btn_w, by, btn_w, btn_h]));
        right - 8.0 - btn_w
    } else {
        right
    };
    out.push((ID_STATUS, [m, by, left_of_buttons - 8.0 - m, btn_h]));
    out.push((
        ID_NOTES,
        [fx, rows[3], field_w, (by - 8.0 - rows[3]).max(24.0)],
    ));
    out
}

enum Mode {
    Note {
        id: Option<String>,
    },
    Event {
        event_id: Option<String>,
        calendar_id: Option<String>,
        /// What the fields started from; fallback if a picker cannot be read.
        base: EventDraft,
        read_only: bool,
        /// An `EventSave` was posted; Simpan stays off until the status arrives.
        saving: bool,
    },
}

struct Editor {
    hwnd: HWND,
    owner: HWND,
    /// Foreground window before the editor opened; gets the focus back.
    previous: HWND,
    mode: Mode,
    scale: f32,
    fonts: Fonts,
    controls: Vec<(i32, HWND)>,
    /// Set once the window was active: only then does deactivation close a note.
    was_active: bool,
    /// The final result was posted.
    finished: bool,
}

/// Segoe UI at the editor's DPI, deleted on drop (after the controls are gone).
struct Fonts {
    /// ClearType, for everything that paints itself dark.
    text: HFONT,
    /// Grayscale anti-aliasing for the pickers: `darken` maps gray shades
    /// cleanly but would leave ClearType's colored fringes as they are.
    gray: HFONT,
}

impl Fonts {
    fn new(scale: f32) -> Self {
        Fonts {
            text: create_font(scale, CLEARTYPE_QUALITY),
            gray: create_font(scale, ANTIALIASED_QUALITY),
        }
    }

    fn apply(&self, control: HWND, id: i32) {
        let font = if matches!(id, ID_DATE | ID_START | ID_END) {
            self.gray
        } else {
            self.text
        };
        send(control, WM_SETFONT, font.0 as usize, 1);
        if id == ID_DATE {
            // The drop-down calendar, also painted by `paint_dark`.
            send(control, DTM_SETMCFONT, self.gray.0 as usize, 1);
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in [self.text, self.gray] {
            if !font.is_invalid() {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
            }
        }
    }
}

thread_local! {
    static EDITOR: RefCell<Option<Editor>> = const { RefCell::new(None) };
    /// The status label and whether it shows an error, for `WM_CTLCOLORSTATIC`
    /// (read without borrowing `EDITOR`, which may be held up the stack).
    static STATUS_LABEL: Cell<(isize, bool)> = const { Cell::new((0, false)) };
    /// The control that last had the keyboard focus, restored on reactivation.
    static LAST_FOCUS: Cell<isize> = const { Cell::new(0) };
    static BRUSHES: Cell<Option<(HBRUSH, HBRUSH)>> = const { Cell::new(None) };
}

/// `(window, field)` brushes, created once and kept for the process lifetime.
fn brushes() -> (HBRUSH, HBRUSH) {
    BRUSHES.with(|b| {
        b.get().unwrap_or_else(|| {
            let made = unsafe { (CreateSolidBrush(BG), CreateSolidBrush(FIELD)) };
            b.set(Some(made));
            made
        })
    })
}

/// Runs `f` on the editor for `hwnd`, or returns `None` when there is none or
/// a handler further up the stack holds it.
fn with_editor<R>(hwnd: HWND, f: impl FnOnce(&mut Editor) -> R) -> Option<R> {
    EDITOR.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        slot.as_mut().filter(|ed| ed.hwnd == hwnd).map(f)
    })
}

/// Removes the editor for `hwnd` from the slot.
fn take_editor(hwnd: HWND) -> Option<Editor> {
    EDITOR.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        if slot.as_ref().is_some_and(|ed| ed.hwnd == hwnd) {
            slot.take()
        } else {
            None
        }
    })
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

fn window_text(hwnd: HWND) -> String {
    let len = usize::try_from(unsafe { GetWindowTextLengthW(hwnd) }).unwrap_or(0);
    let mut buf = vec![0u16; len + 1];
    let copied = usize::try_from(unsafe { GetWindowTextW(hwnd, &mut buf) }).unwrap_or(0);
    String::from_utf16_lossy(buf.get(..copied).unwrap_or(&[]))
}

fn post_result(owner: HWND, result: EditorResult) -> bool {
    let ptr = Box::into_raw(Box::new(result));
    let posted = unsafe {
        PostMessageW(
            Some(owner),
            WM_APP_EDITOR_RESULT,
            WPARAM(0),
            LPARAM(ptr as isize),
        )
    };
    if posted.is_err() {
        // The owner never sees it; nobody else will free it.
        drop(unsafe { Box::from_raw(ptr) });
        return false;
    }
    true
}

fn init_common_controls() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_DATE_CLASSES | ICC_STANDARD_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&icc);
        }
    });
}

const CLASS_NAME: PCWSTR = w!("DynamicIslandEditor");

fn register_class(instance: HINSTANCE) -> Result<()> {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: brushes().0,
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

/// DPI scale and work area of the monitor under `pt`.
fn monitor_at(pt: POINT) -> (f32, RECT) {
    unsafe {
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = 96;
        }
        (dpi_x as f32 / 96.0, info.rcWork)
    }
}

fn create_font(scale: f32, quality: FONT_QUALITY) -> HFONT {
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
            quality,
            0,
            w!("Segoe UI"),
        )
    }
}

/// Opens an editor owned by `owner` (the island), centered `GAP` px below
/// `anchor_bottom_center` (screen px) and kept on that monitor's work area.
/// If an editor is already open, focuses it and returns its handle instead.
pub fn open_editor(owner: HWND, anchor_bottom_center: POINT, kind: EditorKind) -> Result<HWND> {
    let existing = EDITOR.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|slot| slot.as_ref().map(|ed| ed.hwnd))
    });
    if let Some(hwnd) = existing {
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            bring_to_front(hwnd);
            return Ok(hwnd);
        }
        // Stale: the window died without its destroy messages reaching us.
        EDITOR.with(|cell| {
            if let Ok(mut slot) = cell.try_borrow_mut() {
                *slot = None;
            }
        });
    }

    init_common_controls();
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    register_class(instance)?;

    let (scale, work) = monitor_at(anchor_bottom_center);
    let (lw, lh) = match kind {
        EditorKind::Note { .. } => NOTE_SIZE,
        EditorKind::Event { .. } => EVENT_SIZE,
    };
    let size = SIZE {
        cx: (lw * scale).round() as i32,
        cy: (lh * scale).round() as i32,
    };
    let rc = editor_rect(anchor_bottom_center, size, work);
    let title = match &kind {
        EditorKind::Note { .. } => "Note",
        EditorKind::Event {
            read_only: true, ..
        } => "Acara",
        EditorKind::Event { event_id: None, .. } => "Tambah acara",
        EditorKind::Event { .. } => "Edit acara",
    };

    let previous = unsafe { GetForegroundWindow() };
    let ex_style: WINDOW_EX_STYLE = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            CLASS_NAME,
            &HSTRING::from(title),
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
    let dark: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
    }

    let fonts = Fonts::new(scale);
    let controls = match create_controls(hwnd, instance, &kind, &fonts) {
        Ok(controls) => controls,
        Err(error) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
    };
    let mode = match kind {
        EditorKind::Note { id, .. } => Mode::Note { id },
        EditorKind::Event {
            event_id,
            calendar_id,
            draft,
            read_only,
        } => Mode::Event {
            event_id,
            calendar_id,
            base: draft,
            read_only,
            saving: false,
        },
    };
    let editor = Editor {
        hwnd,
        owner,
        previous,
        mode,
        scale,
        fonts,
        controls,
        was_active: false,
        finished: false,
    };
    editor.fill_pickers();
    editor.sync_enabled();
    editor.layout();
    STATUS_LABEL.set((editor.control(ID_STATUS).map_or(0, |h| h.0 as isize), false));
    let first = editor.control(ID_TEXT).or(editor.control(ID_TITLE));
    LAST_FOCUS.set(first.map_or(0, |h| h.0 as isize));
    let stored = EDITOR.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => {
            *slot = Some(editor);
            None
        }
        Err(_) => Some(editor),
    });
    if let Some(editor) = stored {
        // Called from inside an editor handler: an unmanaged window could never close.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        drop(editor);
        return Err(Error::empty());
    }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    bring_to_front(hwnd);
    if let Some(edit) = first {
        // Caret at the end of the existing text.
        let end = unsafe { GetWindowTextLengthW(edit) }.max(0) as usize;
        send(edit, EM_SETSEL, end, end as isize);
    }
    Ok(hwnd)
}

/// Makes `hwnd` the foreground window and focuses its last-focused control.
/// The island process received the click that opened the editor, so it may
/// set the foreground; `SwitchToThisWindow` is the fallback if Windows still
/// refuses (it flashes the window rather than failing silently).
fn bring_to_front(hwnd: HWND) {
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() != hwnd {
            SwitchToThisWindow(hwnd, true);
        }
    }
    focus_last(hwnd);
}

fn focus_last(hwnd: HWND) {
    let last = HWND(LAST_FOCUS.get() as *mut c_void);
    unsafe {
        if !last.is_invalid() && IsChild(hwnd, last).as_bool() {
            let _ = SetFocus(Some(last));
        }
    }
}

fn create_controls(
    parent: HWND,
    instance: HINSTANCE,
    kind: &EditorKind,
    fonts: &Fonts,
) -> Result<Vec<(i32, HWND)>> {
    let mut controls = Vec::new();
    for spec in specs(kind) {
        let mut style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(spec.style);
        if spec.role.is_some() {
            style |= WS_TABSTOP;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                spec.class,
                &HSTRING::from(spec.text.as_str()),
                style,
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(spec.id as isize as *mut c_void)),
                Some(instance),
                None,
            )?
        };
        fonts.apply(hwnd, spec.id);
        unsafe {
            // Dark push buttons, checkbox and scroll bars. The pickers ignore
            // dark themes and colors, so `paint_dark` recolors them.
            if spec.role.is_some() {
                let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), None);
            }
            if let Some(role) = spec.role {
                let _ = SetWindowSubclass(hwnd, Some(control_proc), SUBCLASS_ID, role as usize);
            }
            if spec.role == Some(Role::Picker) {
                // The time pickers' spin buttons are a child window.
                if let Ok(spin) = GetWindow(hwnd, GW_CHILD) {
                    let _ = SetWindowSubclass(
                        spin,
                        Some(control_proc),
                        SUBCLASS_ID,
                        Role::Paint as usize,
                    );
                }
            }
        }
        controls.push((spec.id, hwnd));
    }
    Ok(controls)
}

impl Editor {
    fn control(&self, id: i32) -> Option<HWND> {
        self.controls
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, h)| *h)
    }

    fn is_note(&self) -> bool {
        matches!(self.mode, Mode::Note { .. })
    }

    /// Positions every control for the current client size and scale.
    fn layout(&self) {
        let mut rc = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rc) }.is_err() {
            return;
        }
        let s = self.scale.max(0.5);
        let (cw, ch) = (
            (rc.right - rc.left) as f32 / s,
            (rc.bottom - rc.top) as f32 / s,
        );
        let ids: Vec<i32> = self.controls.iter().map(|(id, _)| *id).collect();
        for (id, [x, y, w, h]) in layout(self.is_note(), &ids, cw, ch) {
            if let Some(hwnd) = self.control(id) {
                let px = |v: f32| (v * s).round() as i32;
                unsafe {
                    let _ = MoveWindow(hwnd, px(x), px(y), px(w).max(0), px(h).max(0), true);
                }
            }
        }
        let margin = (6.0 * s).round() as usize;
        for id in [ID_TEXT, ID_TITLE, ID_NOTES] {
            if let Some(edit) = self.control(id) {
                send(
                    edit,
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                    (margin | margin << 16) as isize,
                );
            }
        }
    }

    /// Formats the pickers and loads the draft's date and times.
    fn fill_pickers(&self) {
        let Mode::Event { base, .. } = &self.mode else {
            return;
        };
        let set = |id, format: PCWSTR, time| {
            if let Some(picker) = self.control(id) {
                let st = system_time(base.date, time);
                send(picker, DTM_SETFORMATW, 0, format.0 as isize);
                send(
                    picker,
                    DTM_SETSYSTEMTIME,
                    GDT_VALID.0 as usize,
                    &st as *const SYSTEMTIME as isize,
                );
            }
        };
        set(ID_DATE, w!("ddd d MMM yyyy"), NaiveTime::MIN);
        set(ID_START, w!("HH':'mm"), base.start);
        set(ID_END, w!("HH':'mm"), base.end);
        if let Some(check) = self.control(ID_ALLDAY) {
            let state = if base.all_day { BST_CHECKED.0 } else { 0 };
            send(check, BM_SETCHECK, state as usize, 0);
        }
    }

    fn all_day_checked(&self) -> bool {
        self.control(ID_ALLDAY)
            .is_some_and(|c| send(c, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize)
    }

    /// Read-only events: everything off but Tutup. Otherwise the time pickers
    /// follow "Seharian" and Simpan is off while saving.
    fn sync_enabled(&self) {
        let Mode::Event {
            read_only, saving, ..
        } = self.mode
        else {
            return;
        };
        let enable = |id, on: bool| {
            if let Some(c) = self.control(id) {
                unsafe {
                    let _ = EnableWindow(c, on);
                }
            }
        };
        let timed = !self.all_day_checked();
        for id in [ID_DATE, ID_ALLDAY, ID_ALLDAY_LABEL] {
            enable(id, !read_only);
        }
        enable(ID_START, !read_only && timed);
        enable(ID_END, !read_only && timed);
        enable(ID_OK, !read_only && !saving);
    }

    fn picker_time(&self, id: i32) -> Option<SYSTEMTIME> {
        let picker = self.control(id)?;
        let mut st = SYSTEMTIME::default();
        let got = send(
            picker,
            DTM_GETSYSTEMTIME,
            0,
            &mut st as *mut SYSTEMTIME as isize,
        );
        (got == GDT_VALID.0 as isize).then_some(st)
    }

    fn read_draft(&self) -> Option<EventDraft> {
        let Mode::Event { base, .. } = &self.mode else {
            return None;
        };
        let text = |id| self.control(id).map(window_text).unwrap_or_default();
        let date = self.picker_time(ID_DATE).as_ref().and_then(date_of);
        let start = self.picker_time(ID_START).as_ref().and_then(time_of);
        let end = self.picker_time(ID_END).as_ref().and_then(time_of);
        Some(EventDraft {
            title: text(ID_TITLE).trim().to_owned(),
            date: date.unwrap_or(base.date),
            all_day: self.all_day_checked(),
            start: start.unwrap_or(base.start),
            end: end.unwrap_or(base.end),
            description: from_edit_text(&text(ID_NOTES)),
        })
    }

    fn set_status(&self, text: &str, error: bool) {
        let Some(label) = self.control(ID_STATUS) else {
            return;
        };
        STATUS_LABEL.set((label.0 as isize, error));
        unsafe {
            let _ = SetWindowTextW(label, &HSTRING::from(text));
        }
    }

    /// The result closing the window would post: the note's text, or `Closed`.
    fn closing_result(&self) -> EditorResult {
        match &self.mode {
            Mode::Note { id } => EditorResult::NoteSaved {
                id: id.clone(),
                text: self
                    .control(ID_TEXT)
                    .map(|c| from_edit_text(&window_text(c)))
                    .unwrap_or_default(),
            },
            Mode::Event { .. } => EditorResult::Closed,
        }
    }

    /// Simpan: validates, posts `EventSave` and waits for the status.
    fn save_event(&mut self) {
        let Some(draft) = self.read_draft() else {
            return;
        };
        let Mode::Event {
            event_id,
            calendar_id,
            read_only: false,
            saving: false,
            ..
        } = &self.mode
        else {
            return;
        };
        if let Err(message) = validate_event(&draft) {
            self.set_status(message, true);
            return;
        }
        let result = EditorResult::EventSave {
            event_id: event_id.clone(),
            calendar_id: calendar_id.clone(),
            draft,
        };
        if !post_result(self.owner, result) {
            self.set_status(STR_SAVE_FAILED, true);
            return;
        }
        if let Mode::Event { saving, .. } = &mut self.mode {
            *saving = true;
        }
        // Simpan is about to be disabled; a disabled control would strand the focus.
        if let Some(title) = self.control(ID_TITLE) {
            unsafe {
                let _ = SetFocus(Some(title));
            }
        }
        self.set_status(STR_SAVING, false);
        self.sync_enabled();
    }

    fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        let fonts = Fonts::new(scale);
        for (id, c) in &self.controls {
            fonts.apply(*c, *id);
        }
        // The old fonts are deleted only once no control uses them.
        self.fonts = fonts;
    }
}

/// Posts the editor's final result (built by `result` while the controls
/// still exist), hands the foreground back if the editor still has it, and
/// destroys the window.
fn finish(hwnd: HWND, result: impl FnOnce(&Editor) -> EditorResult) {
    let Some(mut editor) = take_editor(hwnd) else {
        return;
    };
    if !editor.finished {
        editor.finished = true;
        post_result(editor.owner, result(&editor));
    }
    unsafe {
        // Only when closed from the keyboard or a button: a click elsewhere
        // already chose the next foreground window.
        if GetForegroundWindow() == hwnd
            && !editor.previous.is_invalid()
            && IsWindow(Some(editor.previous)).as_bool()
        {
            let _ = SetForegroundWindow(editor.previous);
        }
        let _ = DestroyWindow(hwnd);
    }
    STATUS_LABEL.set((0, false));
    LAST_FOCUS.set(0);
    // Dropped after the controls are gone: deletes the fonts they used.
    drop(editor);
}

fn on_command(hwnd: HWND, id: i32, code: u32) {
    enum Next {
        Nothing,
        Close,
        Delete(String),
    }
    let next = with_editor(hwnd, |ed| match (id, code) {
        (ID_OK, BN_CLICKED) if ed.is_note() => Next::Close,
        (ID_OK, BN_CLICKED) => {
            ed.save_event();
            Next::Nothing
        }
        (ID_CANCEL, BN_CLICKED) => Next::Close,
        (ID_DELETE, BN_CLICKED) => match &ed.mode {
            Mode::Note { id: Some(id) } => Next::Delete(id.clone()),
            _ => Next::Nothing,
        },
        (ID_ALLDAY, BN_CLICKED) => {
            ed.sync_enabled();
            Next::Nothing
        }
        // STN_CLICKED == BN_CLICKED == 0: the "Seharian" label toggles the box.
        (ID_ALLDAY_LABEL, BN_CLICKED) => {
            if let Some(check) = ed.control(ID_ALLDAY) {
                let state = if ed.all_day_checked() {
                    0
                } else {
                    BST_CHECKED.0
                };
                send(check, BM_SETCHECK, state as usize, 0);
            }
            ed.sync_enabled();
            Next::Nothing
        }
        _ => Next::Nothing,
    });
    match next {
        Some(Next::Close) => finish(hwnd, Editor::closing_result),
        Some(Next::Delete(id)) => finish(hwnd, move |_| EditorResult::NoteDeleted { id }),
        _ => {}
    }
}

fn on_status(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // The editor owns the message whatever happens next.
    let ptr = lparam.0 as *mut String;
    let message = (!ptr.is_null()).then(|| *unsafe { Box::from_raw(ptr) });
    let saved = wparam.0 == 1;
    let waiting = with_editor(hwnd, |ed| {
        let Mode::Event { saving, .. } = &mut ed.mode else {
            return false;
        };
        if !*saving {
            return false;
        }
        if !saved {
            *saving = false;
            ed.set_status(message.as_deref().unwrap_or(STR_SAVE_FAILED), true);
            ed.sync_enabled();
        }
        true
    });
    if waiting == Some(true) && saved {
        finish(hwnd, |_| EditorResult::Closed);
    }
}

fn on_activate(hwnd: HWND, state: u32) {
    if state == WA_INACTIVE {
        let close = with_editor(hwnd, |ed| ed.is_note() && ed.was_active && !ed.finished);
        if close == Some(true) {
            // Never destroy inside WM_ACTIVATE.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_EDITOR_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    } else {
        with_editor(hwnd, |ed| ed.was_active = true);
        focus_last(hwnd);
    }
}

fn on_dpi_changed(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let dpi = (wparam.0 & 0xFFFF) as f32;
    let scale = if dpi > 0.0 { dpi / 96.0 } else { 1.0 };
    with_editor(hwnd, |ed| ed.set_scale(scale));
    let suggested = lparam.0 as *const RECT;
    if let Some(rc) = unsafe { suggested.as_ref() } {
        // WM_SIZE (sent from here, borrow released) lays the controls out again.
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
    with_editor(hwnd, |ed| ed.layout());
}

/// The window went away without `finish` (e.g. its owner was destroyed):
/// still post the final result once.
fn on_destroy(hwnd: HWND) {
    with_editor(hwnd, |ed| {
        if !ed.finished {
            ed.finished = true;
            post_result(ed.owner, ed.closing_result());
        }
    });
}

fn color_control(wparam: WPARAM, lparam: LPARAM, msg: u32) -> LRESULT {
    let hdc = HDC(wparam.0 as *mut c_void);
    let control = HWND(lparam.0 as *mut c_void);
    let (window, field) = brushes();
    let id = unsafe { GetDlgCtrlID(control) };
    // Read-only edits ask with WM_CTLCOLORSTATIC.
    let is_field = msg == WM_CTLCOLOREDIT || matches!(id, ID_TEXT | ID_TITLE | ID_NOTES);
    let (status, error) = STATUS_LABEL.get();
    let text = if error && status != 0 && status == lparam.0 {
        ERROR_TEXT
    } else {
        TEXT
    };
    unsafe {
        SetTextColor(hdc, text);
        SetBkColor(hdc, if is_field { FIELD } else { BG });
    }
    let brush = if is_field { field } else { window };
    LRESULT(brush.0 as isize)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => color_control(wparam, lparam, msg),
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            on_command(hwnd, id, code);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            on_activate(hwnd, (wparam.0 & 0xFFFF) as u32);
            result
        }
        WM_CLOSE | WM_APP_EDITOR_CLOSE => {
            finish(hwnd, Editor::closing_result);
            LRESULT(0)
        }
        WM_APP_EDITOR_STATUS => {
            on_status(hwnd, wparam, lparam);
            LRESULT(0)
        }
        WM_NOTIFY => {
            let header = lparam.0 as *const NMHDR;
            if let Some(header) = unsafe { header.as_ref() }
                && header.code == DTN_DROPDOWN
            {
                // The drop-down calendar exists only while open.
                let calendar = HWND(send(header.hwndFrom, DTM_GETMONTHCAL, 0, 0) as *mut c_void);
                if !calendar.is_invalid() {
                    unsafe {
                        let _ = SetWindowSubclass(
                            calendar,
                            Some(control_proc),
                            SUBCLASS_ID,
                            Role::Paint as usize,
                        );
                    }
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SIZE => {
            with_editor(hwnd, |ed| ed.layout());
            LRESULT(0)
        }
        WM_DPICHANGED => {
            on_dpi_changed(hwnd, wparam, lparam);
            LRESULT(0)
        }
        WM_DESTROY => {
            on_destroy(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // Frees the fonts once the controls are gone, if `finish` did not.
            drop(take_editor(hwnd));
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

const SUBCLASS_ID: usize = 1;

/// Maps a light-theme pixel (`0x00RRGGBB`) onto the dark palette: white
/// becomes `FIELD`, black becomes `TEXT`, grays fall in between. Colored
/// pixels (the blue selection, the today marker) keep their color.
fn darken(px: u32) -> u32 {
    let channel = |v: u32, shift: u32| (v >> shift) & 0xFF;
    let (r, g, b) = (channel(px, 16), channel(px, 8), channel(px, 0));
    if r.max(g).max(b) - r.min(g).min(b) > 48 {
        return px;
    }
    let ink = 255 - (r * 299 + g * 587 + b * 114) / 1000;
    let mix = |shift_colorref: u32| {
        let from = channel(FIELD.0, shift_colorref);
        let to = channel(TEXT.0, shift_colorref);
        (from * (255 - ink) + to * ink) / 255
    };
    // COLORREF is 0x00BBGGRR.
    (mix(0) << 16) | (mix(8) << 8) | mix(16)
}

/// Paints a control that ignores dark colors (the date/time pickers, their
/// spin buttons and the drop-down calendar): it draws itself into an
/// off-screen bitmap via `WM_PRINTCLIENT`, then `darken` recolors it.
fn paint_dark(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if !hdc.is_invalid() && w > 0 && h > 0 {
            let mem = CreateCompatibleDC(Some(hdc));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            if let Ok(dib) = CreateDIBSection(Some(hdc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                let old = SelectObject(mem, HGDIOBJ(dib.0));
                let flags = (PRF_CLIENT | PRF_ERASEBKGND) as isize;
                DefSubclassProc(hwnd, WM_PRINTCLIENT, WPARAM(mem.0 as usize), LPARAM(flags));
                let _ = GdiFlush();
                if !bits.is_null() {
                    let len = (w as usize).saturating_mul(h as usize);
                    let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, len);
                    for px in pixels {
                        *px = darken(*px);
                    }
                }
                let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(dib.0));
            }
            let _ = DeleteDC(mem);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn post_command(control: HWND, id: i32) {
    unsafe {
        if let Ok(parent) = GetParent(control) {
            let _ = PostMessageW(Some(parent), WM_COMMAND, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn key_down(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

/// Keyboard for every input control: Esc closes, Ctrl+Enter (or Enter outside
/// a multi-line field) saves, Tab / Shift+Tab move the focus, Ctrl+A selects
/// all in a text field.
unsafe extern "system" fn control_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let role = Role::from_ref(data);
    if matches!(role, Some(Role::Picker | Role::Paint)) {
        match msg {
            WM_PAINT => {
                paint_dark(hwnd);
                return LRESULT(0);
            }
            // `paint_dark` paints the background too.
            WM_ERASEBKGND => return LRESULT(1),
            _ => {}
        }
    }
    if role == Some(Role::Paint) {
        if msg == WM_NCDESTROY {
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(control_proc), SUBCLASS_ID);
            }
        }
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    let dropdown_open = || role == Some(Role::Picker) && send(hwnd, DTM_GETMONTHCAL, 0, 0) != 0;
    match msg {
        WM_KEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as u16;
            let ctrl = key_down(VK_CONTROL.0);
            if key == VK_ESCAPE.0 && !dropdown_open() {
                post_command(hwnd, ID_CANCEL);
                return LRESULT(0);
            }
            if key == VK_RETURN.0 && !dropdown_open() {
                match role {
                    Some(Role::Multi) if !ctrl => {}
                    Some(Role::Button) if !ctrl => {
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), BM_CLICK, WPARAM(0), LPARAM(0));
                        }
                        return LRESULT(0);
                    }
                    _ => {
                        post_command(hwnd, ID_OK);
                        return LRESULT(0);
                    }
                }
            }
            if key == VK_TAB.0 && !ctrl {
                unsafe {
                    if let Ok(parent) = GetParent(hwnd)
                        && let Ok(next) =
                            GetNextDlgTabItem(parent, Some(hwnd), key_down(VK_SHIFT.0))
                    {
                        let _ = SetFocus(Some(next));
                    }
                }
                return LRESULT(0);
            }
            if key == VK_A.0 && ctrl && matches!(role, Some(Role::Multi | Role::Line)) {
                send(hwnd, EM_SETSEL, 0, -1);
                return LRESULT(0);
            }
        }
        WM_CHAR => {
            // Swallow the characters of the keys handled above, so fields
            // neither beep nor insert them.
            let ch = wparam.0;
            let multi = role == Some(Role::Multi);
            let swallowed = match ch {
                0x01 | 0x09 | 0x0A => true,
                0x1B => !dropdown_open(),
                0x0D => !multi,
                _ => false,
            };
            if swallowed {
                return LRESULT(0);
            }
        }
        WM_SETFOCUS => LAST_FOCUS.set(hwnd.0 as isize),
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(control_proc), SUBCLASS_ID);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// `--demo-editor note|note-new|event|event-new|event-ro` (debug builds only):
/// opens one editor under where the panel would be, with a hidden owner that
/// prints each result. `EventSave` gets a failure after 1.5 s, then success on
/// the next try. Ends when the editor closes.
#[cfg(debug_assertions)]
pub fn run_demo(which: Option<&str>) {
    use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTOPRIMARY;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, KillTimer, MSG, PostQuitMessage, SetTimer, TranslateMessage,
        WM_TIMER,
    };

    thread_local! {
        static DEMO_EDITOR: Cell<isize> = const { Cell::new(0) };
        static SAVES: Cell<u32> = const { Cell::new(0) };
    }

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_APP_EDITOR_RESULT => {
                if let Some(result) = unsafe { take_result(lparam) } {
                    println!("demo-editor: {result:?}");
                    if matches!(result, EditorResult::EventSave { .. }) {
                        unsafe { SetTimer(Some(hwnd), 1, 1500, None) };
                    } else if result.closes_editor() {
                        unsafe { PostQuitMessage(0) };
                    }
                }
                LRESULT(0)
            }
            WM_TIMER => {
                unsafe {
                    let _ = KillTimer(Some(hwnd), 1);
                }
                let editor = HWND(DEMO_EDITOR.get() as *mut c_void);
                let n = SAVES.get();
                SAVES.set(n + 1);
                let (ok, message) = if n == 0 {
                    let text = "Gagal simpan: tidak ada koneksi (demo)".to_string();
                    (0, Box::into_raw(Box::new(text)) as isize)
                } else {
                    (1, 0)
                };
                let posted = unsafe {
                    PostMessageW(
                        Some(editor),
                        WM_APP_EDITOR_STATUS,
                        WPARAM(ok),
                        LPARAM(message),
                    )
                };
                if posted.is_err() && message != 0 {
                    drop(unsafe { Box::from_raw(message as *mut String) });
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    let date = chrono::Local::now().date_naive();
    let draft = |title: &str| EventDraft {
        title: title.into(),
        date,
        all_day: false,
        start: NaiveTime::from_hms_opt(13, 30, 0).unwrap_or(NaiveTime::MIN),
        end: NaiveTime::from_hms_opt(14, 15, 0).unwrap_or(NaiveTime::MIN),
        description: "Bahas layout panel\nBawa laptop".into(),
    };
    let event = |event_id: Option<&str>, title: &str, read_only| EditorKind::Event {
        event_id: event_id.map(Into::into),
        calendar_id: Some("primary".into()),
        draft: draft(title),
        read_only,
    };
    let kind = match which.unwrap_or("note") {
        "note" => EditorKind::Note {
            id: Some("demo".into()),
            text: "Beli kopi\nTelepon ibu jam 7 malam\nCek tiket kereta".into(),
        },
        "note-new" => EditorKind::Note {
            id: None,
            text: String::new(),
        },
        "event" => event(Some("demo-event"), "Rapat desain", false),
        "event-new" => event(None, "", false),
        "event-ro" => event(Some("demo-event"), "Libur nasional", true),
        other => {
            eprintln!(
                "demo-editor: unknown kind `{other}` (note|note-new|event|event-new|event-ro)"
            );
            return;
        }
    };

    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let instance: HINSTANCE = module.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(owner_proc),
            hInstance: instance,
            lpszClassName: w!("DynamicIslandEditorDemoOwner"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        // Hidden, like the island is to the editor: just something to own it.
        let Ok(owner) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("DynamicIslandEditorDemoOwner"),
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
            eprintln!("demo-editor: cannot create the owner window");
            return;
        };
        // Bottom center of where the open panel sits on the primary monitor.
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
        match open_editor(owner, anchor, kind) {
            Ok(editor) => {
                DEMO_EDITOR.set(editor.0 as isize);
                println!("demo-editor: hwnd {:#x}", editor.0 as usize);
            }
            Err(error) => {
                eprintln!("demo-editor: open failed: {error}");
                return;
            }
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

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn draft(title: &str, all_day: bool, start: (u32, u32), end: (u32, u32)) -> EventDraft {
        EventDraft {
            title: title.into(),
            date: NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(),
            all_day,
            start: NaiveTime::from_hms_opt(start.0, start.1, 0).unwrap(),
            end: NaiveTime::from_hms_opt(end.0, end.1, 0).unwrap(),
            description: String::new(),
        }
    }

    const SIZE_400: SIZE = SIZE { cx: 400, cy: 220 };

    #[test]
    fn centered_below_anchor() {
        let work = rect(0, 0, 1920, 1040);
        assert_eq!(
            editor_rect(POINT { x: 960, y: 330 }, SIZE_400, work),
            rect(760, 338, 1160, 558)
        );
    }

    #[test]
    fn clamped_to_work_area() {
        let work = rect(0, 0, 1920, 1040);
        // Off the right and bottom edges.
        assert_eq!(
            editor_rect(POINT { x: 1900, y: 1000 }, SIZE_400, work),
            rect(1520, 820, 1920, 1040)
        );
        // Off the left edge of a monitor left of the primary one.
        let left_monitor = rect(-1280, 0, 0, 984);
        assert_eq!(
            editor_rect(POINT { x: -1250, y: 100 }, SIZE_400, left_monitor),
            rect(-1280, 108, -880, 328)
        );
        // Larger than the work area: pinned to its top-left corner.
        let tiny = rect(0, 0, 300, 200);
        assert_eq!(
            editor_rect(POINT { x: 150, y: 50 }, SIZE_400, tiny),
            rect(0, 0, 400, 220)
        );
    }

    #[test]
    fn empty_title_rejected() {
        assert_eq!(
            validate_event(&draft("", false, (9, 0), (10, 0))),
            Err("Judul wajib diisi")
        );
        assert_eq!(
            validate_event(&draft("  \t ", true, (9, 0), (10, 0))),
            Err("Judul wajib diisi")
        );
        assert_eq!(
            validate_event(&draft("Rapat", false, (9, 0), (10, 0))),
            Ok(())
        );
    }

    #[test]
    fn end_before_start_rejected() {
        let msg = Err("Jam selesai harus setelah jam mulai");
        assert_eq!(
            validate_event(&draft("Rapat", false, (10, 0), (9, 30))),
            msg
        );
        assert_eq!(
            validate_event(&draft("Rapat", false, (10, 0), (10, 0))),
            msg
        );
        assert_eq!(
            validate_event(&draft("Rapat", false, (10, 0), (10, 1))),
            Ok(())
        );
    }

    #[test]
    fn edit_text_line_breaks_round_trip() {
        assert_eq!(to_edit_text("a\nb\r\nc"), "a\r\nb\r\nc");
        assert_eq!(from_edit_text("a\r\nb\r\n"), "a\nb\n");
        let text = "Beli kopi\n\nTelepon ibu";
        assert_eq!(from_edit_text(&to_edit_text(text)), text);
    }

    #[test]
    fn picker_values_round_trip() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let time = NaiveTime::from_hms_opt(13, 30, 0).unwrap();
        let st = system_time(date, time);
        assert_eq!(st.wDayOfWeek, 0, "Sunday");
        assert_eq!(date_of(&st), Some(date));
        assert_eq!(time_of(&st), Some(time));
        let bad = SYSTEMTIME {
            wYear: 2026,
            wMonth: 2,
            wDay: 30,
            ..Default::default()
        };
        assert_eq!(date_of(&bad), None);
    }

    #[test]
    fn only_event_save_keeps_the_editor() {
        let save = EditorResult::EventSave {
            event_id: None,
            calendar_id: None,
            draft: draft("Rapat", false, (9, 0), (10, 0)),
        };
        assert!(!save.closes_editor());
        assert!(EditorResult::Closed.closes_editor());
        assert!(EditorResult::NoteDeleted { id: "n".into() }.closes_editor());
        let saved = EditorResult::NoteSaved {
            id: None,
            text: String::new(),
        };
        assert!(saved.closes_editor());
        let lparam = LPARAM(Box::into_raw(Box::new(saved.clone())) as isize);
        assert_eq!(unsafe { take_result(lparam) }, Some(saved));
        assert_eq!(unsafe { take_result(LPARAM(0)) }, None);
    }

    #[test]
    fn darken_maps_light_grays_and_keeps_colors() {
        let field = 0x2B2B2E;
        let text = 0xEEEEEE;
        assert_eq!(darken(0xFFFFFF), field);
        assert_eq!(darken(0x000000), text);
        // Blue selection highlight stays.
        assert_eq!(darken(0x0078D7), 0x0078D7);
        // A mid gray lands between the two.
        let mid = darken(0x808080);
        assert!(mid & 0xFF > 0x2E && mid & 0xFF < 0xEE);
    }

    #[test]
    fn layouts_stay_inside_the_client_area() {
        let note_ids = [ID_TEXT, ID_DELETE];
        let event_ids: Vec<i32> = specs(&EditorKind::Event {
            event_id: None,
            calendar_id: None,
            draft: draft("", false, (9, 0), (10, 0)),
            read_only: false,
        })
        .iter()
        .map(|s| s.id)
        .collect();
        // Roughly the client areas of the two windows (caption and borders off).
        for (note, ids, cw, ch) in [
            (true, &note_ids[..], 398.0, 190.0),
            (false, &event_ids[..], 418.0, 230.0),
        ] {
            for (id, [x, y, w, h]) in layout(note, ids, cw, ch) {
                assert!(x >= 0.0 && y >= 0.0 && w > 0.0 && h > 0.0, "{id}");
                assert!(x + w <= cw && y + h <= ch, "{id} outside {cw}x{ch}");
            }
        }
    }

    #[test]
    fn all_day_ignores_times() {
        assert_eq!(
            validate_event(&draft("Libur", true, (10, 0), (9, 0))),
            Ok(())
        );
        assert_eq!(
            validate_event(&draft("Libur", true, (10, 0), (10, 0))),
            Ok(())
        );
    }
}
