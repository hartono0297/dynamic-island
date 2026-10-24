use std::collections::HashSet;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::Result;

use crate::model::{Event, Notice, NoticeKind};

const PROBE_SECONDS: u32 = 60;
/// Re-read interval when change events are unavailable.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// Re-read interval when change events are delivered, so a missed event
/// delays a toast by at most this long.
const SAFETY_NET_INTERVAL: Duration = Duration::from_secs(60);

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

/// A `NotificationChanged` handler on the listener; removed when dropped.
struct Subscription {
    listener: UserNotificationListener,
    token: i64,
}

impl Subscription {
    /// Registers a handler that sends one signal per change. Fails for apps
    /// without package identity ("Element not found").
    fn new(listener: &UserNotificationListener, signals: Sender<()>) -> Result<Self> {
        let token = listener.NotificationChanged(&TypedEventHandler::new(move |_, _| {
            let _ = signals.send(());
            Ok(())
        }))?;
        Ok(Self {
            listener: listener.clone(),
            token,
        })
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.listener.RemoveNotificationChanged(self.token);
    }
}

/// How the poller learns that the Action Center may have changed.
enum Trigger {
    /// Change events arrive; re-read on each burst, and every
    /// `SAFETY_NET_INTERVAL` in case one was missed.
    Events {
        _subscription: Subscription,
        signals: Receiver<()>,
        /// At least one event has been delivered since subscribing.
        delivered: bool,
    },
    /// No usable events; re-read every `POLL_INTERVAL`.
    Poll,
}

impl Trigger {
    /// Event mode when the listener accepts a change handler, else polling.
    fn new(listener: &UserNotificationListener) -> Self {
        let (tx, signals) = mpsc::channel();
        match Subscription::new(listener, tx) {
            Ok(subscription) => Trigger::Events {
                _subscription: subscription,
                signals,
                delivered: false,
            },
            Err(_) => Trigger::Poll,
        }
    }

    /// Blocks until the next re-read is due. Returns true when woken by a
    /// change event rather than by the timer.
    fn wait(&mut self) -> bool {
        match self {
            Trigger::Events {
                signals, delivered, ..
            } => {
                let signalled = wait_for_signal(signals, SAFETY_NET_INTERVAL);
                *delivered |= signalled;
                signalled
            }
            Trigger::Poll => {
                std::thread::sleep(POLL_INTERVAL);
                false
            }
        }
    }

    /// Drops to polling when the subscription has never delivered an event
    /// yet a timed re-read found new toasts: the event is not firing.
    fn after_fetch(&mut self, signalled: bool, found_new: bool) {
        if let Trigger::Events { delivered, .. } = self
            && events_not_firing(*delivered, signalled, found_new)
        {
            *self = Trigger::Poll;
        }
    }
}

/// True when a safety-net re-read (not woken by an event) found new toasts
/// although no change event has ever arrived.
fn events_not_firing(delivered: bool, signalled: bool, found_new: bool) -> bool {
    !delivered && !signalled && found_new
}

/// Waits up to `timeout` for a signal, then drains any queued behind it so a
/// burst costs one re-read. True when at least one signal arrived.
fn wait_for_signal(signals: &Receiver<()>, timeout: Duration) -> bool {
    match signals.recv_timeout(timeout) {
        Ok(()) => {
            signals.try_iter().for_each(drop);
            true
        }
        Err(RecvTimeoutError::Timeout) => false,
        // The handler holding the sender is gone; never spin.
        Err(RecvTimeoutError::Disconnected) => {
            std::thread::sleep(timeout);
            false
        }
    }
}

/// Watches the Action Center on a background thread and sends an
/// `Event::Notice` for every new toast from an app not in `muted`. Re-reads
/// on `NotificationChanged` events when the listener supports them, else polls
/// once a second. Exits quietly when listener access has not been granted.
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

        let mut trigger = Trigger::new(&listener);
        let mut tracker = NotificationTracker::new();
        let mut signalled = false;
        loop {
            if let Ok(fresh) = new_notifications(&listener, &mut tracker) {
                trigger.after_fetch(signalled, !fresh.is_empty());
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
            signalled = trigger.wait();
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

    match Subscription::new(&listener, mpsc::channel().0) {
        Ok(_) => println!("Change events: subscribed; the island re-reads on each change."),
        Err(e) => println!("Change events unavailable ({e}); the island polls every second."),
    }
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
    fn signal_burst_is_drained_in_one_wait() {
        let (tx, rx) = mpsc::channel();
        for _ in 0..5 {
            tx.send(()).unwrap();
        }
        assert!(wait_for_signal(&rx, Duration::from_millis(10)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn wait_times_out_without_signal() {
        let (_tx, rx) = mpsc::channel::<()>();
        assert!(!wait_for_signal(&rx, Duration::from_millis(10)));
    }

    #[test]
    fn disconnected_signals_still_wait_the_full_timeout() {
        let (tx, rx) = mpsc::channel::<()>();
        drop(tx);
        let start = std::time::Instant::now();
        assert!(!wait_for_signal(&rx, Duration::from_millis(30)));
        assert!(start.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn falls_back_only_when_timer_finds_toasts_events_never_reported() {
        assert!(events_not_firing(false, false, true));
        assert!(!events_not_firing(true, false, true));
        assert!(!events_not_firing(false, true, true));
        assert!(!events_not_firing(false, false, false));
    }

    #[test]
    fn muted_match_is_case_insensitive_substring() {
        let m = vec!["Claude".to_string(), "Orca".to_string()];
        assert!(is_muted("Claude", &m));
        assert!(is_muted("orca agent", &m));
        assert!(!is_muted("WhatsApp", &m));
    }
}
