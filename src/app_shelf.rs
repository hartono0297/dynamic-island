//! The island's side of the file shelf: what is on it, the cards drawn from
//! that, a dropped list, and a card pressed and pulled out. A child of `app`,
//! so it works on `App` directly. The list and the cards' data are
//! `shelf.rs`; the OLE drop target and `DoDragDrop` are `shelf_ole.rs`; the
//! reducer only knows the shelf is open and which card was clicked.
//!
//! The shelf lives in memory: a restart empties it. Nothing runs while it
//! is closed; whether a file is still there is looked at when the page
//! opens, after a drop and before a card is opened or dragged.

use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::{App, Event, Page, View, client_xy, left_button_down};
use crate::shelf::{self, Probe, Shelf, ShelfCard};
use crate::shortcuts;

/// One-shot: how long the panel stays open after a drop that left the
/// pointer where hover never arrives (it ends at once when hover does).
pub(super) const TIMER_SHELF_HOLD: usize = 27;
const SHELF_HOLD_MS: u32 = 2_500;
/// A drag that leaves the island keeps the panel this long, so one that
/// strays off the edge and back does not flicker it away.
const SHELF_LEAVE_MS: u32 = 600;
/// A press on a card turns into a drag out after this many logical pixels.
const DRAG_START_DIP: f32 = 6.0;

#[derive(Default)]
pub(super) struct ShelfState {
    pub shelf: Shelf,
    /// What the page draws, from `shelf` and the file system as last looked.
    pub cards: Vec<ShelfCard>,
    /// The page showed at the last `sync_shelf`.
    shown: bool,
    /// Files are over the island right now (the drop zone lights).
    pub dragging: bool,
    /// A left press on card `i` at this client point: it may become a drag.
    press: Option<(usize, i32, i32)>,
}

impl App {
    /// Looks at the files again (the page opened, the list changed).
    fn shelf_look(&mut self) {
        self.shelf.cards = self.shelf.shelf.cards(shelf::probe);
    }

    /// The OLE drop target saw files enter the island or leave it.
    pub(super) fn on_shelf_drag(&mut self, on: bool) -> Option<Event> {
        self.shelf.dragging = on;
        if on {
            // A drag that comes back cancels the pending close.
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_SHELF_HOLD);
            }
            Some(Event::ShelfDrag(true))
        } else {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_SHELF_HOLD, SHELF_LEAVE_MS, None);
            }
            // The drop zone's outline goes now, not with the panel.
            if !self.animating() {
                self.draw(Instant::now());
            }
            None
        }
    }

    /// The Shelf page showed: look at the files once, then not again until
    /// it is shown again.
    pub(super) fn sync_shelf(&mut self) {
        let wanted = self.state.view() == View::Panel(Page::Shelf);
        if wanted && !self.shelf.shown {
            self.shelf_look();
        }
        self.shelf.shown = wanted;
    }

    /// Files dropped on the island: they go on the shelf, the panel stays
    /// on it a moment longer so the cards are seen arriving.
    pub(super) fn on_shelf_drop(&mut self, paths: Vec<PathBuf>, now: Instant) {
        self.shelf.dragging = false;
        self.shelf.shelf.add(paths);
        self.shelf_look();
        self.apply(Event::ShelfDrag(true), now);
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SHELF_HOLD, SHELF_HOLD_MS, None);
        }
        self.refresh(now);
    }

    /// `TIMER_SHELF_HOLD`: the drop's hold on the panel ends.
    pub(super) fn on_shelf_hold_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SHELF_HOLD);
        }
        self.apply(Event::ShelfDrag(false), Instant::now());
    }

    /// A card's file opens, after the panel has collapsed. A file that is
    /// gone is only marked: the card turns dim.
    pub(super) fn shelf_open(&mut self, i: usize) {
        let Some(path) = self.shelf.shelf.get(i).map(PathBuf::from) else {
            return;
        };
        if shelf::probe(&path) == Probe::Missing {
            self.shelf_look();
            return;
        }
        self.queue_shortcut(shortcuts::Action::Shell(path.display().to_string()));
    }

    pub(super) fn shelf_remove(&mut self, i: usize) {
        if self.shelf.shelf.remove(i) {
            self.shelf_look();
            self.refresh(Instant::now());
        }
    }

    pub(super) fn shelf_clear(&mut self) {
        if self.shelf.shelf.clear() {
            self.shelf_look();
            self.refresh(Instant::now());
        }
    }

    /// A left press: on a card whose file is there, it may become a drag.
    pub(super) fn shelf_press(&mut self, on_card: Option<usize>, lparam: windows::Win32::Foundation::LPARAM) {
        self.shelf.press = on_card
            .filter(|&i| self.shelf.cards.get(i).is_some_and(|c| !c.stale))
            .map(|i| {
                let (x, y) = client_xy(lparam);
                (i, x, y)
            });
    }

    pub(super) fn shelf_release(&mut self) {
        self.shelf.press = None;
    }

    /// The pointer moved with a card pressed: past a few pixels with the
    /// button still down it is a drag out, and the file to carry comes back
    /// (the caller starts `DoDragDrop` outside the app borrow). The click
    /// that would have opened the card is dropped.
    pub(super) fn shelf_drag_start(
        &mut self,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> Option<PathBuf> {
        let (i, x0, y0) = self.shelf.press?;
        if !left_button_down() || self.state.view() != View::Panel(Page::Shelf) {
            self.shelf.press = None;
            return None;
        }
        let (x, y) = client_xy(lparam);
        let min = DRAG_START_DIP * self.gesture_scale();
        let (dx, dy) = ((x - x0) as f32, (y - y0) as f32);
        if dx * dx + dy * dy < min * min {
            return None;
        }
        self.shelf.press = None;
        self.click.clear();
        let path = PathBuf::from(self.shelf.shelf.get(i)?);
        if shelf::probe(&path) == Probe::Missing {
            self.shelf_look();
            self.refresh(Instant::now());
            return None;
        }
        Some(path)
    }
}
