//! The Win32 side of the capture: freezing the screen and listing the
//! windows. The process is PerMonitorV2 (app.manifest), so every number
//! here is a physical pixel.
//!
//! Nothing runs until the Screenshot tile starts a capture; nothing is
//! kept after the overlay closes.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmFlush, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW,
    HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetSystemMetrics, GetWindowDisplayAffinity,
    GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE,
    SetWindowDisplayAffinity, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    WINDOW_DISPLAY_AFFINITY, WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use super::{Candidate, Rect};

/// The whole desktop at one moment, in memory only.
pub struct Frozen {
    pub w: u32,
    pub h: u32,
    /// Where the frame's top-left pixel sits in the virtual screen.
    pub origin: (i32, i32),
    /// `w * h * 4` bytes, top-down, alpha 255.
    pub bgra: Vec<u8>,
}

impl Frozen {
    pub fn rect(&self) -> Rect {
        Rect::new(self.origin.0, self.origin.1, self.w as i32, self.h as i32)
    }

    pub fn stride(&self) -> usize {
        self.w as usize * 4
    }
}

/// The virtual screen: every monitor together.
pub fn virtual_screen() -> Rect {
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// The monitor holding (or nearest to) `pt`, and its DPI.
pub fn monitor_at(pt: (i32, i32)) -> (Rect, u32) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: pt.0, y: pt.1 }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let rect = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcMonitor;
            Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
        } else {
            virtual_screen()
        };
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            dx = 96;
        }
        (rect, dx.max(96))
    }
}

/// The windows of this process that are on screen: the island, its halo,
/// the stay-behind helper, an open editor.
fn own_windows() -> Vec<HWND> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            list.push(hwnd);
        }
        true.into()
    }
    let mut list: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM((&raw mut list) as isize));
    }
    list
}

/// Keeps the island and its helper windows out of a screen grab for as long
/// as it lives: `WDA_EXCLUDEFROMCAPTURE` on each, restored to what it was
/// when this drops. A window that refuses is hidden instead and shown again.
struct OutOfTheShot {
    excluded: Vec<(HWND, u32)>,
    hidden: Vec<HWND>,
}

impl OutOfTheShot {
    fn new() -> OutOfTheShot {
        let mut guard = OutOfTheShot {
            excluded: Vec::new(),
            hidden: Vec::new(),
        };
        for hwnd in own_windows() {
            let mut before = 0u32;
            unsafe {
                let _ = GetWindowDisplayAffinity(hwnd, &mut before);
                if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_ok() {
                    guard.excluded.push((hwnd, before));
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    guard.hidden.push(hwnd);
                }
            }
        }
        if !guard.excluded.is_empty() || !guard.hidden.is_empty() {
            // The compositor takes the change on its next frame.
            unsafe {
                let _ = DwmFlush();
            }
            if !guard.hidden.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
        guard
    }
}

impl Drop for OutOfTheShot {
    fn drop(&mut self) {
        unsafe {
            for (hwnd, before) in &self.excluded {
                let _ = SetWindowDisplayAffinity(*hwnd, WINDOW_DISPLAY_AFFINITY(*before));
            }
            for hwnd in &self.hidden {
                let _ = ShowWindow(*hwnd, SW_SHOWNOACTIVATE);
            }
        }
        debug_assert!(WDA_NONE.0 == 0);
    }
}

/// Copies the whole virtual screen into memory (GDI `BitBlt` with
/// `CAPTUREBLT`, so layered windows come along), with this process's own
/// windows left out. `None` when GDI could not make the bitmap.
pub fn grab_virtual_screen() -> Option<Frozen> {
    let area = virtual_screen();
    if area.is_empty() {
        return None;
    }
    let (w, h) = (area.w as usize, area.h as usize);
    let mut bgra = vec![0u8; w * h * 4];
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, area.w, area.h);
        let mut ok = !mem.is_invalid() && !bitmap.is_invalid();
        if ok {
            let old = SelectObject(mem, HGDIOBJ(bitmap.0));
            {
                let _hidden = OutOfTheShot::new();
                ok = BitBlt(
                    mem,
                    0,
                    0,
                    area.w,
                    area.h,
                    Some(screen),
                    area.x,
                    area.y,
                    SRCCOPY | CAPTUREBLT,
                )
                .is_ok();
            }
            // The bitmap must be out of the DC for GetDIBits.
            SelectObject(mem, old);
            if ok {
                let mut info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: area.w,
                        // Negative: top-down rows.
                        biHeight: -area.h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let lines = GetDIBits(
                    mem,
                    bitmap,
                    0,
                    area.h as u32,
                    Some(bgra.as_mut_ptr().cast::<c_void>()),
                    &mut info,
                    DIB_RGB_COLORS,
                );
                ok = lines == area.h;
            }
        }
        if !bitmap.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        if !mem.is_invalid() {
            let _ = DeleteDC(mem);
        }
        ReleaseDC(None, screen);
        if !ok {
            return None;
        }
    }
    // The screen has no alpha; make every pixel opaque.
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(Frozen {
        w: area.w as u32,
        h: area.h as u32,
        origin: (area.x, area.y),
        bgra,
    })
}

/// A top-level window worth snapping to.
#[derive(Clone, Copy, Debug)]
pub struct TopWindow {
    /// Kept as a number so it can go to a worker thread.
    pub hwnd: isize,
    pub rect: Rect,
    /// 0 is the frontmost.
    pub z: u32,
}

impl TopWindow {
    pub fn candidate(&self) -> Candidate {
        Candidate {
            rect: self.rect,
            z: self.z,
            depth: 0,
        }
    }
}

/// The visible top-level windows in z-order, frontmost first. Skipped:
/// invisible, minimised, cloaked (another virtual desktop, a suspended
/// app), zero-size, click-through (`WS_EX_TRANSPARENT`), the desktop
/// itself, and this process's own. Each rect is the visible frame
/// (`DWMWA_EXTENDED_FRAME_BOUNDS`, without the invisible resize border),
/// kept inside the virtual screen.
pub fn top_windows(bounds: Rect) -> Vec<TopWindow> {
    struct Walk {
        bounds: Rect,
        pid: u32,
        out: Vec<TopWindow>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let walk = unsafe { &mut *(lparam.0 as *mut Walk) };
        if let Some(rect) = unsafe { snappable(hwnd, walk.pid, walk.bounds) } {
            let z = walk.out.len() as u32;
            walk.out.push(TopWindow {
                hwnd: hwnd.0 as isize,
                rect,
                z,
            });
        }
        true.into()
    }
    let mut walk = Walk {
        bounds,
        pid: unsafe { GetCurrentProcessId() },
        out: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM((&raw mut walk) as isize));
    }
    walk.out
}

/// The window's frame inside `bounds`, or `None` when it is not something
/// to snap to.
unsafe fn snappable(hwnd: HWND, own_pid: u32, bounds: Rect) -> Option<Rect> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own_pid {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return None;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0 {
            return None;
        }
        let mut name = [0u16; 32];
        let n = GetClassNameW(hwnd, &mut name).max(0) as usize;
        let class = String::from_utf16_lossy(&name[..n.min(name.len())]);
        if is_desktop_class(&class) {
            return None;
        }
        let mut frame = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut frame).cast(),
            size_of::<RECT>() as u32,
        )
        .ok()?;
        let rect = Rect::new(
            frame.left,
            frame.top,
            frame.right - frame.left,
            frame.bottom - frame.top,
        )
        .intersect(&bounds);
        (!rect.is_empty()).then_some(rect)
    }
}

/// The desktop's own windows cover the whole screen and would swallow every
/// click on the wallpaper.
fn is_desktop_class(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_is_not_something_to_snap_to() {
        assert!(is_desktop_class("Progman"));
        assert!(is_desktop_class("WorkerW"));
        assert!(!is_desktop_class("Shell_TrayWnd"));
        assert!(!is_desktop_class("CabinetWClass"));
    }

    #[test]
    fn a_frozen_frame_knows_its_place() {
        let f = Frozen {
            w: 4,
            h: 2,
            origin: (-10, 5),
            bgra: vec![0; 32],
        };
        assert_eq!(f.rect(), Rect::new(-10, 5, 4, 2));
        assert_eq!(f.stride(), 16);
    }
}
