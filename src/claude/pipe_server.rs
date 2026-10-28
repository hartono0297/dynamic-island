use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::HSTRING;

use super::PIPE_NAME;
use super::hook::{HookMessage, to_event};
use crate::model::Event;

const BUFFER_SIZE: u32 = 4096;
/// Longest line accepted from a client; anything past it is dropped.
const MAX_LINE: u64 = 64 * 1024;

/// Listens on `\\.\pipe\vale` on a background thread and turns each
/// hook message into an `Event`. Gives up quietly if another process already
/// owns the pipe.
pub fn spawn_pipe_server(tx: Sender<Event>, wake: impl Fn() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("claude-pipe".to_string())
        .spawn(move || serve(PIPE_NAME, tx, wake));
}

/// Accepts one connection at a time, reading one JSON line from each. Returns
/// when the pipe cannot be created or the receiver is gone.
fn serve(pipe_name: &str, tx: Sender<Event>, wake: impl Fn()) {
    let name = HSTRING::from(pipe_name);
    let Some(mut listening) = create_instance(&name, true) else {
        return;
    };
    loop {
        let connected = wait_for_client(&listening);
        // Open the next instance before reading, so a client arriving now
        // finds a busy pipe rather than a missing one.
        let Some(next) = create_instance(&name, false) else {
            return;
        };
        let client = std::mem::replace(&mut listening, next);
        if !connected {
            continue;
        }
        if let Some(event) = read_event(client) {
            if tx.send(event).is_err() {
                return;
            }
            wake();
        }
    }
}

/// A new inbound instance of the pipe, owned as a `File` so it closes on drop.
/// `first` fails if any process already serves this name.
fn create_instance(name: &HSTRING, first: bool) -> Option<File> {
    let mut open_mode = PIPE_ACCESS_INBOUND;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            name,
            FILE_FLAGS_AND_ATTRIBUTES(open_mode.0),
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER_SIZE,
            BUFFER_SIZE,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        return None;
    }
    Some(unsafe { File::from_raw_handle(handle.0) })
}

/// Blocks until a client opens the instance. True when there may be data to
/// read, including a client that already wrote and closed.
fn wait_for_client(pipe: &File) -> bool {
    match unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
        Ok(()) => true,
        Err(e) => {
            e.code() == ERROR_PIPE_CONNECTED.to_hresult() || e.code() == ERROR_NO_DATA.to_hresult()
        }
    }
}

/// Reads the client's single line. Malformed or oversized input yields `None`.
fn read_event(client: File) -> Option<Event> {
    let mut line = Vec::new();
    BufReader::new(client.take(MAX_LINE))
        .read_until(b'\n', &mut line)
        .ok()?;
    let msg: HookMessage = serde_json::from_slice(&line).ok()?;
    Some(to_event(msg))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::claude::hook::{HookKind, HookMessage, send_line};

    /// Sends `line`, retrying while the server thread is still creating its pipe.
    fn send_when_ready(name: &str, line: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !send_line(name, line) {
            assert!(Instant::now() < deadline, "pipe server never came up");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn round_trip_ignores_malformed_lines() {
        let name = format!(r"\\.\pipe\vale-test-{}", std::process::id());
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let server_name = name.clone();
        std::thread::spawn(move || {
            serve(&server_name, tx, move || {
                let _ = wake_tx.send(());
            })
        });

        send_when_ready(&name, b"not json\n");
        let msg = HookMessage {
            kind: HookKind::Activity,
            session_id: "s9".into(),
            project: "p".into(),
            message: None,
        };
        let mut line = serde_json::to_vec(&msg).unwrap();
        line.push(b'\n');
        send_when_ready(&name, &line);

        let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            event,
            Event::ClaudeActivity {
                session_id: "s9".into()
            }
        );
        assert!(wake_rx.recv_timeout(Duration::from_secs(1)).is_ok());
        assert!(
            rx.try_recv().is_err(),
            "malformed line must not produce an event"
        );
    }
}
