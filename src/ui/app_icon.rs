//! The app logo, embedded by `app.rc` as icon resource 1 (`assets/icon.ico`).

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::data_dir;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::LoadIconWithScaleDown;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::Shell::{
    SHCNE_ASSOCCHANGED, SHCNE_UPDATEITEM, SHCNF_IDLIST, SHCNF_PATHW, SHChangeNotify,
};
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

/// Whether the shell must be told about the exe's icon: the stamp left by the
/// last run does not name this version (or there is none). Whitespace around
/// the stamp is ignored.
pub fn stamp_due(stamp: Option<&str>, version: &str) -> bool {
    stamp.map(str::trim) != Some(version)
}

/// The version recorded in the stamp file; `None` when it is missing or
/// unreadable.
pub fn read_stamp(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Records `version` in the stamp file, creating its folder.
pub fn write_stamp(path: &Path, version: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, version)
}

/// Where the stamp lives: next to the other files in the data folder.
pub fn stamp_path() -> PathBuf {
    data_dir::dir().join("shell-icon.txt")
}

/// Tells the shell that `vale.exe` and the associations changed, so Explorer
/// drops the icon it cached for the exe (the Windows 11 taskbar button shows
/// that icon, not the one set with `WM_SETICON`). Asynchronous (no
/// `SHCNF_FLUSH`), so it never blocks the UI thread; best effort.
pub fn refresh_shell_icon() {
    if let Ok(exe) = std::env::current_exe() {
        let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SHChangeNotify(
                SHCNE_UPDATEITEM,
                SHCNF_PATHW,
                Some(wide.as_ptr().cast()),
                None,
            );
        }
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Runs `refresh_shell_icon` the first time this version starts (a copy of
/// the exe over the old one, without the installer, leaves Explorer's cache
/// stale), then records the version in `stamp`. A stamp that cannot be
/// written only means the shell is told again next start.
pub fn refresh_shell_icon_once(stamp: &Path) {
    let version = env!("CARGO_PKG_VERSION");
    if stamp_due(read_stamp(stamp).as_deref(), version) {
        refresh_shell_icon();
        let _ = write_stamp(stamp, version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vale-app-icon-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn the_shell_is_told_once_per_version() {
        assert!(stamp_due(None, "3.3.0"));
        assert!(stamp_due(Some("3.2.1"), "3.3.0"));
        assert!(stamp_due(Some(""), "3.3.0"));
        assert!(!stamp_due(Some("3.3.0"), "3.3.0"));
        assert!(!stamp_due(Some("  3.3.0\r\n"), "3.3.0"));
    }

    #[test]
    fn the_stamp_is_written_and_read_back() {
        let dir = temp_dir("stamp");
        let path = dir.join("nested").join("shell-icon.txt");
        assert_eq!(read_stamp(&path), None);
        write_stamp(&path, "3.3.0").unwrap();
        assert_eq!(read_stamp(&path).as_deref(), Some("3.3.0"));
        assert!(!stamp_due(read_stamp(&path).as_deref(), "3.3.0"));
        write_stamp(&path, "3.4.0").unwrap();
        assert_eq!(read_stamp(&path).as_deref(), Some("3.4.0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
