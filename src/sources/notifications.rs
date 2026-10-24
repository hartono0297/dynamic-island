use std::collections::HashSet;
use std::time::Duration;

use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::Result;

const PROBE_SECONDS: u32 = 60;

/// Diagnostic mode for `--probe-notifications`: requests listener access, then
/// prints every toast that arrives during the next 60 seconds.
pub fn probe() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let listener = match request_access() {
        Ok(Some(listener)) => listener,
        Ok(None) => {
            println!("Izin notifikasi belum diberikan. Buka: ms-settings:privacy-notifications");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("dynamic-island: notification listener unavailable: {e}");
            std::process::exit(1);
        }
    };

    println!("Listening for notifications for {PROBE_SECONDS} s...");
    let mut seen: Option<HashSet<u32>> = None;
    for _ in 0..PROBE_SECONDS {
        match poll(&listener, &mut seen) {
            Ok(lines) => lines.iter().for_each(|line| println!("{line}")),
            Err(e) => eprintln!("dynamic-island: reading notifications failed: {e}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn request_access() -> Result<Option<UserNotificationListener>> {
    let listener = UserNotificationListener::Current()?;
    let status = listener.RequestAccessAsync()?.join()?;
    Ok((status == UserNotificationListenerAccessStatus::Allowed).then_some(listener))
}

/// Returns one line per notification not seen in the previous poll. The first
/// poll only records ids, so notifications already in the Action Center are
/// not reported.
fn poll(listener: &UserNotificationListener, seen: &mut Option<HashSet<u32>>) -> Result<Vec<String>> {
    let notifications = listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    let mut current = HashSet::new();
    let mut lines = Vec::new();
    for notification in &notifications {
        let id = notification.Id()?;
        current.insert(id);
        if seen.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            lines.push(describe(&notification));
        }
    }
    *seen = Some(current);
    Ok(lines)
}

fn describe(notification: &UserNotification) -> String {
    let app = app_name(notification).unwrap_or_else(|_| "?".to_owned());
    let texts = text_elements(notification).unwrap_or_default();
    let title = texts.first().cloned().unwrap_or_default();
    let body = texts.get(1..).unwrap_or_default().join(" ");
    format!("{app} | {title} | {body}")
}

fn app_name(notification: &UserNotification) -> Result<String> {
    Ok(notification.AppInfo()?.DisplayInfo()?.DisplayName()?.to_string())
}

fn text_elements(notification: &UserNotification) -> Result<Vec<String>> {
    let binding = notification
        .Notification()?
        .Visual()?
        .GetBinding(&KnownNotificationBindings::ToastGeneric()?)?;
    binding
        .GetTextElements()?
        .into_iter()
        .map(|text| Ok(text.Text()?.to_string()))
        .collect()
}
