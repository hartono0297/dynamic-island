use std::collections::HashSet;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::Result;

use crate::model::{Event, Notice, NoticeKind};

const PROBE_SECONDS: u32 = 60;
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// Reports which notification ids are new since the previous poll. The first
/// update only records ids, so notifications already in the Action Center
/// when the app starts are not shown.
#[derive(Default)]
pub struct NotificationTracker {
    seen: HashSet<u32>,
    primed: bool,
}

impl NotificationTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the ids in `current` that were absent from the previous update,
    /// in the order given. Ids that disappeared are forgotten.
    pub fn update(&mut self, current: &[u32]) -> Vec<u32> {
        let fresh = if self.primed {
            current
                .iter()
                .copied()
                .filter(|id| !self.seen.contains(id))
                .collect()
        } else {
            Vec::new()
        };
        self.seen = current.iter().copied().collect();
        self.primed = true;
        fresh
    }
}

/// True when any muted entry occurs in `source`, ignoring case.
pub fn is_muted(source: &str, muted: &[String]) -> bool {
    let source = source.to_lowercase();
    muted.iter().any(|m| source.contains(&m.to_lowercase()))
}

/// Polls the Action Center once a second on a background thread and sends a
/// `Event::Notice` for every new toast from an app not in `muted`. Exits
/// quietly when listener access has not been granted.
pub fn spawn_poller(tx: Sender<Event>, wake: impl Fn() + Send + 'static, muted: Vec<String>) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let Ok(listener) = UserNotificationListener::Current() else {
            return;
        };
        if listener.GetAccessStatus().ok() != Some(UserNotificationListenerAccessStatus::Allowed) {
            return;
        }

        let mut tracker = NotificationTracker::new();
        loop {
            if let Ok(fresh) = new_notifications(&listener, &mut tracker) {
                for notice in fresh.iter().filter_map(to_notice) {
                    if is_muted(&notice.source, &muted) {
                        continue;
                    }
                    if tx.send(Event::Notice(notice)).is_err() {
                        return;
                    }
                    wake();
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

/// Brings the app with this AppUserModelID to the front, launching it if needed.
pub fn open_app(app_id: &str) {
    let _ = Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .spawn();
}

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
    let mut tracker = NotificationTracker::new();
    for _ in 0..PROBE_SECONDS {
        match new_notifications(&listener, &mut tracker) {
            Ok(fresh) => {
                for notice in fresh.iter().filter_map(to_notice) {
                    let body = notice.body.replace('\n', " ");
                    println!("{} | {} | {}", notice.source, notice.title, body);
                }
            }
            Err(e) => eprintln!("dynamic-island: reading notifications failed: {e}"),
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn request_access() -> Result<Option<UserNotificationListener>> {
    let listener = UserNotificationListener::Current()?;
    let status = listener.RequestAccessAsync()?.join()?;
    Ok((status == UserNotificationListenerAccessStatus::Allowed).then_some(listener))
}

/// Toasts that appeared since the previous call, in Action Center order.
/// Toasts whose id cannot be read are ignored.
fn new_notifications(
    listener: &UserNotificationListener,
    tracker: &mut NotificationTracker,
) -> Result<Vec<UserNotification>> {
    let current: Vec<(u32, UserNotification)> = listener
        .GetNotificationsAsync(NotificationKinds::Toast)?
        .join()?
        .into_iter()
        .filter_map(|n| n.Id().ok().map(|id| (id, n)))
        .collect();
    let ids: Vec<u32> = current.iter().map(|(id, _)| *id).collect();
    let fresh = tracker.update(&ids);
    Ok(current
        .into_iter()
        .filter(|(id, _)| fresh.contains(id))
        .map(|(_, n)| n)
        .collect())
}

/// Maps a toast to a notice; `None` when the toast is too malformed to show.
/// A toast without text elements is titled with the app name.
fn to_notice(notification: &UserNotification) -> Option<Notice> {
    let key = notification.Id().ok()?.to_string();
    let source = app_name(notification).ok()?;
    let mut texts = text_elements(notification).unwrap_or_default().into_iter();
    let title = texts.next().unwrap_or_else(|| source.clone());
    let body = texts.collect::<Vec<_>>().join("\n");
    let app_id = notification
        .AppInfo()
        .and_then(|info| info.AppUserModelId())
        .ok()
        .map(|id| id.to_string());
    Some(Notice {
        key,
        source,
        title,
        body,
        kind: NoticeKind::App { app_id },
    })
}

fn app_name(notification: &UserNotification) -> Result<String> {
    Ok(notification
        .AppInfo()?
        .DisplayInfo()?
        .DisplayName()?
        .to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_update_primes_without_showing() {
        let mut t = NotificationTracker::new();
        assert!(t.update(&[1, 2, 3]).is_empty());
        assert_eq!(t.update(&[1, 2, 3, 4]), vec![4]);
    }

    #[test]
    fn returns_new_ids_in_given_order() {
        let mut t = NotificationTracker::new();
        t.update(&[1]);
        assert_eq!(t.update(&[1, 7, 5]), vec![7, 5]);
    }

    #[test]
    fn forgets_ids_no_longer_present() {
        let mut t = NotificationTracker::new();
        t.update(&[1, 2]);
        assert!(t.update(&[2]).is_empty());
        assert_eq!(t.seen.len(), 1);
    }

    #[test]
    fn muted_match_is_case_insensitive_substring() {
        let m = vec!["Claude".to_string(), "Orca".to_string()];
        assert!(is_muted("Claude", &m));
        assert!(is_muted("orca agent", &m));
        assert!(!is_muted("WhatsApp", &m));
    }
}
