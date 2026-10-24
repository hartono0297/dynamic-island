use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};

/// True while a fullscreen app, game or presentation is in the foreground.
/// Returns false when the shell cannot be queried.
// Not called outside the debug probe yet; removed when app.rs wires the fullscreen check in (Task 10).
#[allow(dead_code)]
pub fn is_fullscreen() -> bool {
    let Ok(state) = (unsafe { SHQueryUserNotificationState() }) else {
        return false;
    };
    matches!(
        state,
        QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
    )
}
