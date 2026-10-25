//! The app logo, embedded by `app.rc` as icon resource 1 (`assets/icon.ico`).

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::LoadIconWithScaleDown;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED, LoadImageW, SM_CXSMICON,
    SM_CYSMICON,
};
use windows::core::{PCWSTR, Result};

/// `MAKEINTRESOURCE(1)`, the id `app.rc` gives the icon.
const ICON_ID: PCWSTR = PCWSTR(1 as _);

/// The logo at the small-icon size for `dpi` (16 px at 96 DPI), scaled down
/// from the nearest larger image when the file has no exact match. The
/// caller owns the icon and destroys it with `DestroyIcon`.
pub fn small_icon_for_dpi(dpi: u32) -> Result<HICON> {
    unsafe {
        let cx = GetSystemMetricsForDpi(SM_CXSMICON, dpi);
        let cy = GetSystemMetricsForDpi(SM_CYSMICON, dpi);
        let module: HINSTANCE = GetModuleHandleW(None)?.into();
        LoadIconWithScaleDown(Some(module), ICON_ID, cx, cy)
    }
}

/// Large and small icons for a window class, at system DPI. They are shared
/// (`LR_SHARED`) and must not be destroyed; a null handle on failure gives
/// the class the system default.
pub fn class_icons(instance: HINSTANCE) -> (HICON, HICON) {
    let load = |cx, cy, flags| unsafe {
        LoadImageW(
            Some(instance),
            ICON_ID,
            IMAGE_ICON,
            cx,
            cy,
            LR_SHARED | flags,
        )
        .map(|handle| HICON(handle.0))
        .unwrap_or_default()
    };
    let (sm_cx, sm_cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
    (
        load(0, 0, LR_DEFAULTSIZE),
        load(sm_cx, sm_cy, Default::default()),
    )
}
