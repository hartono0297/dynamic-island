use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, HWND_TOPMOST, IDC_ARROW, LoadCursorW, RegisterClassExW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WNDCLASSEXW, WNDPROC, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, Result, w};

use super::geometry::{WINDOW_H, WINDOW_W};

/// The island's layered popup: a fixed-size transparent canvas at the
/// top-center of the primary monitor. Content and per-pixel alpha come from
/// `Renderer::draw` via `UpdateLayeredWindow`.
pub struct IslandWindow {
    hwnd: HWND,
    scale: f32,
    origin: POINT,
    size: SIZE,
}

impl IslandWindow {
    pub fn create(wnd_proc: WNDPROC) -> Result<Self> {
        let class_name = w!("DynamicIslandWindow");
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: wnd_proc,
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class_name,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 {
                let error = Error::from_thread();
                if error.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
                    return Err(error);
                }
            }
            let (origin, size, scale) = placement();
            let ex_style: WINDOW_EX_STYLE =
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let hwnd = CreateWindowExW(
                ex_style,
                class_name,
                w!("Dynamic Island"),
                WS_POPUP,
                origin.x,
                origin.y,
                size.cx,
                size.cy,
                None,
                None,
                Some(instance),
                None,
            )?;
            // A layered window stays invisible until its first UpdateLayeredWindow.
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(Self {
                hwnd,
                scale,
                origin,
                size,
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Re-centers on the primary monitor at its current DPI. Call on
    /// `WM_DPICHANGED` and `WM_DISPLAYCHANGE`, then redraw.
    pub fn reposition(&mut self) {
        let (origin, size, scale) = placement();
        self.origin = origin;
        self.size = size;
        self.scale = scale;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                origin.x,
                origin.y,
                size.cx,
                size.cy,
                SWP_NOACTIVATE,
            );
        }
    }

    /// Top-left corner in screen coordinates (physical pixels).
    pub(super) fn origin(&self) -> POINT {
        self.origin
    }

    /// Window size in physical pixels.
    pub(super) fn size(&self) -> SIZE {
        self.size
    }
}

/// Top-center of the primary monitor, sized for that monitor's DPI.
///
/// The DPI is read from the monitor rather than `GetDpiForWindow`: the island
/// always lives on the primary monitor, where the two agree, and asking the
/// monitor avoids moving the window first, which would send `WM_DPICHANGED`
/// back into the caller's window procedure mid-call.
fn placement() -> (POINT, SIZE, f32) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = 96;
        }
        let scale = dpi_x as f32 / 96.0;
        let size = SIZE {
            cx: (WINDOW_W * scale).round() as i32,
            cy: (WINDOW_H * scale).round() as i32,
        };
        let rc = info.rcMonitor;
        let origin = POINT {
            x: rc.left + (rc.right - rc.left - size.cx) / 2,
            y: rc.top,
        };
        (origin, size, scale)
    }
}
