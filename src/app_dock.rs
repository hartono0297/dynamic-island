//! The island's side of the page dock (`ui::dock`): the displayed values,
//! the pointer feeding the targets, and the frame step.
//!
//! Power: there is no timer. A mouse move only sets targets; the
//! `FrameClock` runs while a displayed value is still moving
//! (`App::animating_at` asks `moving`) and stops once everything has
//! settled, so a pointer resting on an icon costs nothing. A child of
//! `app`, so it works on `App` directly.

use std::time::Instant;

use windows::Win32::Foundation::LPARAM;

use super::{App, client_xy};
use crate::model::View;
use crate::ui::dock::{Dock, DockFrame};

/// A frame after a long gap (the clock was stopped) must not jump the ease.
const MAX_DT: f32 = 0.1;

pub(super) struct DockState {
    dock: Dock,
    /// When the values were last advanced.
    last: Instant,
}

impl Default for DockState {
    fn default() -> Self {
        DockState {
            dock: Dock::new(),
            last: Instant::now(),
        }
    }
}

impl DockState {
    pub(super) fn frame(&self) -> DockFrame {
        self.dock.frame()
    }

    pub(super) fn moving(&self) -> bool {
        self.dock.moving()
    }
}

impl App {
    /// The pointer moved over the island: it is in the dock's zone (or not).
    pub(super) fn dock_follow(&mut self, lparam: LPARAM) {
        let (x, y) = client_xy(lparam);
        let (x, y) = self.renderer.to_dip(x as f32, y as f32);
        let inside = matches!(self.state.view(), View::Panel(_))
            && self
                .renderer
                .dock_zone()
                .is_some_and(|z| z.contains(x, y, self.dock.dock.expanded()));
        self.dock_point(inside.then_some(x));
    }

    /// The pointer left the island (or the window).
    pub(super) fn dock_leave(&mut self) {
        self.dock_point(None);
    }

    /// Sets the pointer; starts the clock when a value has to move.
    fn dock_point(&mut self, pointer_x: Option<f32>) {
        let layout = self
            .renderer
            .dock_zone()
            .map_or(&[][..], |z| z.icons.as_slice());
        if !self.dock.dock.hover(pointer_x, layout) || !self.dock.dock.moving() {
            return;
        }
        // A clock that was off starts from now, not from the last frame.
        if self.clock.is_none() {
            self.dock.last = Instant::now();
        }
        self.ensure_clock();
    }

    /// Before a frame is drawn: the closed panel is dots at once; a page
    /// without dots lets the row collapse.
    pub(super) fn dock_sync(&mut self) {
        if !matches!(self.state.view(), View::Panel(_)) {
            self.dock.dock.reset();
        } else if self.dock.dock.expanded() && self.renderer.dock_zone().is_none() {
            self.dock_point(None);
        }
    }

    /// One frame-clock tick: eases the values to their targets.
    pub(super) fn dock_step(&mut self, now: Instant) {
        let dt = now
            .saturating_duration_since(self.dock.last)
            .as_secs_f32()
            .min(MAX_DT);
        self.dock.last = now;
        self.dock.dock.advance(dt, self.animations);
    }
}
