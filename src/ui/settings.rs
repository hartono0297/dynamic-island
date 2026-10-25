//! The settings window: a normal, single-instance window on the island's UI
//! thread, drawn with Direct2D (`settings_controls`) from a pure layout
//! (`settings_layout`). Only the two Google credential fields are native
//! EDIT controls.
//!
//! The window never changes settings itself: each click posts an `Action`
//! to its owner (the island) with `WM_APP_SETTINGS_ACTION`, and the owner
//! applies and saves it, then pushes the new `Model` back with `update`.
//! Controls animate when the model changes, so a change made elsewhere (the
//! tray menu's style items) animates the same way. Opening links, the data
//! folder and the Claude Code hook buttons need no island state and run
//! here.
//!
//! Nothing redraws while idle: hover and press repaint on change only, and
//! animations run a `FrameClock` only while they last.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Once;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DeleteObject, EndPaint, FW_NORMAL, GetDC, GetMonitorInfoW, GetTextMetricsW,
    HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC, SelectObject, SetBkColor,
    SetTextColor, TEXTMETRICW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, EM_SETSEL, SetWindowTheme, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi,
    MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_A, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, EN_CHANGE, EN_KILLFOCUS,
    EN_SETFOCUS, ES_AUTOHSCROLL, ES_PASSWORD, GetClientRect, GetWindowTextLengthW, GetWindowTextW,
    HICON, HMENU, HTCLIENT, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND, IMAGE_ICON, IsIconic,
    IsWindow, KillTimer, LR_DEFAULTCOLOR, LoadCursorW, LoadImageW, MoveWindow, PostMessageW,
    RegisterClassExW, SM_CXICON, SM_CXSMICON, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNA,
    SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetCursor, SetForegroundWindow,
    SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR,
    WM_SETFONT, WM_SETICON, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

use super::frame_clock::{FrameClock, WM_APP_FRAME};
use super::settings_controls::{Gfx, Scene, ease};
use super::settings_layout::{
    CLIENT_H, CLIENT_W, Field, GUIDE_URL, GoogleState, Group, Hit, Layout, RELEASES_URL, REPO_URL,
    Section, State, Tone, battery_step, layout,
};
use crate::calendar::CalStatus;
use crate::claude::install::{self, InstallOutcome, UninstallOutcome};
use crate::config::{Config, Style};
use crate::i18n::{Key, Lang, text};
use crate::model::PomoField;
use crate::pomodoro::PomoSettings;
use crate::sources::battery::read_battery;
use crate::theme::{self, Accent, Surface, WindowPalette, WindowTheme};

/// Posted to the owner for each change: `lparam` is a `Box<Action>` raw
/// pointer the owner frees (`take_action`). WM_APP + 1..7 are taken.
pub const WM_APP_SETTINGS_ACTION: u32 = WM_APP + 8;
/// Enter in a credential field, posted by the field to the window.
const WM_APP_SUBMIT: u32 = WM_APP + 9;
/// The section fade ended: the credential fields may show.
const WM_APP_PLACE: u32 = WM_APP + 10;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Style(Style),
    Accent(Accent),
    Surface(Surface),
    WindowMode(WindowTheme),
    Lang(Lang),
    Autostart(bool),
    IdleClock(bool),
    Lyrics(bool),
    BatteryThreshold(u8),
    DndHelp,
    Pomo(PomoField, i32),
    SaveGoogle {
        id: String,
        secret: String,
    },
    ConnectGoogle,
    DisconnectGoogle,
    /// The window closed.
    Closed,
}

/// Takes ownership of a `WM_APP_SETTINGS_ACTION`'s `lparam`.
///
/// # Safety
/// `lparam` must come from a `WM_APP_SETTINGS_ACTION` message, taken once.
pub unsafe fn take_action(lparam: LPARAM) -> Option<Action> {
    let ptr = lparam.0 as *mut Action;
    (!ptr.is_null()).then(|| *unsafe { Box::from_raw(ptr) })
}

/// What the window shows, as the owner has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub style: Style,
    pub accent: Accent,
    pub surface: Surface,
    pub window: WindowTheme,
    pub lang: Lang,
    pub autostart: bool,
    pub idle_clock: bool,
    pub lyrics: bool,
    pub battery_threshold: u8,
    pub pomodoro: PomoSettings,
    /// A pomodoro phase runs or is paused.
    pub pomodoro_locked: bool,
    pub calendar: CalStatus,
    pub google_client_id: String,
    pub google_client_secret: String,
}

impl Model {
    pub fn from_config(cfg: &Config) -> Self {
        Model {
            style: cfg.style,
            accent: cfg.accent,
            surface: cfg.surface,
            window: cfg.window_theme,
            lang: cfg.language,
            autostart: cfg.start_with_windows,
            idle_clock: cfg.idle_clock,
            lyrics: cfg.lyrics,
            battery_threshold: cfg.battery_threshold,
            pomodoro: cfg.pomodoro,
            pomodoro_locked: false,
            calendar: CalStatus::NotConfigured,
            google_client_id: cfg.google_client_id.clone(),
            google_client_secret: cfg.google_client_secret.clone(),
        }
    }
}

const ANIM: Duration = Duration::from_millis(120);
const TIMER_CONFIRM: usize = 1;
const TIMER_GOOGLE_NOTE: usize = 2;
const TIMER_HOOK_NOTE: usize = 3;
const CONFIRM_MS: u32 = 4000;
const NOTE_MS: u32 = 5000;
const ERROR_NOTE_MS: u32 = 10_000;
const ID_CLIENT_ID: i32 = 101;
const ID_CLIENT_SECRET: i32 = 102;
const SUBCLASS_ID: usize = 1;
/// Upward offset the new section's content starts from.
const CONTENT_RISE: f32 = 6.0;

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
}

impl Tween {
    /// From where `current` is now (or `rest`) to `to`.
    fn toward(current: Option<Tween>, rest: f32, to: f32, now: Instant) -> Tween {
        Tween {
            from: current.map_or(rest, |t| t.value(now)),
            to,
            start: now,
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.start).as_secs_f32() / ANIM.as_secs_f32()
    }

    fn value(&self, now: Instant) -> f32 {
        self.from + (self.to - self.from) * ease(self.progress(now))
    }

    fn running(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }
}

#[derive(Default)]
struct Anims {
    segments: Vec<(Group, Tween)>,
    /// Toggle knobs sliding, by the toggle's hit.
    toggles: Vec<(Hit, Tween)>,
    nav: Option<Tween>,
    content: Option<Tween>,
}

impl Anims {
    fn running(&self, now: Instant) -> bool {
        let live = |t: &Option<Tween>| t.is_some_and(|t| t.running(now));
        live(&self.nav)
            || live(&self.content)
            || self.segments.iter().any(|(_, t)| t.running(now))
            || self.toggles.iter().any(|(_, t)| t.running(now))
    }

    fn segment(&self, group: Group) -> Option<Tween> {
        self.segments
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, t)| *t)
    }

    fn set_segment(&mut self, group: Group, t: Tween) {
        self.segments.retain(|(g, _)| *g != group);
        self.segments.push((group, t));
    }

    /// Slides the knob of the toggle `hit` from `from` toward `on`.
    fn slide_toggle(&mut self, hit: Hit, from: bool, on: bool, now: Instant) {
        let running = self
            .toggles
            .iter()
            .find(|(h, _)| *h == hit)
            .map(|(_, t)| *t);
        let t = Tween::toward(
            running,
            f32::from(u8::from(from)),
            f32::from(u8::from(on)),
            now,
        );
        self.toggles.retain(|(h, _)| *h != hit);
        self.toggles.push((hit, t));
    }

    /// Drops finished tweens.
    fn prune(&mut self, now: Instant) {
        self.segments.retain(|(_, t)| t.running(now));
        self.toggles.retain(|(_, t)| t.running(now));
        for t in [&mut self.nav, &mut self.content] {
            if t.is_some_and(|t| !t.running(now)) {
                *t = None;
            }
        }
    }
}

/// The two credential fields and the GDI objects they use.
struct Edits {
    id: HWND,
    secret: HWND,
    font: HFONT,
    /// Text line height of `font`, in physical pixels.
    line_h: i32,
    brush: HBRUSH,
    brush_rgb: u32,
}

impl Edits {
    fn hwnds(&self) -> [HWND; 2] {
        [self.id, self.secret]
    }
}

impl Drop for Edits {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
            let _ = DeleteObject(HGDIOBJ(self.brush.0));
        }
    }
}

struct Settings {
    hwnd: HWND,
    owner: HWND,
    dpi: u32,
    model: Model,
    section: Section,
    layout: Layout,
    gfx: Gfx,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    /// `TME_LEAVE` is armed.
    tracking: bool,
    anims: Anims,
    clock: Option<FrameClock>,
    edits: Option<Edits>,
    /// What the credential fields hold now.
    field_text: [String; 2],
    focused: Option<Field>,
    has_battery: bool,
    hooks: Option<install::HookState>,
    google_note: Option<(String, Tone)>,
    hook_note: Option<(String, Tone)>,
    /// "Putuskan" waits for its second click.
    confirm: bool,
    /// Small and large window icons, owned (not shared) and destroyed with the window.
    icons: [HICON; 2],
    /// The mode the title bar and fields were last styled for.
    styled_light: Option<bool>,
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// Runs `f` on the open window's state, unless a handler up the stack holds it.
fn with<R>(f: impl FnOnce(&mut Settings) -> R) -> Option<R> {
    SETTINGS.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        slot.as_mut().map(f)
    })
}

pub fn is_open() -> bool {
    SETTINGS.with(|cell| cell.try_borrow().map(|s| s.is_some()).unwrap_or(true))
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

/// `0xRRGGBB` as GDI's `0x00BBGGRR`.
const fn colorref(rgb: u32) -> COLORREF {
    COLORREF((rgb >> 16) & 0xFF | (rgb & 0xFF00) | (rgb & 0xFF) << 16)
}

fn post_action(owner: HWND, action: Action) {
    let ptr = Box::into_raw(Box::new(action));
    let posted = unsafe {
        PostMessageW(
            Some(owner),
            WM_APP_SETTINGS_ACTION,
            WPARAM(0),
            LPARAM(ptr as isize),
        )
    };
    if posted.is_err() {
        drop(unsafe { Box::from_raw(ptr) });
    }
}

const CLASS_NAME: PCWSTR = w!("DynamicIslandSettings");

fn register_class(instance: HINSTANCE) -> Result<()> {
    static ONCE: Once = Once::new();
    let mut result = Ok(());
    ONCE.call_once(|| unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            let error = Error::from_thread();
            if error.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
                result = Err(error);
            }
        }
    });
    result
}

/// `assets/icon.ico` at the small or large icon size for `dpi`. Not shared:
/// the window owns it, so the 16 px image is the real 16 px one.
fn load_icon(instance: HINSTANCE, dpi: u32, large: bool) -> HICON {
    let metric = if large { SM_CXICON } else { SM_CXSMICON };
    unsafe {
        let size = GetSystemMetricsForDpi(metric, dpi);
        LoadImageW(
            Some(instance),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .map(|h| HICON(h.0))
        .unwrap_or_default()
    }
}

/// Sets the window's icons for `dpi` and destroys the ones they replace.
fn set_icons(hwnd: HWND, dpi: u32) {
    let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
        return;
    };
    let instance: HINSTANCE = module.into();
    let icons = [
        load_icon(instance, dpi, false),
        load_icon(instance, dpi, true),
    ];
    send(hwnd, WM_SETICON, ICON_SMALL as usize, icons[0].0 as isize);
    send(hwnd, WM_SETICON, ICON_BIG as usize, icons[1].0 as isize);
    let old = with(|st| std::mem::replace(&mut st.icons, icons));
    match old {
        Some(old) => destroy_icons(old),
        // The window is not registered (yet): nobody else will free them.
        None => destroy_icons(icons),
    }
}

fn destroy_icons(icons: [HICON; 2]) {
    for icon in icons {
        if !icon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn create_font(dpi: u32) -> HFONT {
    let px = (13.0 * dpi as f32 / 96.0).round() as i32;
    unsafe {
        CreateFontW(
            -px,
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
            w!("Segoe UI"),
        )
    }
}

fn line_height(hwnd: HWND, font: HFONT) -> i32 {
    unsafe {
        let hdc = GetDC(Some(hwnd));
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(hdc, &mut tm);
        SelectObject(hdc, old);
        ReleaseDC(Some(hwnd), hdc);
        tm.tmHeight.max(1)
    }
}

/// The primary monitor's DPI and work area.
fn primary_monitor() -> (u32, RECT) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dx, mut dy) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            dx = 96;
        }
        (dx, info.rcWork)
    }
}

const STYLE: WINDOW_STYLE = WINDOW_STYLE(
    WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_CLIPCHILDREN.0,
);

/// Outer size for the logical client size at `dpi`.
fn outer_size(dpi: u32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: (CLIENT_W * s).round() as i32,
        bottom: (CLIENT_H * s).round() as i32,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rc, STYLE, false, WINDOW_EX_STYLE(0), dpi);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

/// Opens the window on `section` (Appearance by default), or brings the open
/// one forward (restored if minimized) with the model brought up to date.
pub fn open(owner: HWND, model: Model, section: Option<Section>) -> Result<HWND> {
    let existing = SETTINGS.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.as_ref().map(|st| st.hwnd))
    });
    if let Some(hwnd) = existing {
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            update(model);
            if let Some(section) = section {
                set_section(hwnd, section);
            }
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }
        SETTINGS.with(|cell| {
            if let Ok(mut slot) = cell.try_borrow_mut() {
                *slot = None;
            }
        });
    }

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    register_class(instance)?;
    let (dpi, work) = primary_monitor();
    let (w, h) = outer_size(dpi);
    let x = work.left + ((work.right - work.left) - w).max(0) / 2;
    let y = work.top + ((work.bottom - work.top) - h).max(0) / 2;
    let title = text(model.lang, Key::SettingsTitle);
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            &HSTRING::from(title),
            STYLE,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance),
            None,
        )?
    };
    // The window may have landed on a monitor with another DPI.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let gfx = match Gfx::new() {
        Ok(gfx) => gfx,
        Err(error) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
    };
    let edits = create_edits(hwnd, instance, dpi, &model);
    let field_text = [
        model.google_client_id.clone(),
        model.google_client_secret.clone(),
    ];
    let state = Settings {
        hwnd,
        owner,
        dpi,
        model,
        section: section.unwrap_or(Section::Appearance),
        layout: Layout::default(),
        gfx,
        hover: None,
        pressed: None,
        tracking: false,
        anims: Anims::default(),
        clock: None,
        edits,
        field_text,
        focused: None,
        has_battery: read_battery().is_some(),
        hooks: install::hook_state(),
        google_note: None,
        hook_note: None,
        confirm: false,
        icons: [HICON::default(); 2],
        styled_light: None,
    };
    let stored = SETTINGS.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => {
            *slot = Some(state);
            None
        }
        Err(_) => Some(state),
    });
    if stored.is_some() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(Error::empty());
    }
    set_icons(hwnd, dpi);
    restyle();
    relayout_and_place();
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(hwnd)
}

fn create_edits(parent: HWND, instance: HINSTANCE, dpi: u32, model: &Model) -> Option<Edits> {
    let font = create_font(dpi);
    let make = |id: i32, text: &str, extra: i32, cue: PCWSTR| -> Option<HWND> {
        let style = WS_CHILD | WS_TABSTOP | WINDOW_STYLE((ES_AUTOHSCROLL | extra) as u32);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                &HSTRING::from(text),
                style,
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(id as isize as *mut c_void)),
                Some(instance),
                None,
            )
            .ok()?
        };
        send(hwnd, WM_SETFONT, font.0 as usize, 0);
        send(hwnd, EM_SETCUEBANNER, 0, cue.0 as isize);
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(edit_proc), SUBCLASS_ID, 0);
        }
        Some(hwnd)
    };
    let id = make(
        ID_CLIENT_ID,
        &model.google_client_id,
        0,
        w!("…apps.googleusercontent.com"),
    );
    let secret = make(
        ID_CLIENT_SECRET,
        &model.google_client_secret,
        ES_PASSWORD,
        w!("GOCSPX-…"),
    );
    match (id, secret) {
        (Some(id), Some(secret)) => Some(Edits {
            id,
            secret,
            font,
            line_h: line_height(parent, font),
            brush: unsafe { CreateSolidBrush(colorref(0)) },
            brush_rgb: 0,
        }),
        _ => {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
            None
        }
    }
}

/// Brings the window up to `model`; animates what changed.
pub fn update(model: Model) {
    let changed = with(|st| {
        if st.model == model {
            return None;
        }
        Some(st.on_model(model))
    })
    .flatten();
    let Some(changes) = changed else {
        return;
    };
    if let Some((hwnd, lang)) = changes.title {
        unsafe {
            let _ = SetWindowTextW(hwnd, &HSTRING::from(text(lang, Key::SettingsTitle)));
        }
    }
    for (edit, value) in changes.fields {
        unsafe {
            let _ = SetWindowTextW(edit, &HSTRING::from(value.as_str()));
        }
    }
    restyle();
    relayout_and_place();
}

/// Restyles for the window palette in effect (Windows switched between
/// light and dark, or the window mode or accent changed).
pub fn restyle() {
    let Some((hwnd, edits, p, light_changed)) = with(|st| {
        let p = theme::current_window();
        let changed = st.styled_light != Some(p.light);
        st.styled_light = Some(p.light);
        if let Some(e) = &mut st.edits
            && e.brush_rgb != p.field
        {
            let old =
                std::mem::replace(&mut e.brush, unsafe { CreateSolidBrush(colorref(p.field)) });
            e.brush_rgb = p.field;
            unsafe {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
        let edits = st.edits.as_ref().map(Edits::hwnds);
        (st.hwnd, edits, p, changed)
    }) else {
        return;
    };
    if light_changed {
        apply_mode(hwnd, &p);
        for edit in edits.into_iter().flatten() {
            let theme = if p.light {
                w!("Explorer")
            } else {
                w!("DarkMode_Explorer")
            };
            unsafe {
                let _ = SetWindowTheme(edit, theme, None);
            }
        }
    }
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        for edit in edits.into_iter().flatten() {
            let _ = InvalidateRect(Some(edit), None, true);
        }
    }
}

/// Dark or light title bar, coloured like the window so it reads as one surface.
fn apply_mode(hwnd: HWND, p: &WindowPalette) {
    let dark = i32::from(!p.light);
    let caption = colorref(p.bg).0;
    let text = colorref(p.text).0;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
        // Windows 11 only; older systems keep the standard caption.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        );
    }
}

/// Closes the window (island shutdown).
pub fn close() {
    let hwnd = SETTINGS.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|s| s.as_ref().map(|st| st.hwnd))
    });
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// What `on_model` leaves to do once the state is released.
struct ModelChanges {
    title: Option<(HWND, Lang)>,
    fields: Vec<(HWND, String)>,
}

/// Where each credential field goes: hidden, or at a client rect.
type Placement = (HWND, Option<RECT>);

/// Recomputes the layout and moves the native fields to match.
fn relayout_and_place() {
    let Some(placements) = with(|st| {
        st.relayout();
        st.placements()
    }) else {
        return;
    };
    place(placements);
}

fn place(placements: Vec<Placement>) {
    for (edit, rect) in placements {
        unsafe {
            match rect {
                Some(rc) => {
                    let _ = MoveWindow(
                        edit,
                        rc.left,
                        rc.top,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        true,
                    );
                    let _ = ShowWindow(edit, SW_SHOWNA);
                }
                None => {
                    let _ = ShowWindow(edit, SW_HIDE);
                }
            }
        }
    }
}

impl Settings {
    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn state(&self) -> State<'_> {
        let m = &self.model;
        let id = self.field_text[0].trim();
        let secret = self.field_text[1].trim();
        State {
            lang: m.lang,
            style: m.style,
            accent: m.accent,
            surface: m.surface,
            window: m.window,
            autostart: m.autostart,
            idle_clock: m.idle_clock,
            lyrics: m.lyrics,
            battery_threshold: m.battery_threshold,
            has_battery: self.has_battery,
            pomodoro: m.pomodoro,
            pomodoro_locked: m.pomodoro_locked,
            google: GoogleState {
                status: m.calendar,
                dirty: id != m.google_client_id.trim() || secret != m.google_client_secret.trim(),
                has_id: !id.is_empty(),
                confirm: self.confirm,
            },
            google_note: self.google_note.as_ref().map(|(s, t)| (s.as_str(), *t)),
            hooks: self.hooks,
            hook_note: self.hook_note.as_ref().map(|(s, t)| (s.as_str(), *t)),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    fn relayout(&mut self) {
        let l = layout(self.section, &self.state(), &self.gfx);
        self.layout = l;
        self.invalidate();
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The fields show on Integrations once its content has faded in.
    fn placements(&self) -> Vec<Placement> {
        let Some(e) = &self.edits else {
            return Vec::new();
        };
        let fading = self
            .anims
            .content
            .is_some_and(|t| t.running(Instant::now()));
        let s = self.scale();
        [(Field::ClientId, e.id), (Field::ClientSecret, e.secret)]
            .into_iter()
            .map(|(field, edit)| {
                let rect = self
                    .layout
                    .fields
                    .iter()
                    .find(|(f, _)| *f == field)
                    .filter(|_| !fading)
                    .map(|(_, r)| {
                        let px = |v: f32| (v * s).round() as i32;
                        let left = px(r.x + 10.0);
                        let right = px(r.right() - 10.0);
                        let mid = px(r.y + r.h / 2.0);
                        RECT {
                            left,
                            top: mid - e.line_h / 2,
                            right,
                            bottom: mid - e.line_h / 2 + e.line_h,
                        }
                    });
                (edit, rect)
            })
            .collect()
    }

    fn on_model(&mut self, model: Model) -> ModelChanges {
        let now = Instant::now();
        let old = std::mem::replace(&mut self.model, model);
        let new = &self.model;
        let index = |group: Group, m: &Model| -> f32 {
            match group {
                Group::Style => usize::from(m.style == Style::Notch) as f32,
                Group::Surface => usize::from(m.surface == Surface::Graphite) as f32,
                Group::Window => WindowTheme::ALL
                    .iter()
                    .position(|w| *w == m.window)
                    .unwrap_or(0) as f32,
                Group::Lang => usize::from(m.lang == Lang::En) as f32,
            }
        };
        for group in [Group::Style, Group::Surface, Group::Window, Group::Lang] {
            let (from, to) = (index(group, &old), index(group, new));
            if from != to {
                let t = Tween::toward(self.anims.segment(group), from, to, now);
                self.anims.set_segment(group, t);
            }
        }
        if old.autostart != new.autostart {
            self.anims
                .slide_toggle(Hit::Autostart, old.autostart, new.autostart, now);
        }
        if old.idle_clock != new.idle_clock {
            self.anims
                .slide_toggle(Hit::IdleClock, old.idle_clock, new.idle_clock, now);
        }
        if old.lyrics != new.lyrics {
            self.anims
                .slide_toggle(Hit::Lyrics, old.lyrics, new.lyrics, now);
        }
        let mut changes = ModelChanges {
            title: None,
            fields: Vec::new(),
        };
        if old.lang != new.lang {
            changes.title = Some((self.hwnd, new.lang));
            // Messages in the old language would linger; they are short-lived anyway.
            self.google_note = None;
            self.hook_note = None;
        }
        // A credential changed elsewhere (config.json edited by hand and picked
        // up): refresh a field the user has not touched.
        if let Some(e) = &self.edits {
            let pairs = [
                (0, e.id, &old.google_client_id, &new.google_client_id),
                (
                    1,
                    e.secret,
                    &old.google_client_secret,
                    &new.google_client_secret,
                ),
            ];
            for (i, edit, before, after) in pairs {
                if before != after && self.field_text[i] == *before {
                    self.field_text[i] = after.clone();
                    changes.fields.push((edit, after.clone()));
                }
            }
        }
        self.start_clock();
        changes
    }

    fn start_clock(&mut self) {
        if self.clock.is_none() && self.anims.running(Instant::now()) {
            self.clock = Some(FrameClock::start(self.hwnd.0 as isize));
        }
    }

    fn render(&mut self) {
        let now = Instant::now();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let size = (
            (rc.right - rc.left).max(1) as u32,
            (rc.bottom - rc.top).max(1) as u32,
        );
        let content = self.anims.content.map_or(1.0, |t| t.value(now));
        let p = theme::current_window();
        let scene = Scene {
            layout: &self.layout,
            p,
            island: theme::island(self.model.accent, self.model.surface),
            style: self.model.style,
            lang: self.model.lang,
            hover: self.hover,
            pressed: self.pressed,
            nav_pos: self
                .anims
                .nav
                .map_or(self.section.index() as f32, |t| t.value(now)),
            section: self.section,
            content_alpha: content,
            content_dy: (1.0 - content) * CONTENT_RISE,
            segments: self
                .anims
                .segments
                .iter()
                .map(|(g, t)| (*g, t.value(now)))
                .collect(),
            toggles: self
                .anims
                .toggles
                .iter()
                .map(|(h, t)| (*h, t.value(now)))
                .collect(),
            focused: self.focused,
        };
        let dpi = self.dpi as f32;
        if let Err(error) = self.gfx.paint(self.hwnd, size, dpi, &scene) {
            eprintln!("dynamic-island: settings draw failed: {error}");
        }
        let content_was_running = self.anims.content.is_some();
        self.anims.prune(now);
        if !self.anims.running(now) {
            // Dropping joins the frame thread, which wakes within one frame.
            self.clock = None;
        }
        if content_was_running && self.anims.content.is_none() {
            // The fade ended: the native fields may show now.
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_PLACE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn hit_at(&self, lparam: LPARAM) -> Option<Hit> {
        let x = (lparam.0 & 0xFFFF) as i16 as f32 / self.scale();
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32 / self.scale();
        self.layout.hit(x, y)
    }

    fn set_note(&mut self, google: bool, note: Option<(String, Tone)>) {
        let error = note.as_ref().is_some_and(|(_, t)| *t == Tone::Bad);
        let (slot, timer) = if google {
            (&mut self.google_note, TIMER_GOOGLE_NOTE)
        } else {
            (&mut self.hook_note, TIMER_HOOK_NOTE)
        };
        *slot = note;
        let ms = if error { ERROR_NOTE_MS } else { NOTE_MS };
        unsafe {
            SetTimer(Some(self.hwnd), timer, ms, None);
        }
    }

    /// A click on `hit`. Returns what must happen once the state is released.
    fn on_click(&mut self, hit: Hit) -> Vec<After> {
        if hit != Hit::GoogleDisconnect && self.confirm {
            self.confirm = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_CONFIRM);
            }
        }
        let m = self.model.clone();
        let lang = m.lang;
        let post = |a: Action| vec![After::Post(a)];
        let after = match hit {
            Hit::Nav(section) => vec![After::Section(section)],
            Hit::Style(v) if v != m.style => post(Action::Style(v)),
            Hit::Accent(v) if v != m.accent => post(Action::Accent(v)),
            Hit::Surface(v) if v != m.surface => post(Action::Surface(v)),
            Hit::WindowMode(v) if v != m.window => post(Action::WindowMode(v)),
            Hit::Lang(v) if v != m.lang => post(Action::Lang(v)),
            Hit::Autostart => post(Action::Autostart(!m.autostart)),
            Hit::IdleClock => post(Action::IdleClock(!m.idle_clock)),
            Hit::Lyrics => post(Action::Lyrics(!m.lyrics)),
            Hit::Battery(dir) => {
                let t = battery_step(m.battery_threshold, dir);
                if t == m.battery_threshold {
                    Vec::new()
                } else {
                    post(Action::BatteryThreshold(t))
                }
            }
            Hit::DndHelp => post(Action::DndHelp),
            Hit::Pomo(field, dir) => post(Action::Pomo(field, dir)),
            Hit::GoogleSave => {
                self.set_note(true, Some((text(lang, Key::Saved).into(), Tone::Good)));
                post(self.save_action())
            }
            Hit::GoogleConnect => {
                let mut after = Vec::new();
                if self.state().google.dirty {
                    after.push(After::Post(self.save_action()));
                }
                after.push(After::Post(Action::ConnectGoogle));
                let note = text(lang, Key::CheckBrowser).into();
                self.set_note(true, Some((note, Tone::Secondary)));
                after
            }
            Hit::GoogleDisconnect if self.confirm => {
                self.confirm = false;
                let note = text(lang, Key::Disconnected).into();
                self.set_note(true, Some((note, Tone::Secondary)));
                post(Action::DisconnectGoogle)
            }
            Hit::GoogleDisconnect => {
                self.confirm = true;
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_CONFIRM, CONFIRM_MS, None);
                }
                Vec::new()
            }
            Hit::GoogleGuide => vec![After::Open(GUIDE_URL.into())],
            Hit::Repo => vec![After::Open(REPO_URL.into())],
            Hit::Releases => vec![After::Open(RELEASES_URL.into())],
            Hit::DataFolder => vec![After::OpenDataFolder],
            Hit::HooksInstall => {
                let note = match install::install_for_current_exe() {
                    InstallOutcome::Installed => (text(lang, Key::HooksOn).into(), Tone::Good),
                    InstallOutcome::AlreadyInstalled => {
                        (text(lang, Key::HooksAlready).into(), Tone::Good)
                    }
                    InstallOutcome::Error(e) => {
                        (format!("{}: {e}", text(lang, Key::Failed)), Tone::Bad)
                    }
                };
                self.hooks = install::hook_state();
                self.set_note(false, Some(note));
                Vec::new()
            }
            Hit::HooksRemove => {
                let note = match install::uninstall_for_current_user() {
                    UninstallOutcome::Removed => {
                        (text(lang, Key::HooksRemoved).into(), Tone::Secondary)
                    }
                    UninstallOutcome::NothingToRemove => {
                        (text(lang, Key::HooksNone).into(), Tone::Secondary)
                    }
                    UninstallOutcome::Error(e) => {
                        (format!("{}: {e}", text(lang, Key::Failed)), Tone::Bad)
                    }
                };
                self.hooks = install::hook_state();
                self.set_note(false, Some(note));
                Vec::new()
            }
            // The current choice again: nothing to change.
            Hit::Style(_)
            | Hit::Accent(_)
            | Hit::Surface(_)
            | Hit::WindowMode(_)
            | Hit::Lang(_) => Vec::new(),
        };
        self.relayout();
        after
    }

    fn save_action(&self) -> Action {
        Action::SaveGoogle {
            id: self.field_text[0].trim().to_owned(),
            secret: self.field_text[1].trim().to_owned(),
        }
    }

    fn switch_section(&mut self, section: Section) -> bool {
        if section == self.section {
            return false;
        }
        let now = Instant::now();
        let from = self.section.index() as f32;
        self.anims.nav = Some(Tween::toward(
            self.anims.nav,
            from,
            section.index() as f32,
            now,
        ));
        self.anims.content = Some(Tween {
            from: 0.0,
            to: 1.0,
            start: now,
        });
        self.section = section;
        self.focused = None;
        self.hover = None;
        self.start_clock();
        true
    }
}

/// Side effects of a click, run after the state is released (they may send
/// messages back to this window or pump the queue).
enum After {
    Post(Action),
    Section(Section),
    Open(String),
    OpenDataFolder,
}

fn run_after(hwnd: HWND, owner: HWND, after: Vec<After>) {
    for a in after {
        match a {
            After::Post(action) => post_action(owner, action),
            After::Section(section) => set_section(hwnd, section),
            After::Open(url) => shell_open(&url),
            After::OpenDataFolder => {
                let dir = crate::config::path()
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                let _ = std::fs::create_dir_all(&dir);
                shell_open(&dir.display().to_string());
            }
        }
    }
}

fn shell_open(target: &str) {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

fn set_section(hwnd: HWND, section: Section) {
    if with(|st| st.switch_section(section)) == Some(true) {
        // A focused field is about to hide; keep the keyboard on the window.
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
        relayout_and_place();
    }
}

fn on_mouse_move(hwnd: HWND, lparam: LPARAM) {
    let arm = with(|st| {
        let hit = st.hit_at(lparam);
        if hit != st.hover {
            st.hover = hit;
            st.invalidate();
        }
        !std::mem::replace(&mut st.tracking, true)
    });
    if arm == Some(true) {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe {
            let _ = TrackMouseEvent(&mut track);
        }
    }
}

fn on_mouse_leave() {
    with(|st| {
        st.tracking = false;
        if st.hover.take().is_some() {
            st.invalidate();
        }
    });
}

fn on_button_down(hwnd: HWND, lparam: LPARAM) {
    let hit = with(|st| {
        st.pressed = st.hit_at(lparam);
        st.invalidate();
        st.pressed
    });
    unsafe {
        // A click outside the fields takes the keyboard back from them.
        let _ = SetFocus(Some(hwnd));
        if hit.flatten().is_some() {
            SetCapture(hwnd);
        }
    }
}

fn on_button_up(hwnd: HWND, lparam: LPARAM) {
    let clicked = with(|st| {
        let hit = st.hit_at(lparam);
        let pressed = st.pressed.take();
        st.invalidate();
        (st.owner, hit.filter(|h| Some(*h) == pressed))
    });
    unsafe {
        let _ = ReleaseCapture();
    }
    let Some((owner, Some(hit))) = clicked else {
        return;
    };
    let after = with(|st| st.on_click(hit)).unwrap_or_default();
    run_after(hwnd, owner, after);
    relayout_and_place();
}

fn on_key(hwnd: HWND, key: u16) -> bool {
    if key == VK_ESCAPE.0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return true;
    }
    let step = if key == VK_DOWN.0 {
        1
    } else if key == VK_UP.0 {
        -1
    } else if key == VK_TAB.0 {
        // Tab from the window goes to the first field, when it shows.
        let first = with(|st| {
            (st.section == Section::Integrations)
                .then(|| st.edits.as_ref().map(|e| e.id))
                .flatten()
        });
        if let Some(Some(edit)) = first {
            unsafe {
                let _ = SetFocus(Some(edit));
            }
        }
        return true;
    } else {
        return false;
    };
    let next = with(|st| {
        let i = st.section.index() as i32 + step;
        Section::ALL
            .get(i.clamp(0, Section::ALL.len() as i32 - 1) as usize)
            .copied()
    });
    if let Some(Some(section)) = next {
        set_section(hwnd, section);
    }
    true
}

fn on_command(wparam: WPARAM, lparam: LPARAM) {
    let id = (wparam.0 & 0xFFFF) as i32;
    let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
    let field = match id {
        ID_CLIENT_ID => Field::ClientId,
        ID_CLIENT_SECRET => Field::ClientSecret,
        _ => return,
    };
    let edit = HWND(lparam.0 as *mut c_void);
    match code {
        EN_CHANGE => {
            let value = window_text(edit);
            with(|st| {
                st.field_text[usize::from(field == Field::ClientSecret)] = value;
                st.google_note = None;
                st.relayout();
            });
        }
        EN_SETFOCUS => {
            with(|st| {
                st.focused = Some(field);
                st.invalidate();
            });
        }
        EN_KILLFOCUS => {
            with(|st| {
                if st.focused == Some(field) {
                    st.focused = None;
                }
                st.invalidate();
            });
        }
        _ => {}
    }
}

fn on_dpi_changed(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let dpi = (wparam.0 & 0xFFFF) as u32;
    let edits = with(|st| {
        st.dpi = dpi.max(96);
        st.gfx.reset_target();
        st.edits.as_mut().map(|e| {
            let font = create_font(st.dpi);
            let old = std::mem::replace(&mut e.font, font);
            e.line_h = line_height(st.hwnd, font);
            (e.hwnds(), font, old)
        })
    })
    .flatten();
    if let Some((hwnds, font, old)) = edits {
        for edit in hwnds {
            send(edit, WM_SETFONT, font.0 as usize, 1);
        }
        unsafe {
            let _ = DeleteObject(HGDIOBJ(old.0));
        }
    }
    set_icons(hwnd, dpi);
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
    relayout_and_place();
}

fn on_timer(id: usize) {
    with(|st| {
        unsafe {
            let _ = KillTimer(Some(st.hwnd), id);
        }
        match id {
            TIMER_CONFIRM => st.confirm = false,
            TIMER_GOOGLE_NOTE => st.google_note = None,
            TIMER_HOOK_NOTE => st.hook_note = None,
            _ => return,
        }
        st.relayout();
    });
}

fn color_edit(wparam: WPARAM) -> LRESULT {
    let hdc = HDC(wparam.0 as *mut c_void);
    let p = theme::current_window();
    let brush = with(|st| st.edits.as_ref().map(|e| e.brush)).flatten();
    unsafe {
        SetTextColor(hdc, colorref(p.text));
        SetBkColor(hdc, colorref(p.field));
    }
    LRESULT(brush.map_or(0, |b| b.0 as isize))
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };
            with(Settings::render);
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        // Direct2D paints every pixel.
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            let (w, h) = (
                (lparam.0 & 0xFFFF) as u32,
                ((lparam.0 >> 16) & 0xFFFF) as u32,
            );
            with(|st| st.gfx.resize(w, h));
            LRESULT(0)
        }
        WM_APP_FRAME => {
            with(|st| {
                if let Some(clock) = &st.clock {
                    clock.ack();
                }
                st.invalidate();
            });
            LRESULT(0)
        }
        WM_APP_PLACE => {
            relayout_and_place();
            LRESULT(0)
        }
        WM_APP_SUBMIT => {
            let save = with(|st| {
                let dirty = st.state().google.dirty;
                if dirty {
                    let after = st.on_click(Hit::GoogleSave);
                    Some((st.owner, after))
                } else {
                    None
                }
            })
            .flatten();
            if let Some((owner, after)) = save {
                run_after(hwnd, owner, after);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, lparam);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            on_mouse_leave();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            on_button_down(hwnd, lparam);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            on_button_up(hwnd, lparam);
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let link = with(|st| matches!(st.hover, Some(Hit::GoogleGuide))) == Some(true);
            unsafe {
                let cursor = LoadCursorW(None, if link { IDC_HAND } else { IDC_ARROW });
                SetCursor(cursor.ok());
            }
            LRESULT(1)
        }
        WM_KEYDOWN if on_key(hwnd, (wparam.0 & 0xFFFF) as u16) => LRESULT(0),
        WM_COMMAND => {
            on_command(wparam, lparam);
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => color_edit(wparam),
        WM_TIMER => {
            on_timer(wparam.0);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            on_dpi_changed(hwnd, wparam, lparam);
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(owner) = with(|st| {
                st.clock = None;
                st.owner
            }) {
                post_action(owner, Action::Closed);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let state = SETTINGS.with(|cell| cell.try_borrow_mut().ok().and_then(|mut s| s.take()));
            if let Some(st) = state {
                destroy_icons(st.icons);
                drop(st);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn key_down(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

/// Keyboard in the credential fields: Esc closes the window, Enter saves,
/// Tab moves to the other field, Ctrl+A selects all.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let parent = || unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd) }.ok();
    match msg {
        WM_KEYDOWN => {
            let key = (wparam.0 & 0xFFFF) as u16;
            let ctrl = key_down(VK_CONTROL.0);
            if key == VK_ESCAPE.0 {
                if let Some(p) = parent() {
                    unsafe {
                        let _ = PostMessageW(Some(p), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(0);
            }
            if key == VK_RETURN.0 {
                if let Some(p) = parent() {
                    unsafe {
                        let _ = PostMessageW(Some(p), WM_APP_SUBMIT, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(0);
            }
            if key == VK_TAB.0 {
                let other = with(|st| {
                    st.edits
                        .as_ref()
                        .map(|e| if e.id == hwnd { e.secret } else { e.id })
                })
                .flatten();
                // Shift+Tab goes the same way: there are only two.
                if let Some(other) = other {
                    unsafe {
                        let _ = SetFocus(Some(other));
                    }
                }
                return LRESULT(0);
            }
            if key == VK_A.0 && ctrl {
                send(hwnd, EM_SETSEL, 0, -1);
                return LRESULT(0);
            }
        }
        WM_CHAR => {
            if matches!(wparam.0, 0x01 | 0x09 | 0x0A | 0x0D | 0x1B) {
                return LRESULT(0);
            }
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), SUBCLASS_ID);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// `--settings [section] [dark|light|system] [id|en]` (debug builds only):
/// the window on its own, with a hidden owner that applies each action to a
/// demo model and pushes it back, so every control works. Nothing is saved
/// and the island is not involved; the hook buttons and links are real, so
/// leave them alone. `DI_ACCENT`, `DI_GOOGLE` (`ok`, `needslogin`,
/// `reconnect`, `offline`, `none`) and `DI_POMO_LOCKED=1` set the start.
#[cfg(debug_assertions)]
pub fn run_demo(args: &[String]) {
    use crate::i18n;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage, WS_EX_TOOLWINDOW,
        WS_POPUP,
    };

    thread_local! {
        static DEMO: RefCell<Option<Model>> = const { RefCell::new(None) };
    }

    fn apply(m: &mut Model, action: Action) {
        match action {
            Action::Style(v) => m.style = v,
            Action::Accent(v) => m.accent = v,
            Action::Surface(v) => m.surface = v,
            Action::WindowMode(v) => m.window = v,
            Action::Lang(v) => {
                m.lang = v;
                i18n::set_lang(v);
            }
            Action::Autostart(v) => m.autostart = v,
            Action::IdleClock(v) => m.idle_clock = v,
            Action::Lyrics(v) => m.lyrics = v,
            Action::BatteryThreshold(v) => m.battery_threshold = v,
            Action::Pomo(field, dir) => {
                let (min, max, step) = crate::pomodoro::range(field);
                let v = match field {
                    PomoField::Focus => &mut m.pomodoro.focus_min,
                    PomoField::ShortBreak => &mut m.pomodoro.short_break_min,
                    PomoField::LongBreak => &mut m.pomodoro.long_break_min,
                    PomoField::Sessions => &mut m.pomodoro.sessions,
                };
                *v = if dir > 0 {
                    v.saturating_add(step)
                } else {
                    v.saturating_sub(step)
                }
                .clamp(min, max);
            }
            Action::SaveGoogle { id, secret } => {
                if m.calendar == CalStatus::NotConfigured && !id.is_empty() {
                    m.calendar = CalStatus::NeedsLogin;
                }
                m.google_client_id = id;
                m.google_client_secret = secret;
            }
            Action::ConnectGoogle => m.calendar = CalStatus::Ok,
            Action::DisconnectGoogle => m.calendar = CalStatus::NeedsLogin,
            Action::DndHelp | Action::Closed => {}
        }
        theme::set_current(theme::Theme {
            accent: m.accent,
            surface: m.surface,
            window: m.window,
        });
    }

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_APP_SETTINGS_ACTION {
            if let Some(action) = unsafe { take_action(lparam) } {
                println!("settings-demo: {action:?}");
                if action == Action::Closed {
                    unsafe { PostQuitMessage(0) };
                    return LRESULT(0);
                }
                let model = DEMO.with_borrow_mut(|m| {
                    m.as_mut().map(|m| {
                        apply(m, action);
                        m.clone()
                    })
                });
                if let Some(model) = model {
                    update(model);
                }
            }
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let section = args.first().and_then(|s| Section::from_name(s));
    let window = args
        .get(1)
        .and_then(|s| WindowTheme::from_name(s))
        .unwrap_or_default();
    let lang = args
        .get(2)
        .and_then(|s| Lang::from_code(s))
        .unwrap_or(i18n::lang());
    i18n::set_lang(lang);
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let accent = Accent::from_name(&env("DI_ACCENT")).unwrap_or_default();
    let calendar = match env("DI_GOOGLE").to_ascii_lowercase().as_str() {
        "none" => CalStatus::NotConfigured,
        "needslogin" => CalStatus::NeedsLogin,
        "reconnect" => CalStatus::Reconnect,
        "offline" => CalStatus::Offline,
        _ => CalStatus::Ok,
    };
    let configured = calendar != CalStatus::NotConfigured;
    let cfg = Config::default();
    let model = Model {
        accent,
        window,
        lang,
        pomodoro_locked: env("DI_POMO_LOCKED") == "1",
        lyrics: env("DI_LYRICS") == "1",
        calendar,
        google_client_id: if configured {
            "123456789012-demo.apps.googleusercontent.com".into()
        } else {
            String::new()
        },
        google_client_secret: if configured {
            "demo-secret".into()
        } else {
            String::new()
        },
        ..Model::from_config(&cfg)
    };
    theme::set_system_light(theme::read_system_light());
    theme::set_current(theme::Theme {
        accent,
        surface: model.surface,
        window,
    });
    DEMO.with_borrow_mut(|m| *m = Some(model.clone()));
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let instance: HINSTANCE = module.into();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(owner_proc),
            hInstance: instance,
            lpszClassName: w!("DynamicIslandSettingsDemoOwner"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let Ok(owner) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("DynamicIslandSettingsDemoOwner"),
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
            eprintln!("settings-demo: cannot create the owner window");
            return;
        };
        match open(owner, model, section) {
            Ok(hwnd) => println!("settings-demo: hwnd {:#x}", hwnd.0 as usize),
            Err(error) => {
                eprintln!("settings-demo: open failed: {error}");
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

    #[test]
    fn tweens_ease_toward_the_target_and_retarget_smoothly() {
        let t0 = Instant::now();
        let t = Tween::toward(None, 0.0, 2.0, t0);
        assert_eq!(t.value(t0), 0.0);
        assert_eq!(t.value(t0 + ANIM), 2.0);
        assert!(t.running(t0 + ANIM / 2));
        assert!(!t.running(t0 + ANIM));
        // Retargeting mid-way starts from where it is, not from the old start.
        let mid = t0 + ANIM / 2;
        let back = Tween::toward(Some(t), 2.0, 0.0, mid);
        assert!((back.value(mid) - t.value(mid)).abs() < 1e-6);
    }

    #[test]
    fn model_from_config_copies_the_settings() {
        let cfg = Config {
            style: Style::Notch,
            accent: Accent::Pink,
            battery_threshold: 35,
            google_client_id: "id".into(),
            idle_clock: false,
            lyrics: true,
            ..Config::default()
        };
        let m = Model::from_config(&cfg);
        assert_eq!(
            (
                m.style,
                m.accent,
                m.battery_threshold,
                m.google_client_id.as_str(),
                m.idle_clock,
                m.lyrics
            ),
            (Style::Notch, Accent::Pink, 35, "id", false, true)
        );
    }

    #[test]
    fn actions_round_trip_through_lparam() {
        let action = Action::SaveGoogle {
            id: "a".into(),
            secret: "b".into(),
        };
        let lparam = LPARAM(Box::into_raw(Box::new(action.clone())) as isize);
        assert_eq!(unsafe { take_action(lparam) }, Some(action));
        assert_eq!(unsafe { take_action(LPARAM(0)) }, None);
    }
}
