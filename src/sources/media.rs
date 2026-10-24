use std::sync::mpsc::{self, Receiver, Sender};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::Result;

use crate::model::{Event, MediaCommand, MediaInfo};

/// Thumbnails larger than this are dropped rather than held in memory.
const MAX_ART_BYTES: u64 = 2 * 1024 * 1024;

/// What a WinRT event handler reports to the watcher thread. Handlers run on
/// WinRT threads, so they only send one of these and return.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Signal {
    /// The manager's current session changed.
    Session,
    /// The current session's title, artist or thumbnail changed.
    Properties,
    /// The current session's play/pause state changed.
    Playback,
}

/// The work implied by a burst of signals, coalesced so that a flurry of
/// events costs one rebuild.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Refresh {
    /// Re-subscribe to whichever session is now current.
    session: bool,
    /// Re-read title, artist and thumbnail.
    properties: bool,
}

impl Refresh {
    fn add(&mut self, signal: Signal) {
        match signal {
            Signal::Session => {
                self.session = true;
                self.properties = true;
            }
            Signal::Properties => self.properties = true,
            Signal::Playback => {}
        }
    }
}

/// Remembers the last value sent so that repeated events with identical
/// content are not forwarded.
#[derive(Default)]
struct LastSent(Option<Option<MediaInfo>>);

impl LastSent {
    /// True (and remembers `next`) when `next` differs from what was last sent.
    fn changed(&mut self, next: &Option<MediaInfo>) -> bool {
        if self.0.as_ref() == Some(next) {
            return false;
        }
        self.0 = Some(next.clone());
        true
    }
}

/// Title, artist and thumbnail of the current session; re-read only when the
/// session reports new media properties, not on every playback change.
struct Properties {
    title: String,
    artist: String,
    art: Option<Vec<u8>>,
}

/// Handlers registered on one session; removed when dropped.
struct SessionSubscription {
    session: Session,
    properties_token: Option<i64>,
    playback_token: Option<i64>,
}

impl SessionSubscription {
    fn new(session: Session, signals: &Sender<Signal>) -> Self {
        let s = signals.clone();
        let properties_token = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Properties);
                Ok(())
            }))
            .ok();
        let s = signals.clone();
        let playback_token = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Playback);
                Ok(())
            }))
            .ok();
        Self {
            session,
            properties_token,
            playback_token,
        }
    }
}

impl Drop for SessionSubscription {
    fn drop(&mut self) {
        if let Some(token) = self.properties_token {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = self.playback_token {
            let _ = self.session.RemovePlaybackInfoChanged(token);
        }
    }
}

/// State owned by the watcher thread.
struct Watcher {
    manager: Manager,
    manager_token: Option<i64>,
    signals: Sender<Signal>,
    subscription: Option<SessionSubscription>,
    properties: Option<Properties>,
}

impl Watcher {
    fn new(manager: Manager, signals: Sender<Signal>) -> Self {
        let s = signals.clone();
        let manager_token = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = s.send(Signal::Session);
                Ok(())
            }))
            .ok();
        Self {
            manager,
            manager_token,
            signals,
            subscription: None,
            properties: None,
        }
    }

    /// Applies `refresh` and returns what the island should now show.
    fn refresh(&mut self, refresh: Refresh) -> Option<MediaInfo> {
        if refresh.session {
            // Drop the old handlers before subscribing to the new session.
            self.subscription = None;
            self.properties = None;
            self.subscription = self
                .manager
                .GetCurrentSession()
                .ok()
                .map(|session| SessionSubscription::new(session, &self.signals));
        }
        let session = &self.subscription.as_ref()?.session;
        if refresh.properties || self.properties.is_none() {
            // On a failed read keep the previous values for this session.
            if let Ok(properties) = read_properties(session) {
                self.properties = Some(properties);
            }
        }
        let properties = self.properties.as_ref()?;
        Some(MediaInfo {
            title: properties.title.clone(),
            artist: properties.artist.clone(),
            playing: is_playing(session),
            art: properties.art.clone(),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(token) = self.manager_token {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
    }
}

/// Watches the system media session on a background thread and sends
/// `Event::Media` whenever what is playing changes: once at start, then on
/// session, track and play/pause changes. The thread blocks between events.
// Not called outside the debug probe yet; removed when app.rs wires the watcher in (Task 10).
#[allow(dead_code)]
pub fn spawn_watcher(tx: Sender<Event>, wake: impl Fn() + Send + Sync + 'static) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let Ok(manager) = Manager::RequestAsync().and_then(|op| op.join()) else {
            return;
        };
        let (signals, signal_rx) = mpsc::channel();
        let mut watcher = Watcher::new(manager, signals);
        let mut last = LastSent::default();
        let mut refresh = Refresh::default();
        refresh.add(Signal::Session);
        loop {
            let info = watcher.refresh(refresh);
            if last.changed(&info) {
                if tx.send(Event::Media(info)).is_err() {
                    return;
                }
                wake();
            }
            refresh = next_refresh(&signal_rx);
        }
    });
}

/// Blocks for the next signal, then folds in any already queued behind it.
fn next_refresh(signals: &Receiver<Signal>) -> Refresh {
    let mut refresh = Refresh::default();
    // The watcher holds a sender, so `recv` only returns once a signal arrives.
    if let Ok(signal) = signals.recv() {
        refresh.add(signal);
    }
    for signal in signals.try_iter() {
        refresh.add(signal);
    }
    refresh
}

/// Sends a transport command to the current media session on a short-lived
/// thread, so the caller never waits on WinRT. Errors are ignored.
// Not called yet; removed when app.rs handles `Effect::Media` (Task 10).
#[allow(dead_code)]
pub fn send_command(cmd: MediaCommand) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _ = try_send_command(cmd);
    });
}

fn try_send_command(cmd: MediaCommand) -> Result<bool> {
    let session = Manager::RequestAsync()?.join()?.GetCurrentSession()?;
    let op = match cmd {
        MediaCommand::Prev => session.TrySkipPreviousAsync()?,
        MediaCommand::PlayPause => session.TryTogglePlayPauseAsync()?,
        MediaCommand::Next => session.TrySkipNextAsync()?,
    };
    op.join()
}

fn read_properties(session: &Session) -> Result<Properties> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let art = props
        .Thumbnail()
        .ok()
        .and_then(|thumbnail| read_thumbnail(&thumbnail).ok().flatten());
    Ok(Properties {
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        art,
    })
}

/// The thumbnail's encoded image bytes; `None` when empty or over the size cap.
fn read_thumbnail(thumbnail: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = thumbnail.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if !art_size_ok(size) {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(Some(bytes))
}

fn art_size_ok(size: u64) -> bool {
    size > 0 && size <= MAX_ART_BYTES
}

fn is_playing(session: &Session) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .is_ok_and(|status| status == PlaybackStatus::Playing)
}

/// Diagnostic mode for `--probe-media`: prints every media update until Ctrl+C.
#[cfg(debug_assertions)]
pub fn probe() {
    let (tx, rx) = mpsc::channel();
    spawn_watcher(tx, || {});
    println!("Watching media sessions; press Ctrl+C to stop.");
    for event in rx {
        let Event::Media(info) = event else { continue };
        match info {
            None => println!("None"),
            Some(m) => println!(
                "{} | {} | playing={} | art={} bytes",
                m.title,
                m.artist,
                m.playing,
                m.art.as_ref().map_or(0, Vec::len)
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str, playing: bool, art: Option<Vec<u8>>) -> Option<MediaInfo> {
        Some(MediaInfo {
            title: title.to_string(),
            artist: "Artist".to_string(),
            playing,
            art,
        })
    }

    #[test]
    fn first_value_is_always_sent_even_none() {
        let mut last = LastSent::default();
        assert!(last.changed(&None));
        assert!(!last.changed(&None));
    }

    #[test]
    fn identical_info_is_not_resent() {
        let mut last = LastSent::default();
        assert!(last.changed(&info("A", true, Some(vec![1, 2]))));
        assert!(!last.changed(&info("A", true, Some(vec![1, 2]))));
    }

    #[test]
    fn play_state_title_and_late_art_count_as_changes() {
        let mut last = LastSent::default();
        assert!(last.changed(&info("A", true, None)));
        assert!(last.changed(&info("A", false, None)));
        assert!(last.changed(&info("B", false, None)));
        assert!(last.changed(&info("B", false, Some(vec![9]))));
        assert!(last.changed(&None));
    }

    #[test]
    fn playback_signal_alone_does_not_reread_properties() {
        let mut r = Refresh::default();
        r.add(Signal::Playback);
        assert_eq!(r, Refresh::default());
    }

    #[test]
    fn session_change_implies_property_reread() {
        let mut r = Refresh::default();
        r.add(Signal::Playback);
        r.add(Signal::Session);
        assert_eq!(
            r,
            Refresh {
                session: true,
                properties: true
            }
        );
    }

    #[test]
    fn coalesces_queued_signals() {
        let (tx, rx) = mpsc::channel();
        tx.send(Signal::Playback).unwrap();
        tx.send(Signal::Properties).unwrap();
        tx.send(Signal::Playback).unwrap();
        let r = next_refresh(&rx);
        assert_eq!(
            r,
            Refresh {
                session: false,
                properties: true
            }
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn art_size_cap() {
        assert!(!art_size_ok(0));
        assert!(art_size_ok(1));
        assert!(art_size_ok(MAX_ART_BYTES));
        assert!(!art_size_ok(MAX_ART_BYTES + 1));
    }
}
