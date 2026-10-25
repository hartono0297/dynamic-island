use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::model::PomoField;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PomoSettings {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub sessions: u32,
}

impl Default for PomoSettings {
    fn default() -> Self {
        Self {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            sessions: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PomoStatus {
    Idle,
    Running { ends_at: Instant },
    Paused { remaining: Duration },
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseDone {
    pub finished: Phase,
    pub next: Phase,
}

#[derive(Clone, Debug)]
pub struct Pomodoro {
    settings: PomoSettings,
    phase: Phase,
    session: u32,
    status: PomoStatus,
}

/// `(min, max, step)` for each stepper, in minutes (sessions: count).
fn range(field: PomoField) -> (u32, u32, u32) {
    match field {
        PomoField::Focus => (5, 120, 5),
        PomoField::ShortBreak => (1, 30, 1),
        PomoField::LongBreak => (5, 60, 5),
        PomoField::Sessions => (1, 12, 1),
    }
}

impl PomoSettings {
    fn field_mut(&mut self, field: PomoField) -> &mut u32 {
        match field {
            PomoField::Focus => &mut self.focus_min,
            PomoField::ShortBreak => &mut self.short_break_min,
            PomoField::LongBreak => &mut self.long_break_min,
            PomoField::Sessions => &mut self.sessions,
        }
    }

    /// Pulls hand-edited config values back into the stepper ranges, so a
    /// phase can never be zero minutes long.
    fn clamped(mut self) -> Self {
        for field in [
            PomoField::Focus,
            PomoField::ShortBreak,
            PomoField::LongBreak,
            PomoField::Sessions,
        ] {
            let (min, max, _) = range(field);
            let v = self.field_mut(field);
            *v = (*v).clamp(min, max);
        }
        self
    }
}

impl Pomodoro {
    pub fn new(settings: PomoSettings) -> Self {
        Self {
            settings: settings.clamped(),
            phase: Phase::Focus,
            session: 1,
            status: PomoStatus::Idle,
        }
    }

    pub fn settings(&self) -> PomoSettings {
        self.settings
    }

    #[allow(dead_code)] // Read by the pomodoro renderer (Task 8).
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 1-based focus session within the current cycle.
    pub fn session(&self) -> u32 {
        self.session
    }

    pub fn status(&self) -> PomoStatus {
        self.status
    }

    pub fn is_active(&self) -> bool {
        self.status != PomoStatus::Idle
    }

    pub fn start_pause(&mut self, now: Instant) {
        self.status = match self.status {
            PomoStatus::Idle | PomoStatus::Ready => PomoStatus::Running {
                ends_at: now + self.full_length(),
            },
            PomoStatus::Paused { remaining } => PomoStatus::Running {
                ends_at: now + remaining,
            },
            PomoStatus::Running { ends_at } => PomoStatus::Paused {
                remaining: ends_at.saturating_duration_since(now),
            },
        };
    }

    pub fn reset(&mut self) {
        self.phase = Phase::Focus;
        self.session = 1;
        self.status = PomoStatus::Idle;
    }

    /// Ends the current phase now, whatever its state, and waits for a start.
    pub fn skip(&mut self, _now: Instant) -> PhaseDone {
        self.advance()
    }

    pub fn tick(&mut self, now: Instant) -> Option<PhaseDone> {
        match self.status {
            PomoStatus::Running { ends_at } if now >= ends_at => Some(self.advance()),
            _ => None,
        }
    }

    #[allow(dead_code)] // Read by the pomodoro renderer (Task 8).
    pub fn remaining(&self, now: Instant) -> Duration {
        match self.status {
            PomoStatus::Idle | PomoStatus::Ready => self.full_length(),
            PomoStatus::Running { ends_at } => ends_at.saturating_duration_since(now),
            PomoStatus::Paused { remaining } => remaining,
        }
    }

    /// Elapsed fraction of the current phase, 0..=1.
    #[allow(dead_code)] // Read by the pomodoro renderer (Task 8).
    pub fn progress(&self, now: Instant) -> f32 {
        let full = self.full_length().as_secs_f32();
        (1.0 - self.remaining(now).as_secs_f32() / full).clamp(0.0, 1.0)
    }

    /// Steps one setting up (`dir > 0`) or down (`dir < 0`). Only allowed while
    /// no phase is in progress; a change made while Ready applies to the phase
    /// that starts next.
    pub fn adjust(&mut self, field: PomoField, dir: i32) -> bool {
        if !matches!(self.status, PomoStatus::Idle | PomoStatus::Ready) || dir == 0 {
            return false;
        }
        let (min, max, step) = range(field);
        let v = self.settings.field_mut(field);
        let next = if dir > 0 {
            v.saturating_add(step)
        } else {
            v.saturating_sub(step)
        }
        .clamp(min, max);
        let changed = next != *v;
        *v = next;
        changed
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match self.status {
            PomoStatus::Running { ends_at } => Some(ends_at),
            _ => None,
        }
    }

    fn full_length(&self) -> Duration {
        let min = match self.phase {
            Phase::Focus => self.settings.focus_min,
            Phase::ShortBreak => self.settings.short_break_min,
            Phase::LongBreak => self.settings.long_break_min,
        };
        Duration::from_secs(60 * u64::from(min))
    }

    fn advance(&mut self) -> PhaseDone {
        let finished = self.phase;
        self.phase = match finished {
            Phase::Focus if self.session >= self.settings.sessions => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            Phase::ShortBreak => {
                self.session += 1;
                Phase::Focus
            }
            Phase::LongBreak => {
                self.session = 1;
                Phase::Focus
            }
        };
        self.status = PomoStatus::Ready;
        PhaseDone {
            finished,
            next: self.phase,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(n: u64) -> Duration {
        Duration::from_secs(60 * n)
    }

    fn running(p: &Pomodoro) -> bool {
        matches!(p.status(), PomoStatus::Running { .. })
    }

    #[test]
    fn start_runs_focus_for_full_length() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert_eq!(p.remaining(t0 + m(5)), m(20));
        assert_eq!(p.next_deadline(), Some(t0 + m(25)));
    }

    #[test]
    fn pause_freezes_remaining() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        p.start_pause(t0 + m(10));
        assert_eq!(p.status(), PomoStatus::Paused { remaining: m(15) });
        assert_eq!(p.remaining(t0 + m(40)), m(15));
        assert_eq!(p.next_deadline(), None);
        p.start_pause(t0 + m(40));
        assert_eq!(p.next_deadline(), Some(t0 + m(55)));
    }

    #[test]
    fn focus_end_goes_to_short_break_ready() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert_eq!(
            p.tick(t0 + m(25)),
            Some(PhaseDone {
                finished: Phase::Focus,
                next: Phase::ShortBreak
            })
        );
        assert_eq!(p.status(), PomoStatus::Ready);
        assert!(!running(&p));
        assert_eq!(p.phase(), Phase::ShortBreak);
        assert!(p.is_active());
        assert_eq!(p.next_deadline(), None);
    }

    #[test]
    fn last_focus_goes_to_long_break_then_session_one() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings {
            sessions: 2,
            ..PomoSettings::default()
        });
        let mut seen = vec![(p.phase(), p.session())];
        let mut now = t0;
        for _ in 0..4 {
            p.start_pause(now);
            now = p.next_deadline().unwrap();
            assert!(p.tick(now).is_some());
            seen.push((p.phase(), p.session()));
        }
        assert_eq!(
            seen,
            vec![
                (Phase::Focus, 1),
                (Phase::ShortBreak, 1),
                (Phase::Focus, 2),
                (Phase::LongBreak, 2),
                (Phase::Focus, 1),
            ]
        );
    }

    #[test]
    fn tick_before_end_does_nothing() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert_eq!(p.tick(t0 + m(24)), None);
        assert!(running(&p));
        assert_eq!(p.phase(), Phase::Focus);
    }

    #[test]
    fn skip_advances_immediately() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert_eq!(
            p.skip(t0 + m(3)),
            PhaseDone {
                finished: Phase::Focus,
                next: Phase::ShortBreak
            }
        );
        assert_eq!(p.status(), PomoStatus::Ready);
        assert_eq!(p.phase(), Phase::ShortBreak);
    }

    #[test]
    fn reset_returns_to_idle_focus_one() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        p.skip(t0);
        p.skip(t0);
        assert_eq!((p.phase(), p.session()), (Phase::Focus, 2));
        p.start_pause(t0);
        p.reset();
        assert_eq!(p.status(), PomoStatus::Idle);
        assert_eq!(p.phase(), Phase::Focus);
        assert_eq!(p.session(), 1);
        assert!(!p.is_active());
        assert_eq!(p.remaining(t0), m(25));
    }

    #[test]
    fn adjust_clamps_and_steps() {
        let mut p = Pomodoro::new(PomoSettings::default());
        assert!(p.adjust(PomoField::Focus, 1));
        assert_eq!(p.settings().focus_min, 30);

        let mut p = Pomodoro::new(PomoSettings {
            focus_min: 120,
            short_break_min: 1,
            long_break_min: 60,
            sessions: 12,
        });
        assert!(!p.adjust(PomoField::Focus, 1));
        assert_eq!(p.settings().focus_min, 120);
        assert!(!p.adjust(PomoField::ShortBreak, -1));
        assert_eq!(p.settings().short_break_min, 1);
        assert!(!p.adjust(PomoField::Sessions, 1));
        assert_eq!(p.settings().sessions, 12);
        assert!(!p.adjust(PomoField::LongBreak, 1));
        assert!(p.adjust(PomoField::LongBreak, -1));
        assert_eq!(p.settings().long_break_min, 55);
    }

    #[test]
    fn adjust_ignored_while_running() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert!(!p.adjust(PomoField::Focus, 1));
        assert_eq!(p.settings(), PomoSettings::default());
        p.start_pause(t0 + m(1));
        assert!(!p.adjust(PomoField::Focus, 1), "paused counts as busy");
    }

    #[test]
    fn adjust_while_ready_applies_to_next_phase() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        p.tick(t0 + m(25));
        assert!(p.adjust(PomoField::ShortBreak, 1));
        assert_eq!(p.remaining(t0 + m(25)), m(6));
        p.start_pause(t0 + m(25));
        assert_eq!(p.next_deadline(), Some(t0 + m(31)));
    }

    #[test]
    fn skip_when_idle_finishes_unstarted_phase() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        assert_eq!(
            p.skip(t0),
            PhaseDone {
                finished: Phase::Focus,
                next: Phase::ShortBreak
            }
        );
        assert_eq!(p.status(), PomoStatus::Ready);
    }

    #[test]
    fn progress_is_elapsed_fraction() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        assert_eq!(p.progress(t0), 0.0);
        p.start_pause(t0);
        assert!((p.progress(t0 + m(5)) - 0.2).abs() < 1e-3);
        assert_eq!(p.progress(t0 + m(30)), 1.0);
    }
}
