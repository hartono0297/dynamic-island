//! What the CLI page needs beyond `cli::Session`: how long the current or
//! last command ran and how it ended, which output lines are echoed
//! commands, the history suggestions under the cursor, and the colour of a
//! line. Pure, so the page's behaviour is tested without a window.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::cli::{self, Output};

/// Suggestions shown above the input at most.
pub const MAX_SUGGESTIONS: usize = 3;

/// The run the header chip describes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RunState {
    /// Nothing ran yet (or since `cls`).
    #[default]
    Idle,
    Running {
        since: Instant,
    },
    /// The last command ended: its exit code (`None` if unknown), whether
    /// Stop ended it, and how long it took.
    Ended {
        code: Option<i32>,
        killed: bool,
        took: Duration,
    },
}

impl RunState {
    pub fn running(&self) -> bool {
        matches!(self, RunState::Running { .. })
    }
}

/// Start and end of the shell runs, for the header chip.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunClock {
    state: RunState,
}

impl RunClock {
    pub fn start(&mut self, now: Instant) {
        self.state = RunState::Running { since: now };
    }

    /// The running command ended at `now`.
    pub fn end(&mut self, code: Option<i32>, killed: bool, now: Instant) {
        let took = match self.state {
            RunState::Running { since } => now.saturating_duration_since(since),
            _ => Duration::ZERO,
        };
        self.state = RunState::Ended { code, killed, took };
    }

    /// `cls` forgets a finished run; a running one keeps its chip.
    pub fn clear(&mut self) {
        if !self.state.running() {
            self.state = RunState::Idle;
        }
    }

    pub fn state(&self) -> RunState {
        self.state
    }
}

/// Which output lines are the echoes of entered commands, by their index
/// among all lines ever pushed (`Output::total` before the echo).
#[derive(Clone, Debug, Default)]
pub struct Echoes {
    starts: VecDeque<u64>,
}

impl Echoes {
    /// The echo about to be pushed to `output` is a command.
    pub fn mark(&mut self, output: &Output) {
        self.starts.push_back(output.total());
        // Indices of lines that scrolled out of the buffer are of no use.
        let first = output.total().saturating_sub(cli::OUTPUT_CAP as u64);
        while self.starts.front().is_some_and(|&i| i < first) {
            self.starts.pop_front();
        }
    }

    /// Whether line `i` of `output.lines()` is an echoed command.
    pub fn is_echo(&self, output: &Output, i: usize) -> bool {
        let first = output.total() - output.lines().len() as u64;
        self.starts.binary_search(&(first + i as u64)).is_ok()
    }
}

/// How an output line is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// An entered command: the prompt in the accent, then the command.
    Command,
    Output,
    /// A line that reads as an error, or a failed exit: soft red.
    Error,
    /// `[exit 0]`: sage.
    Ok,
    /// The island's own quiet lines: `[dihentikan]`, `[exit ?]`.
    Quiet,
}

pub fn line_kind(line: &str, echo: bool) -> LineKind {
    if echo {
        return LineKind::Command;
    }
    if line == cli::exit_line(Some(0), false) {
        return LineKind::Ok;
    }
    if line == cli::exit_line(None, true) || line == cli::exit_line(None, false) {
        return LineKind::Quiet;
    }
    if line.starts_with("[exit ") && line.ends_with(']') {
        return LineKind::Error;
    }
    let lower = line.trim_start().to_lowercase();
    let error = lower.starts_with("fail")
        || lower.starts_with("error")
        || lower.contains("error:")
        || lower.contains("exception")
        || lower.starts_with("fatal:")
        || lower.starts_with("cd: ");
    if error {
        LineKind::Error
    } else {
        LineKind::Output
    }
}

/// The text of an echoed command (`> cmd` → `cmd`).
pub fn echo_command(line: &str) -> &str {
    line.strip_prefix("> ").unwrap_or(line)
}

/// Earlier commands that start with what is typed, newest first, each once,
/// never the typed text itself; none for an empty input.
pub fn suggestions<'a>(history: &'a [String], typed: &str, max: usize) -> Vec<&'a str> {
    let typed = typed.trim_start();
    if typed.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<&str> = Vec::new();
    for cmd in history.iter().rev() {
        if out.len() >= max {
            break;
        }
        if cmd.starts_with(typed) && cmd != typed && !out.contains(&cmd.as_str()) {
            out.push(cmd);
        }
    }
    out
}

/// Tab: the input becomes the highlighted suggestion; `None` when there is
/// none to take.
pub fn accept(suggestions: &[&str], selected: usize) -> Option<String> {
    suggestions
        .get(selected.min(suggestions.len().saturating_sub(1)))
        .map(|s| (*s).to_owned())
}

/// The highlighted suggestion moved by `step`, wrapping around `len`.
pub fn step_selection(selected: usize, len: usize, step: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as i64;
    ((selected.min(len as usize - 1) as i64 + i64::from(step)).rem_euclid(len)) as usize
}

/// How far the output can scroll up: lines that do not fit in `rows`.
pub fn max_scroll(lines: usize, rows: usize) -> usize {
    lines.saturating_sub(rows)
}

/// The output lines to show: `rows` lines ending `scroll` lines above the
/// newest (clamped), as a range of indices into the buffer.
pub fn visible_range(lines: usize, rows: usize, scroll: usize) -> std::ops::Range<usize> {
    let scroll = scroll.min(max_scroll(lines, rows));
    let end = lines - scroll;
    end.saturating_sub(rows)..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn suggestions_match_the_prefix_newest_first_once() {
        let h = history(&[
            "git log --oneline -12",
            "npm test",
            "git pull",
            "git status",
            "dir",
            "git pull",
        ]);
        assert_eq!(
            suggestions(&h, "git", 3),
            ["git pull", "git status", "git log --oneline -12"]
        );
        assert_eq!(suggestions(&h, "git", 2), ["git pull", "git status"]);
        assert_eq!(suggestions(&h, "git s", 3), ["git status"]);
        // The exact command typed is not suggested back.
        assert!(suggestions(&h, "git status", 3).is_empty());
        assert!(suggestions(&h, "", 3).is_empty(), "empty input");
        assert!(suggestions(&h, "  ", 3).is_empty());
        assert!(suggestions(&h, "cargo", 3).is_empty());
        assert!(suggestions(&[], "git", 3).is_empty());
    }

    #[test]
    fn tab_takes_the_highlighted_suggestion() {
        let list = ["git status", "git pull"];
        assert_eq!(accept(&list, 0).as_deref(), Some("git status"));
        assert_eq!(accept(&list, 1).as_deref(), Some("git pull"));
        assert_eq!(accept(&list, 9).as_deref(), Some("git pull"), "clamped");
        assert_eq!(accept(&[], 0), None);
        assert_eq!(step_selection(0, 3, 1), 1);
        assert_eq!(step_selection(2, 3, 1), 0, "wraps");
        assert_eq!(step_selection(0, 3, -1), 2);
        assert_eq!(step_selection(5, 0, 1), 0);
    }

    #[test]
    fn the_clock_times_a_run_and_keeps_its_end() {
        let t0 = Instant::now();
        let mut c = RunClock::default();
        assert_eq!(c.state(), RunState::Idle);
        c.start(t0);
        assert!(c.state().running());
        c.clear();
        assert!(c.state().running(), "cls keeps a running chip");
        c.end(Some(1), false, t0 + Duration::from_millis(3800));
        assert_eq!(
            c.state(),
            RunState::Ended {
                code: Some(1),
                killed: false,
                took: Duration::from_millis(3800)
            }
        );
        c.clear();
        assert_eq!(c.state(), RunState::Idle);
    }

    #[test]
    fn echoes_are_tracked_by_line() {
        let mut out = Output::default();
        let mut e = Echoes::default();
        e.mark(&out);
        out.push(cli::echo_line("npm run build"));
        out.push("> rumah-kemang@1.4.0 build".into());
        e.mark(&out);
        out.push(cli::echo_line("dir"));
        assert!(e.is_echo(&out, 0));
        assert!(!e.is_echo(&out, 1), "program output that looks like one");
        assert!(e.is_echo(&out, 2));
        // After `cls` the indices still line up.
        out.clear();
        e.mark(&out);
        out.push(cli::echo_line("git status"));
        out.push("On branch main".into());
        assert!(e.is_echo(&out, 0) && !e.is_echo(&out, 1));
    }

    #[test]
    fn lines_are_coloured_by_what_they_say() {
        assert_eq!(line_kind("> dir", true), LineKind::Command);
        assert_eq!(line_kind("[exit 0]", false), LineKind::Ok);
        assert_eq!(line_kind("[exit 1]", false), LineKind::Error);
        assert_eq!(line_kind("[exit ?]", false), LineKind::Quiet);
        assert_eq!(line_kind("[dihentikan]", false), LineKind::Quiet);
        assert_eq!(line_kind("FAIL src/rab.test.ts", false), LineKind::Error);
        assert_eq!(
            line_kind("error: could not compile", false),
            LineKind::Error
        );
        assert_eq!(line_kind("npm ERR! code 1", false), LineKind::Output);
        // A summary stays plain, as in the design.
        assert_eq!(
            line_kind("Tests: 1 failed, 23 passed", false),
            LineKind::Output
        );
        assert_eq!(line_kind("cd: folder tidak ada: x", false), LineKind::Error);
        assert_eq!(line_kind("PASS src/denah.test.ts", false), LineKind::Output);
        assert_eq!(echo_command("> git status"), "git status");
    }

    #[test]
    fn the_view_ends_scroll_lines_above_the_newest() {
        assert_eq!(visible_range(3, 8, 0), 0..3);
        assert_eq!(visible_range(20, 8, 0), 12..20);
        assert_eq!(visible_range(20, 8, 5), 7..15);
        assert_eq!(visible_range(20, 8, 99), 0..8, "clamped at the oldest");
        assert_eq!(max_scroll(20, 8), 12);
        assert_eq!(max_scroll(3, 8), 0);
    }
}
