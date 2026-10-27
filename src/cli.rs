//! The CLI page's command runner: one command at a time in PowerShell or
//! cmd, without a console window, its stdout and stderr merged into one pipe
//! and streamed line by line to the island from a reader thread. Also the
//! in-app builtins (`cd`, `cls`), the command history, the output buffer and
//! what the panel card shows.
//!
//! A run lives in a Job Object: Stop ends the command with everything it
//! started, and so does quitting the island while it runs. Programs a
//! finished command left behind (a window opened with `start`) are released
//! from the job and keep running.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, TerminateProcess, WaitForSingleObject};
use windows::core::PCWSTR;

use crate::i18n::{self, Key, tr};

/// Lines kept in the output buffer; older ones scroll away.
pub const OUTPUT_CAP: usize = 2000;
/// Commands kept in the history file.
pub const HISTORY_CAP: usize = 100;
/// Output lines of the last command the panel card shows.
pub const CARD_LINES: usize = 4;
/// A line longer than this is cut into pieces (a program printing without
/// newlines must not grow one line forever).
const MAX_LINE: usize = 4096;
/// After the command's process exits, how long its output may still take to
/// arrive. A program it started that keeps the pipe open (a window opened
/// with `start`) would otherwise keep the run "running" until it closes.
const OUTPUT_GRACE: Duration = Duration::from_millis(500);

/// Prefix that makes PowerShell write UTF-8 and leaves out progress bars
/// (they come out as garbage on a pipe).
const PS_PREFIX: &str =
    "[Console]::OutputEncoding=[Text.Encoding]::UTF8; $ProgressPreference='SilentlyContinue'; ";

/// The shell commands run in; island-owned setting `cli_shell`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shell {
    #[default]
    PowerShell,
    Cmd,
}

impl Shell {
    pub const ALL: [Shell; 2] = [Shell::PowerShell, Shell::Cmd];

    /// Its name in config.json and on screen.
    pub fn name(self) -> &'static str {
        match self {
            Shell::PowerShell => "PowerShell",
            Shell::Cmd => "cmd",
        }
    }

    /// Case-insensitive.
    pub fn from_name(name: &str) -> Option<Shell> {
        Shell::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }
}

impl Serialize for Shell {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// An unknown name (a typo in config.json) is the default shell rather than
/// an invalid config.
impl<'de> Deserialize<'de> for Shell {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Ok(Shell::from_name(&name).unwrap_or_default())
    }
}

/// From a run's threads to the island.
#[derive(Clone, Debug, PartialEq)]
pub enum CliEvent {
    /// Lines the command printed, in order (stdout and stderr merged).
    Output { run: u64, lines: Vec<String> },
    /// The command ended: its exit code (`None` if unknown), and whether
    /// Stop ended it.
    Exit {
        run: u64,
        code: Option<i32>,
        killed: bool,
    },
}

/// Where a run's events go; called from its threads.
pub type Sink = Arc<dyn Fn(CliEvent) + Send + Sync>;

/// The program and the arguments that run `cmd` in `shell`.
#[derive(Debug, PartialEq)]
pub struct Invocation {
    pub program: PathBuf,
    /// Passed as separate, quoted arguments.
    pub args: Vec<String>,
    /// Appended to the command line exactly as it is (cmd's own quoting).
    pub raw: Option<String>,
    /// An environment variable for the shell.
    pub env: Option<(&'static str, String)>,
}

/// Holds the command for cmd (see `invocation`).
const CMD_VAR: &str = "DI_CLI_CMD";

/// PowerShell: `-Command` with the UTF-8 prefix (each argument quoted by
/// the standard rules, which PowerShell undoes).
///
/// cmd: `chcp 65001`, then a second cmd runs the command. cmd reads the code
/// page once at start, so in a single `chcp 65001 & cmd` line its own output
/// (`echo`, `dir`) would still be ANSI/OEM; the inner cmd starts after the
/// switch. The command reaches it through `DI_CLI_CMD` and delayed
/// expansion, which the outer cmd does after parsing, so `&`, `|`, quotes
/// and `!` arrive exactly as typed; with `/s` the inner cmd drops only the
/// quotes put around it.
pub fn invocation(shell: Shell, cmd: &str) -> Invocation {
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32"));
    let at = |rel: &str, bare: &str| {
        system
            .as_ref()
            .map(|dir| dir.join(rel))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from(bare))
    };
    match shell {
        Shell::PowerShell => Invocation {
            program: at("WindowsPowerShell\\v1.0\\powershell.exe", "powershell.exe"),
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!("{PS_PREFIX}{cmd}"),
            ],
            raw: None,
            env: None,
        },
        Shell::Cmd => Invocation {
            program: at("cmd.exe", "cmd.exe"),
            args: Vec::new(),
            raw: Some(format!(
                "/d /v:on /s /c \"chcp 65001>nul & cmd /d /s /c \"!{CMD_VAR}!\"\""
            )),
            env: Some((CMD_VAR, cmd.to_owned())),
        },
    }
}

/// A running command. Dropping it while the command still runs ends it
/// (with everything it started); dropping it after the command ended lets
/// programs it left behind go on.
pub struct RunHandle {
    /// `None` if the job could not be set up; Stop then ends only the
    /// command's own process.
    job: Option<OwnedHandle>,
    process: OwnedHandle,
    killed: Arc<AtomicBool>,
}

impl RunHandle {
    /// Ends the command and everything it started. Its `Exit` event follows
    /// with `killed: true`.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        unsafe {
            let ended = match &self.job {
                Some(job) => TerminateJobObject(raw(job), 1).is_ok(),
                None => false,
            };
            if !ended {
                let _ = TerminateProcess(raw(&self.process), 1);
            }
        }
    }

    fn exited(&self) -> bool {
        unsafe { WaitForSingleObject(raw(&self.process), 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        // The command is over: what it started may outlive the job handle.
        if self.exited()
            && let Some(job) = &self.job
        {
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            unsafe {
                let _ = SetInformationJobObject(
                    raw(job),
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
        }
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// A job that ends its processes when its last handle closes.
pub(crate) fn kill_on_close_job() -> Option<OwnedHandle> {
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
        let job = OwnedHandle::from_raw_handle(job.0);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        Some(job)
    }
}

/// Starts `cmd` in `shell` in `cwd`, without a window. Output lines and then
/// the exit go to `sink` as events tagged `id`.
pub fn spawn(id: u64, cmd: &str, cwd: &Path, shell: Shell, sink: Sink) -> io::Result<RunHandle> {
    let inv = invocation(shell, cmd);
    let (mut reader, writer) = io::pipe()?;
    let mut command = Command::new(&inv.program);
    command.args(&inv.args);
    if let Some(raw) = &inv.raw {
        command.raw_arg(raw);
    }
    if let Some((name, value)) = &inv.env {
        command.env(name, value);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .creation_flags(CREATE_NO_WINDOW.0);
    let mut child = command.spawn()?;
    // The pipe's write end must close here too, or the reader never ends.
    drop(command);
    let process = child.as_handle().try_clone_to_owned()?;
    let job = kill_on_close_job()
        .filter(|job| unsafe { AssignProcessToJobObject(raw(job), raw(&process)).is_ok() });
    let killed = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let out = sink.clone();
    std::thread::Builder::new()
        .name("cli-output".into())
        .spawn(move || {
            let mut split = LineSplitter::default();
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let lines = split.push(&buf[..n]);
                        if !lines.is_empty() {
                            out(CliEvent::Output { run: id, lines });
                        }
                    }
                }
            }
            if let Some(last) = split.finish() {
                out(CliEvent::Output {
                    run: id,
                    lines: vec![last],
                });
            }
            let _ = done_tx.send(());
        })?;

    let stopped = killed.clone();
    std::thread::Builder::new()
        .name("cli-wait".into())
        .spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            // Normally the output ends with the process; see `OUTPUT_GRACE`.
            let _ = done_rx.recv_timeout(OUTPUT_GRACE);
            sink(CliEvent::Exit {
                run: id,
                code,
                killed: stopped.load(Ordering::SeqCst),
            });
        })?;

    Ok(RunHandle {
        job,
        process,
        killed,
    })
}

/// Splits a byte stream into lines: `\n` or `\r\n` ends one, UTF-8 is
/// decoded lossily, and `clean_line` tidies each.
#[derive(Default)]
pub struct LineSplitter {
    partial: Vec<u8>,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                lines.push(clean_line(&self.partial));
                self.partial.clear();
            } else {
                self.partial.push(b);
                if self.partial.len() >= MAX_LINE {
                    lines.push(clean_line(&self.partial));
                    self.partial.clear();
                }
            }
        }
        lines
    }

    /// The last line, when the output did not end with a newline.
    pub fn finish(&mut self) -> Option<String> {
        if self.partial.is_empty() {
            return None;
        }
        let line = clean_line(&self.partial);
        self.partial.clear();
        Some(line)
    }
}

/// One line as shown: lossy UTF-8, without the `\r` of `\r\n`, a byte order
/// mark, or ANSI escape sequences (colours) the window cannot show.
pub fn clean_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_suffix('\r').unwrap_or(&text);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{feff}' => {}
            '\u{1b}' => {
                // CSI: ESC [ parameters final-byte; other escapes: ESC + one char.
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Commands the island handles itself.
#[derive(Clone, Debug, PartialEq)]
pub enum Builtin {
    /// `cd`, `cd path`, `chdir path`, `cd /d path`, `cd..`; `None` = home.
    Cd(Option<String>),
    /// `cls`, `clear`, `Clear-Host`.
    Clear,
}

pub fn builtin(input: &str) -> Option<Builtin> {
    let input = input.trim();
    let lower = input.to_ascii_lowercase();
    if matches!(lower.as_str(), "cls" | "clear" | "clear-host") {
        return Some(Builtin::Clear);
    }
    // cmd also takes `cd..` and `cd\` without a space.
    for short in ["cd..", "cd\\", "cd/"] {
        if lower.starts_with(short) && !lower[short.len()..].starts_with(char::is_alphanumeric) {
            return Some(Builtin::Cd(Some(input[2..].trim().to_owned())));
        }
    }
    let (word, rest) = match input.find(char::is_whitespace) {
        Some(i) => (&input[..i], input[i..].trim()),
        None => (input, ""),
    };
    if !word.eq_ignore_ascii_case("cd") && !word.eq_ignore_ascii_case("chdir") {
        return None;
    }
    let rest = rest
        .strip_prefix("/d ")
        .or_else(|| rest.strip_prefix("/D "))
        .map_or(rest, str::trim);
    let rest = rest
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(rest)
        .trim();
    Some(Builtin::Cd((!rest.is_empty()).then(|| rest.to_owned())))
}

/// The folder `cd arg` goes to from `cwd`: home for no argument or `~`,
/// `~\x` under home, `X:` the drive's root, anything else relative to `cwd`
/// (or absolute). `Err` carries the argument when it is not a folder.
pub fn resolve_cd(cwd: &Path, arg: Option<&str>, home: &Path) -> Result<PathBuf, String> {
    let target = match arg {
        None | Some("~") => home.to_path_buf(),
        Some(a) if a.starts_with("~\\") || a.starts_with("~/") => home.join(&a[2..]),
        Some(a) if a.len() == 2 && a.ends_with(':') && a.as_bytes()[0].is_ascii_alphabetic() => {
            PathBuf::from(format!("{a}\\"))
        }
        Some(a) => cwd.join(a),
    };
    let shown = arg.unwrap_or("~").to_owned();
    match std::fs::canonicalize(&target) {
        Ok(path) if path.is_dir() => Ok(plain_path(path)),
        _ => Err(shown),
    }
}

/// `\\?\C:\x` → `C:\x` and `\\?\UNC\server\share` → `\\server\share`:
/// cmd refuses a verbatim working directory.
pub fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        return PathBuf::from(rest);
    }
    path
}

/// `%USERPROFILE%`, else the current directory.
pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `%APPDATA%\DynamicIsland\cli-history.txt`.
pub fn history_path() -> PathBuf {
    crate::config::path().with_file_name("cli-history.txt")
}

/// `%APPDATA%\DynamicIsland\cli-cwd.txt`: the folder `cd` last went to.
pub fn cwd_path() -> PathBuf {
    crate::config::path().with_file_name("cli-cwd.txt")
}

/// The saved working directory if it still exists, else home.
pub fn load_cwd(path: &Path) -> PathBuf {
    std::fs::read_to_string(path)
        .ok()
        .map(|t| PathBuf::from(t.trim()))
        .filter(|p| !p.as_os_str().is_empty() && p.is_dir())
        .unwrap_or_else(home)
}

pub fn save_cwd(path: &Path, cwd: &Path) -> io::Result<()> {
    write_atomic(path, &cwd.to_string_lossy())
}

/// A sibling `.tmp` file renamed over `path`. Also writes the folder
/// menu's lists (`cli_folders`).
pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Commands run, oldest first: at most `HISTORY_CAP`, no command twice in a row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    items: Vec<String>,
}

impl History {
    /// Bytes that are not UTF-8 (a file saved by another editor) become
    /// U+FFFD rather than an empty history the next save would write over.
    pub fn load(path: &Path) -> Self {
        let mut h = History::default();
        if let Ok(bytes) = std::fs::read(path) {
            for line in String::from_utf8_lossy(&bytes).lines() {
                h.push(line);
            }
        }
        h
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = self.items.join("\n");
        text.push('\n');
        write_atomic(path, &text)
    }

    /// Adds `cmd` unless it is blank or the same as the last one.
    pub fn push(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() || self.items.last().is_some_and(|l| l == cmd) {
            return;
        }
        self.items.push(cmd.to_owned());
        if self.items.len() > HISTORY_CAP {
            let extra = self.items.len() - HISTORY_CAP;
            self.items.drain(..extra);
        }
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }
}

/// Up / Down through the history in the input line. Leaving the newest
/// entry with Down gives back what was typed before Up.
#[derive(Clone, Debug, Default)]
pub struct HistoryCursor {
    pos: Option<usize>,
    draft: String,
}

impl HistoryCursor {
    /// The older entry to show, or `None` at the oldest (nothing changes).
    pub fn up(&mut self, items: &[String], typed: &str) -> Option<String> {
        let next = match self.pos {
            None if !items.is_empty() => {
                self.draft = typed.to_owned();
                items.len() - 1
            }
            Some(p) if p > 0 => p - 1,
            _ => return None,
        };
        self.pos = Some(next);
        Some(items[next].clone())
    }

    /// The newer entry, the typed draft past the newest, or `None` when not
    /// browsing.
    pub fn down(&mut self, items: &[String]) -> Option<String> {
        let p = self.pos?;
        if p + 1 < items.len() {
            self.pos = Some(p + 1);
            Some(items[p + 1].clone())
        } else {
            self.pos = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    pub fn reset(&mut self) {
        self.pos = None;
        self.draft.clear();
    }
}

/// The output buffer: the last `OUTPUT_CAP` lines.
#[derive(Clone, Debug, Default)]
pub struct Output {
    lines: VecDeque<String>,
    /// Lines ever pushed (`clear` does not reset it): what the window has
    /// not shown yet is the difference to the count it last saw.
    total: u64,
}

impl Output {
    pub fn push(&mut self, line: String) {
        self.lines.push_back(line);
        self.total += 1;
        if self.lines.len() > OUTPUT_CAP {
            self.lines.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn lines(&self) -> &VecDeque<String> {
        &self.lines
    }

    /// The whole buffer for an EDIT control (`\r\n` line breaks).
    // The CLI window that read it is gone (the panel draws `lines`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn text(&self) -> String {
        let mut text = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                text.push_str("\r\n");
            }
            text.push_str(line);
        }
        text
    }
}

/// What the window must do to show the output as it is now.
// Unused since the CLI moved into the panel, which draws the buffer itself.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
pub enum SyncStep {
    Nothing,
    /// Add these lines at the end.
    Append(Vec<String>),
    /// Show the whole buffer instead.
    Reset,
}

/// What the CLI window has shown of the output. Lines are appended; once it
/// holds a few hundred more than the buffer (old ones scrolled out of the
/// buffer but not out of the window), it is given the whole buffer again,
/// so the window never holds much more than `OUTPUT_CAP` lines.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Default)]
pub struct WindowSync {
    /// `Output::total` when last synced.
    seen: u64,
    /// Lines the window holds.
    held: usize,
}

#[cfg_attr(not(test), allow(dead_code))]
impl WindowSync {
    /// The window now shows the whole buffer (just opened, or after `cls`).
    pub fn reset(&mut self, output: &Output) {
        self.seen = output.total();
        self.held = output.lines().len();
    }

    pub fn next(&mut self, output: &Output) -> SyncStep {
        let new = (output.total() - self.seen) as usize;
        if new == 0 {
            return SyncStep::Nothing;
        }
        let lines = output.lines();
        if new > lines.len() || self.held + new > lines.len() + OUTPUT_CAP / 4 {
            self.reset(output);
            return SyncStep::Reset;
        }
        self.seen = output.total();
        self.held += new;
        SyncStep::Append(lines.iter().skip(lines.len() - new).cloned().collect())
    }
}

/// `[exit 0]`, or `[dihentikan]` / `[stopped]` after Stop.
pub fn exit_line(code: Option<i32>, killed: bool) -> String {
    if killed {
        return format!("[{}]", tr(Key::CliStopped));
    }
    match code {
        Some(c) => format!("[exit {c}]"),
        None => "[exit ?]".into(),
    }
}

/// The echo of a command in the output.
pub fn echo_line(cmd: &str) -> String {
    format!("> {cmd}")
}

/// What the island keeps of the CLI: the output, the last command and its
/// last lines (the panel card), and which run is going.
#[derive(Default)]
pub struct Session {
    pub output: Output,
    last_command: Option<String>,
    tail: VecDeque<String>,
    running: Option<u64>,
}

impl Session {
    /// A command was entered: echoed, and the card follows it. While another
    /// runs, only the output gets it: the card stays with the running one.
    pub fn begin(&mut self, cmd: &str) {
        self.output.push(echo_line(cmd));
        if self.running.is_none() {
            self.last_command = Some(cmd.to_owned());
            self.tail.clear();
        }
    }

    /// The command `begin` echoed now runs as `run`.
    pub fn started(&mut self, run: u64) {
        self.running = Some(run);
    }

    /// A line from the island itself (a builtin's answer, an error); on the
    /// card only while nothing runs.
    pub fn note(&mut self, line: String) {
        if self.running.is_some() {
            self.output.push(line);
        } else {
            self.add(line);
        }
    }

    /// Lines of `run`; ignored (false) unless it is the one running.
    pub fn lines(&mut self, run: u64, lines: Vec<String>) -> bool {
        if self.running != Some(run) {
            return false;
        }
        for line in lines {
            self.add(line);
        }
        true
    }

    /// `run` ended; ignored (false) unless it is the one running.
    pub fn exit(&mut self, run: u64, code: Option<i32>, killed: bool) -> bool {
        if self.running != Some(run) {
            return false;
        }
        self.running = None;
        self.add(exit_line(code, killed));
        true
    }

    /// `cls`: the output goes, and the card too unless a command runs (its
    /// lines keep coming to the card).
    pub fn clear(&mut self) {
        self.output.clear();
        if self.running.is_none() {
            self.last_command = None;
            self.tail.clear();
        }
    }

    pub fn running(&self) -> Option<u64> {
        self.running
    }

    // The panel card that showed these is now the full terminal; the
    // notch's run status (v3 Run 8) is their next reader.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn last_command(&self) -> Option<&str> {
        self.last_command.as_deref()
    }

    /// The last command's last `CARD_LINES` lines, oldest first.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn tail(&self) -> impl Iterator<Item = &str> {
        self.tail.iter().map(String::as_str)
    }

    fn add(&mut self, line: String) {
        self.tail.push_back(line.clone());
        while self.tail.len() > CARD_LINES {
            self.tail.pop_front();
        }
        self.output.push(line);
    }
}

/// "cd: folder tidak ada: x".
pub fn cd_error(arg: &str) -> String {
    i18n::cd_not_found(i18n::lang(), arg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// A fresh directory under the system temp dir; removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::AtomicU32;
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "dynamic-island-cli-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(plain_path(std::fs::canonicalize(&dir).unwrap()))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Runs `cmd` to the end and returns its lines and exit event.
    fn run_in(shell: Shell, cmd: &str, cwd: &Path) -> (Vec<String>, Option<i32>, bool) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Sink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let handle = spawn(7, cmd, cwd, shell, sink).expect("spawn");
        let mut lines = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("event") {
                CliEvent::Output { run, lines: more } => {
                    assert_eq!(run, 7);
                    lines.extend(more);
                }
                CliEvent::Exit { run, code, killed } => {
                    assert_eq!(run, 7);
                    drop(handle);
                    return (lines, code, killed);
                }
            }
        }
    }

    fn run(shell: Shell, cmd: &str) -> (Vec<String>, Option<i32>, bool) {
        let dir = TempDir::new("run");
        run_in(shell, cmd, &dir.0)
    }

    #[test]
    fn echo_in_powershell() {
        let (lines, code, killed) = run(Shell::PowerShell, "Write-Output 'halo dunia'");
        assert_eq!(lines, ["halo dunia"]);
        assert_eq!((code, killed), (Some(0), false));
    }

    #[test]
    fn echo_in_cmd() {
        let (lines, code, _) = run(Shell::Cmd, "echo halo dunia");
        assert_eq!(lines, ["halo dunia"]);
        assert_eq!(code, Some(0));
    }

    #[test]
    fn quotes_reach_powershell_intact() {
        let (lines, _, _) = run(Shell::PowerShell, r#"Write-Output "a ""b"" c" 'd e'"#);
        assert_eq!(lines, [r#"a "b" c"#, "d e"]);
    }

    #[test]
    fn cmd_runs_pipes_and_ampersands_as_typed() {
        let (lines, _, _) = run(Shell::Cmd, r#"echo "x y" & echo one| findstr one"#);
        assert_eq!(lines, [r#""x y" "#, "one"]);
    }

    #[test]
    fn many_lines_keep_their_order() {
        let (lines, _, _) = run(Shell::PowerShell, "1..300 | ForEach-Object { \"line $_\" }");
        let want: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
        assert_eq!(lines, want);
        let (lines, _, _) = run(Shell::Cmd, "for /L %i in (1,1,50) do @echo line %i");
        let want: Vec<String> = (1..=50).map(|i| format!("line {i}")).collect();
        assert_eq!(lines, want);
    }

    #[test]
    fn stderr_is_captured() {
        let (lines, _, _) = run(Shell::Cmd, "echo oops 1>&2");
        assert_eq!(lines, ["oops "]);
        let (lines, _, _) = run(
            Shell::PowerShell,
            "[Console]::Error.WriteLine('oops'); Write-Output fine",
        );
        assert!(lines.contains(&"oops".to_owned()), "{lines:?}");
        assert!(lines.contains(&"fine".to_owned()), "{lines:?}");
    }

    #[test]
    fn exit_codes_come_back() {
        assert_eq!(run(Shell::Cmd, "exit /b 3").1, Some(3));
        assert_eq!(run(Shell::PowerShell, "exit 4").1, Some(4));
        // A failing PowerShell command: its error text shows, exit 1.
        let (lines, code, _) = run(Shell::PowerShell, "Get-Item 'Z:\\tidak\\ada'");
        assert_eq!(code, Some(1));
        assert!(!lines.is_empty());
    }

    #[test]
    fn output_is_utf8_in_both_shells() {
        let (lines, _, _) = run(Shell::PowerShell, "Write-Output 'héllo ✓ 日本'");
        assert_eq!(lines, ["héllo ✓ 日本"]);
        let (lines, _, _) = run(Shell::Cmd, "echo héllo ✓");
        assert_eq!(lines, ["héllo ✓"]);
        // cmd's own listing too, not only echo.
        let dir = TempDir::new("utf8");
        std::fs::write(dir.0.join("✓é.txt"), "x").unwrap();
        let (lines, _, _) = run_in(Shell::Cmd, "dir /b", &dir.0);
        assert_eq!(lines, ["✓é.txt"]);
    }

    #[test]
    fn cmd_takes_the_command_exactly_as_typed() {
        let (lines, _, _) = run(Shell::Cmd, "echo \"a&b\" ^& c! 100%");
        assert_eq!(lines, ["\"a&b\" & c! 100%"]);
        let (lines, _, _) = run(Shell::Cmd, "for /L %i in (1,1,3) do @echo n%i");
        assert_eq!(lines, ["n1", "n2", "n3"]);
        let (lines, _, _) = run(Shell::Cmd, "echo %OS%");
        assert_eq!(lines, ["Windows_NT"]);
        // A command that starts with a quoted program path.
        let (lines, code, _) = run(
            Shell::Cmd,
            "\"C:\\Windows\\System32\\cmd.exe\" /d /c echo inner",
        );
        assert_eq!((lines, code), (vec!["inner".to_owned()], Some(0)));
    }

    #[test]
    fn runs_in_the_given_folder() {
        let dir = TempDir::new("cwd");
        let (lines, _, _) = run_in(Shell::Cmd, "cd", &dir.0);
        assert_eq!(lines, [dir.0.to_string_lossy().to_string()]);
        let (lines, _, _) = run_in(Shell::PowerShell, "(Get-Location).Path", &dir.0);
        assert_eq!(lines, [dir.0.to_string_lossy().to_string()]);
    }

    fn kill_after_start(shell: Shell, cmd: &str) {
        let dir = TempDir::new("kill");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Sink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let handle = spawn(9, cmd, &dir.0, shell, sink).expect("spawn");
        std::thread::sleep(Duration::from_millis(800));
        let t0 = Instant::now();
        handle.kill();
        let exit = loop {
            match rx
                .recv_timeout(Duration::from_secs(10))
                .expect("exit after kill")
            {
                CliEvent::Exit { killed, .. } => break killed,
                CliEvent::Output { .. } => {}
            }
        };
        assert!(exit, "marked as stopped");
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    }

    #[test]
    fn exit_is_reported_while_a_started_program_holds_the_output() {
        // `start /b` leaves ping behind with the pipe as its stderr.
        let t0 = Instant::now();
        let (lines, code, killed) = run(
            Shell::Cmd,
            "start \"\" /b ping -n 4 127.0.0.1 >nul & echo started",
        );
        assert_eq!(
            (lines, code, killed),
            (vec!["started".to_owned()], Some(0), false)
        );
        assert!(
            t0.elapsed() < Duration::from_millis(2500),
            "{:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn kill_stops_a_long_powershell_command() {
        kill_after_start(Shell::PowerShell, "Start-Sleep 20");
    }

    #[test]
    fn kill_stops_a_long_cmd_command_and_its_children() {
        // ping is a child of cmd: the job ends both, so the pipe closes.
        kill_after_start(Shell::Cmd, "ping -n 20 127.0.0.1");
    }

    #[test]
    fn invocations_follow_the_pinned_command_lines() {
        let ps = invocation(Shell::PowerShell, "dir");
        assert!(ps.program.ends_with("powershell.exe"));
        assert_eq!(
            ps.args[..4],
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
        );
        assert!(ps.args[4].starts_with("[Console]::OutputEncoding=[Text.Encoding]::UTF8;"));
        assert!(ps.args[4].ends_with("; dir"));
        assert_eq!((ps.raw, ps.env), (None, None));
        let cmd = invocation(Shell::Cmd, "dir /b");
        assert!(cmd.program.ends_with("cmd.exe"));
        assert!(cmd.args.is_empty());
        assert_eq!(
            cmd.raw.as_deref(),
            Some("/d /v:on /s /c \"chcp 65001>nul & cmd /d /s /c \"!DI_CLI_CMD!\"\"")
        );
        assert_eq!(cmd.env, Some(("DI_CLI_CMD", "dir /b".to_owned())));
    }

    #[test]
    fn lines_split_across_chunks_and_line_endings() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"one\r\ntw"), ["one"]);
        assert_eq!(s.push(b"o\nthree"), ["two"]);
        assert_eq!(s.finish(), Some("three".into()));
        assert_eq!(s.finish(), None);
        // A UTF-8 character split between two reads stays whole.
        let bytes = "é\n".as_bytes();
        assert!(s.push(&bytes[..1]).is_empty());
        assert_eq!(s.push(&bytes[1..]), ["é"]);
        // Endless output without a newline is cut.
        let long = vec![b'x'; MAX_LINE + 10];
        let lines = s.push(&long);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_LINE);
        assert_eq!(s.finish().map(|l| l.len()), Some(10));
    }

    #[test]
    fn lines_lose_bom_colours_and_stray_returns() {
        assert_eq!(clean_line("\u{feff}hi".as_bytes()), "hi");
        assert_eq!(clean_line(b"\x1b[31mred\x1b[0m done"), "red done");
        assert_eq!(clean_line(b"50%\r100%\r"), "50%100%");
        assert_eq!(clean_line(b"tab\there"), "tab\there");
        assert_eq!(clean_line(&[0x66, 0xFF, 0x6F]), "f\u{fffd}o");
    }

    #[test]
    fn builtins_are_recognised() {
        let cd = |s: Option<&str>| Some(Builtin::Cd(s.map(Into::into)));
        assert_eq!(builtin("cd"), cd(None));
        assert_eq!(builtin("  CD  "), cd(None));
        assert_eq!(builtin("cd .."), cd(Some("..")));
        assert_eq!(builtin("cd.."), cd(Some("..")));
        assert_eq!(builtin("cd\\"), cd(Some("\\")));
        assert_eq!(builtin("cd ~"), cd(Some("~")));
        assert_eq!(builtin("chdir D:\\Kerja"), cd(Some("D:\\Kerja")));
        assert_eq!(builtin("cd /d D:\\Kerja"), cd(Some("D:\\Kerja")));
        assert_eq!(
            builtin("cd \"C:\\Program Files\""),
            cd(Some("C:\\Program Files"))
        );
        assert_eq!(builtin("cd 'C:\\a b'"), cd(Some("C:\\a b")));
        assert_eq!(builtin("cls"), Some(Builtin::Clear));
        assert_eq!(builtin("Clear"), Some(Builtin::Clear));
        assert_eq!(builtin("clear-host"), Some(Builtin::Clear));
        for not in ["cdx", "dir", "cd..x", "echo cd", "clsx", "cls /?x"] {
            assert_eq!(builtin(not), None, "{not}");
        }
    }

    #[test]
    fn cd_resolves_relative_absolute_home_and_parent() {
        let root = TempDir::new("cd");
        let a = root.0.join("a");
        let b = a.join("b c");
        std::fs::create_dir_all(&b).unwrap();
        let home = root.0.join("home");
        std::fs::create_dir_all(home.join("docs")).unwrap();

        assert_eq!(resolve_cd(&root.0, Some("a"), &home), Ok(a.clone()));
        assert_eq!(resolve_cd(&a, Some("b c"), &home), Ok(b.clone()));
        assert_eq!(resolve_cd(&b, Some(".."), &home), Ok(a.clone()));
        assert_eq!(resolve_cd(&b, Some("..\\.."), &home), Ok(root.0.clone()));
        let abs = b.to_string_lossy().to_string();
        assert_eq!(resolve_cd(&home, Some(&abs), &home), Ok(b.clone()));
        assert_eq!(resolve_cd(&a, None, &home), Ok(home.clone()));
        assert_eq!(resolve_cd(&a, Some("~"), &home), Ok(home.clone()));
        assert_eq!(
            resolve_cd(&a, Some("~\\docs"), &home),
            Ok(home.join("docs"))
        );
        // Never a verbatim `\\?\` path.
        let drive = &abs[..2];
        let got = resolve_cd(&a, Some(drive), &home).unwrap();
        assert_eq!(got, PathBuf::from(format!("{drive}\\")));
    }

    #[test]
    fn cd_refuses_missing_folders_and_files() {
        let root = TempDir::new("cd-err");
        std::fs::write(root.0.join("file.txt"), "x").unwrap();
        assert_eq!(
            resolve_cd(&root.0, Some("nope"), &root.0),
            Err("nope".into())
        );
        assert_eq!(
            resolve_cd(&root.0, Some("file.txt"), &root.0),
            Err("file.txt".into())
        );
    }

    #[test]
    fn verbatim_paths_become_plain() {
        assert_eq!(
            plain_path(PathBuf::from("\\\\?\\C:\\Users")),
            PathBuf::from("C:\\Users")
        );
        assert_eq!(
            plain_path(PathBuf::from("\\\\?\\UNC\\srv\\share")),
            PathBuf::from("\\\\srv\\share")
        );
        assert_eq!(plain_path(PathBuf::from("D:\\x")), PathBuf::from("D:\\x"));
    }

    #[test]
    fn saved_cwd_is_used_only_while_it_exists() {
        let root = TempDir::new("cwd-file");
        let file = root.0.join("cli-cwd.txt");
        assert_eq!(load_cwd(&file), home());
        let work = root.0.join("work");
        std::fs::create_dir_all(&work).unwrap();
        save_cwd(&file, &work).unwrap();
        assert_eq!(load_cwd(&file), work);
        std::fs::remove_dir_all(&work).unwrap();
        assert_eq!(load_cwd(&file), home());
    }

    #[test]
    fn history_skips_repeats_and_keeps_the_last_hundred() {
        let mut h = History::default();
        for cmd in ["dir", "dir", "  ", "git status", "dir", "dir"] {
            h.push(cmd);
        }
        assert_eq!(h.items(), ["dir", "git status", "dir"]);
        for i in 0..150 {
            h.push(&format!("echo {i}"));
        }
        assert_eq!(h.items().len(), HISTORY_CAP);
        assert_eq!(h.items()[0], "echo 50");
        assert_eq!(h.items()[99], "echo 149");
    }

    #[test]
    fn history_round_trips_through_its_file() {
        let root = TempDir::new("history");
        let path = root.0.join("sub").join("cli-history.txt");
        assert_eq!(History::load(&path), History::default());
        let mut h = History::default();
        for cmd in ["dir", "echo héllo", "git log --oneline"] {
            h.push(cmd);
        }
        h.save(&path).unwrap();
        assert_eq!(History::load(&path), h);
        // A hand-edited file with repeats and blank lines loads cleaned up.
        std::fs::write(&path, "a\na\n\nb\r\n").unwrap();
        assert_eq!(History::load(&path).items(), ["a", "b"]);
        // Not UTF-8 (saved as ANSI elsewhere): the other lines survive.
        std::fs::write(&path, b"dir\necho caf\xE9\ngit status\n").unwrap();
        assert_eq!(
            History::load(&path).items(),
            ["dir", "echo caf\u{FFFD}", "git status"]
        );
    }

    #[test]
    fn history_cursor_walks_back_and_returns_the_draft() {
        let items: Vec<String> = ["one", "two", "three"].map(String::from).to_vec();
        let mut c = HistoryCursor::default();
        assert_eq!(c.down(&items), None, "not browsing yet");
        assert_eq!(c.up(&items, "typ").as_deref(), Some("three"));
        assert_eq!(c.up(&items, "x").as_deref(), Some("two"));
        assert_eq!(c.up(&items, "x").as_deref(), Some("one"));
        assert_eq!(c.up(&items, "x"), None, "oldest stays");
        assert_eq!(c.down(&items).as_deref(), Some("two"));
        assert_eq!(c.down(&items).as_deref(), Some("three"));
        assert_eq!(c.down(&items).as_deref(), Some("typ"), "back to the draft");
        assert_eq!(c.down(&items), None);
        assert_eq!(HistoryCursor::default().up(&[], "x"), None);
        c.up(&items, "");
        c.reset();
        assert_eq!(c.up(&items, "").as_deref(), Some("three"));
    }

    #[test]
    fn output_keeps_the_last_two_thousand_lines() {
        let mut o = Output::default();
        for i in 0..OUTPUT_CAP + 5 {
            o.push(format!("l{i}"));
        }
        assert_eq!(o.lines().len(), OUTPUT_CAP);
        assert_eq!(o.lines()[0], "l5");
        assert_eq!(o.total(), OUTPUT_CAP as u64 + 5);
        let mut small = Output::default();
        small.push("a".into());
        small.push("b".into());
        assert_eq!(small.text(), "a\r\nb");
        small.clear();
        assert_eq!(small.text(), "");
        assert_eq!(small.total(), 2, "clear keeps the count");
    }

    #[test]
    fn window_sync_appends_new_lines_or_resets() {
        let mut o = Output::default();
        let mut shown = WindowSync::default();
        o.push("a".into());
        o.push("b".into());
        assert_eq!(
            shown.next(&o),
            SyncStep::Append(vec!["a".into(), "b".into()])
        );
        assert_eq!(shown.next(&o), SyncStep::Nothing);
        o.push("c".into());
        assert_eq!(shown.next(&o), SyncStep::Append(vec!["c".into()]));
        // After `cls` the window starts over.
        o.clear();
        shown.reset(&o);
        assert_eq!(shown.next(&o), SyncStep::Nothing);
        // More lines than the buffer keeps: the whole buffer instead.
        for i in 0..OUTPUT_CAP + 1 {
            o.push(format!("l{i}"));
        }
        assert_eq!(shown.next(&o), SyncStep::Reset);
        // Appends past a full buffer: reset every 500 or so, never more held.
        let mut resets = 0;
        for i in 0..3 * OUTPUT_CAP {
            o.push(format!("m{i}"));
            if shown.next(&o) == SyncStep::Reset {
                resets += 1;
            }
            assert!(shown.held <= OUTPUT_CAP + OUTPUT_CAP / 4);
        }
        assert_eq!(resets, 3 * OUTPUT_CAP / (OUTPUT_CAP / 4 + 1));
    }

    #[test]
    fn exit_lines() {
        assert_eq!(exit_line(Some(0), false), "[exit 0]");
        assert_eq!(exit_line(Some(-1), false), "[exit -1]");
        assert_eq!(exit_line(None, false), "[exit ?]");
        assert_eq!(exit_line(Some(1), true), "[dihentikan]");
        i18n::set_lang(i18n::Lang::En);
        assert_eq!(exit_line(Some(1), true), "[stopped]");
    }

    #[test]
    fn session_tracks_the_running_command_and_the_card() {
        let mut s = Session::default();
        s.begin("dir");
        s.started(1);
        assert_eq!(s.running(), Some(1));
        assert!(!s.lines(2, vec!["stale".into()]), "another run is ignored");
        assert!(s.lines(1, (1..=6).map(|i| format!("f{i}")).collect()));
        assert_eq!(s.last_command(), Some("dir"));
        assert_eq!(s.tail().collect::<Vec<_>>(), ["f3", "f4", "f5", "f6"]);
        assert!(s.exit(1, Some(0), false));
        assert!(!s.exit(1, Some(0), false), "only once");
        assert_eq!(s.running(), None);
        assert_eq!(s.tail().collect::<Vec<_>>(), ["f4", "f5", "f6", "[exit 0]"]);
        assert_eq!(s.output.lines()[0], "> dir");
        assert_eq!(s.output.lines().len(), 8);
        // A builtin's answer replaces the card's lines.
        s.begin("cd nope");
        s.note("cd: x".into());
        assert_eq!(s.tail().collect::<Vec<_>>(), ["cd: x"]);
        s.clear();
        assert_eq!((s.last_command(), s.output.lines().len()), (None, 0));
        assert_eq!(s.tail().count(), 0);
    }

    #[test]
    fn builtins_while_a_command_runs_stay_off_the_card() {
        let mut s = Session::default();
        s.begin("ping -n 20 127.0.0.1");
        s.started(1);
        s.lines(1, vec!["Reply 1".into()]);
        // `cd`, an error, another command refused, and `cls`.
        s.begin("cd nope");
        s.note("cd: x".into());
        s.begin("dir");
        s.note("busy".into());
        assert_eq!(s.last_command(), Some("ping -n 20 127.0.0.1"));
        assert_eq!(s.tail().collect::<Vec<_>>(), ["Reply 1"]);
        assert_eq!(s.output.lines().len(), 6, "the window still shows them");
        s.clear();
        assert_eq!(s.output.lines().len(), 0);
        assert_eq!(s.last_command(), Some("ping -n 20 127.0.0.1"));
        // The run goes on onto the card.
        s.lines(1, vec!["Reply 2".into()]);
        s.exit(1, Some(0), false);
        assert_eq!(
            s.tail().collect::<Vec<_>>(),
            ["Reply 1", "Reply 2", "[exit 0]"]
        );
        // Once it is over, builtins take the card again.
        s.begin("cd ..");
        assert_eq!(s.last_command(), Some("cd .."));
        assert_eq!(s.tail().count(), 0);
    }

    #[test]
    fn shell_names_round_trip_and_typos_fall_back() {
        assert_eq!(Shell::default(), Shell::PowerShell);
        for s in Shell::ALL {
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<Shell>(&json).unwrap(), s);
        }
        assert_eq!(serde_json::to_string(&Shell::Cmd).unwrap(), "\"cmd\"");
        assert_eq!(
            serde_json::from_str::<Shell>("\"CMD\"").unwrap(),
            Shell::Cmd
        );
        assert_eq!(
            serde_json::from_str::<Shell>("\"bash\"").unwrap(),
            Shell::PowerShell
        );
    }
}
