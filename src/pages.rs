//! Which panel pages the wheel walks through, and in what order: the
//! "Halaman panel" setting. Every page is always listed exactly once; a page
//! that is off is skipped by the wheel and the page dots, but its own button
//! (the Clipboard tile, a disk warning) still opens it.

use serde::{Deserialize, Serialize};

use crate::model::Page;

/// One page in `config.json`: its name and whether the wheel shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageEntry {
    pub page: String,
    pub on: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelPages {
    entries: Vec<(Page, bool)>,
}

impl Default for PanelPages {
    fn default() -> Self {
        PanelPages {
            entries: Page::ALL.iter().map(|p| (*p, true)).collect(),
        }
    }
}

impl PanelPages {
    /// The saved list made whole: unknown and repeated names are dropped, a
    /// page the file does not name (an older config) joins right after the
    /// page before it in `Page::ALL`, and a list with nothing on shows all.
    pub fn from_saved(saved: &[PageEntry]) -> Self {
        let mut entries: Vec<(Page, bool)> = Vec::new();
        for e in saved {
            if let Some(page) = page_from_name(&e.page)
                && !entries.iter().any(|(p, _)| *p == page)
            {
                entries.push((page, e.on));
            }
        }
        for (i, page) in Page::ALL.iter().enumerate() {
            if entries.iter().any(|(p, _)| p == page) {
                continue;
            }
            let after = Page::ALL[..i]
                .iter()
                .rev()
                .find_map(|prev| entries.iter().position(|(p, _)| p == prev));
            entries.insert(after.map_or(0, |a| a + 1), (*page, true));
        }
        if entries.iter().all(|(_, on)| !on) {
            for e in &mut entries {
                e.1 = true;
            }
        }
        PanelPages { entries }
    }

    pub fn to_saved(&self) -> Vec<PageEntry> {
        self.entries
            .iter()
            .map(|(p, on)| PageEntry {
                page: page_name(*p).to_owned(),
                on: *on,
            })
            .collect()
    }

    pub fn entries(&self) -> &[(Page, bool)] {
        &self.entries
    }

    /// The pages the wheel and the dots show, in order.
    pub fn visible(&self) -> Vec<Page> {
        self.entries
            .iter()
            .filter(|(_, on)| *on)
            .map(|(p, _)| *p)
            .collect()
    }

    pub fn is_on(&self, page: Page) -> bool {
        self.entries.iter().any(|(p, on)| *p == page && *on)
    }

    /// Flips `page`; refused (false) when it is the last page still shown.
    pub fn toggle(&mut self, page: Page) -> bool {
        let i = self.position(page);
        if self.entries[i].1 && self.visible().len() == 1 {
            return false;
        }
        self.entries[i].1 = !self.entries[i].1;
        true
    }

    /// Moves `page` `delta` places (negative: earlier), stopping at the
    /// ends; false when it did not move.
    pub fn move_by(&mut self, page: Page, delta: i32) -> bool {
        let from = self.position(page);
        let last = self.entries.len() as i64 - 1;
        let to = (from as i64 + i64::from(delta)).clamp(0, last) as usize;
        if to == from {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        true
    }

    /// Where `page` is in the order, shown or not.
    pub fn position(&self, page: Page) -> usize {
        self.entries
            .iter()
            .position(|(p, _)| *p == page)
            .expect("every page is listed")
    }

    /// The page `steps` shown pages after `from` (negative: before),
    /// stopping at the ends. From a page that is off (opened by its own
    /// button) the count starts at its place in the order; with nothing
    /// shown on that side it stays.
    pub fn step(&self, from: Page, steps: i64) -> Page {
        if steps == 0 {
            return from;
        }
        let at = self.position(from);
        let side: Vec<Page> = if steps > 0 {
            self.entries[at + 1..]
                .iter()
                .filter(|(_, on)| *on)
                .map(|(p, _)| *p)
                .collect()
        } else {
            self.entries[..at]
                .iter()
                .rev()
                .filter(|(_, on)| *on)
                .map(|(p, _)| *p)
                .collect()
        };
        let n = steps.unsigned_abs() as usize;
        side.get(n - 1).or(side.last()).copied().unwrap_or(from)
    }
}

/// The page's name in `config.json`.
pub fn page_name(page: Page) -> &'static str {
    match page {
        Page::Calendar => "Calendar",
        Page::Pomodoro => "Pomodoro",
        Page::Reminders => "Reminders",
        Page::Notes => "Notes",
        Page::Media => "Media",
        Page::Clipboard => "Clipboard",
        Page::System => "System",
        Page::Cli => "Cli",
        Page::Shortcuts => "Shortcuts",
    }
}

/// Case-insensitive; `None` for a name no page has.
pub fn page_from_name(name: &str) -> Option<Page> {
    let name = name.trim();
    Page::ALL
        .into_iter()
        .find(|p| page_name(*p).eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(page: &str, on: bool) -> PageEntry {
        PageEntry {
            page: page.to_owned(),
            on,
        }
    }

    #[test]
    fn default_shows_all_nine_pages_in_the_design_order() {
        let p = PanelPages::default();
        assert_eq!(p.visible(), Page::ALL.to_vec());
        assert_eq!(
            Page::ALL,
            [
                Page::Calendar,
                Page::Pomodoro,
                Page::Reminders,
                Page::Notes,
                Page::Media,
                Page::Clipboard,
                Page::System,
                Page::Cli,
                Page::Shortcuts,
            ]
        );
    }

    #[test]
    fn names_round_trip_and_unknown_names_are_none() {
        for page in Page::ALL {
            assert_eq!(page_from_name(page_name(page)), Some(page));
        }
        assert_eq!(page_from_name(" media "), Some(Page::Media));
        assert_eq!(page_from_name("Weather"), None);
    }

    #[test]
    fn saved_order_and_flags_are_kept() {
        let mut saved: Vec<PageEntry> = Page::ALL
            .iter()
            .rev()
            .map(|p| entry(page_name(*p), true))
            .collect();
        saved[0].on = false; // Shortcuts, now first, is off
        let p = PanelPages::from_saved(&saved);
        let order: Vec<Page> = p.entries().iter().map(|e| e.0).collect();
        let mut reversed = Page::ALL.to_vec();
        reversed.reverse();
        assert_eq!(order, reversed);
        assert!(!p.is_on(Page::Shortcuts));
        assert_eq!(p.visible().first(), Some(&Page::Cli));
        assert_eq!(PanelPages::from_saved(&p.to_saved()), p);
    }

    #[test]
    fn unknown_and_repeated_names_are_dropped() {
        let saved = [
            entry("Notes", true),
            entry("Weather", true),
            entry("notes", false),
        ];
        let p = PanelPages::from_saved(&saved);
        assert_eq!(p.entries().len(), Page::ALL.len());
        assert!(p.is_on(Page::Notes), "the first Notes entry wins");
        // Calendar was not named, and nothing comes before it by default.
        assert_eq!(p.entries()[0].0, Page::Calendar);
    }

    #[test]
    fn a_page_missing_from_the_file_joins_after_its_default_neighbour() {
        // An older config without Clipboard and System.
        let saved: Vec<PageEntry> = [
            Page::Calendar,
            Page::Pomodoro,
            Page::Reminders,
            Page::Notes,
            Page::Media,
            Page::Cli,
            Page::Shortcuts,
        ]
        .iter()
        .map(|p| entry(page_name(*p), true))
        .collect();
        let p = PanelPages::from_saved(&saved);
        assert_eq!(p.visible(), Page::ALL.to_vec());
    }

    #[test]
    fn a_file_with_every_page_off_shows_them_all() {
        let saved: Vec<PageEntry> = Page::ALL
            .iter()
            .map(|p| entry(page_name(*p), false))
            .collect();
        assert_eq!(PanelPages::from_saved(&saved).visible(), Page::ALL.to_vec());
        assert_eq!(PanelPages::from_saved(&[]), PanelPages::default());
    }

    #[test]
    fn the_last_visible_page_cannot_be_turned_off() {
        let mut p = PanelPages::default();
        for page in &Page::ALL[1..] {
            assert!(p.toggle(*page));
        }
        assert_eq!(p.visible(), vec![Page::Calendar]);
        assert!(!p.toggle(Page::Calendar), "refused");
        assert!(p.is_on(Page::Calendar));
        assert!(p.toggle(Page::Notes), "turning one back on works");
        assert!(p.is_on(Page::Notes));
    }

    #[test]
    fn moving_a_page_swaps_it_with_its_neighbour_and_stops_at_the_ends() {
        let mut p = PanelPages::default();
        assert!(p.move_by(Page::Media, -1));
        assert_eq!(p.position(Page::Media), 3);
        assert_eq!(p.position(Page::Notes), 4);
        assert!(!p.move_by(Page::Calendar, -1), "already first");
        assert!(!p.move_by(Page::Shortcuts, 1), "already last");
        assert!(p.move_by(Page::Calendar, 1));
        assert_eq!(p.entries()[0].0, Page::Pomodoro);
    }

    #[test]
    fn the_wheel_skips_pages_that_are_off_and_stops_at_the_ends() {
        let mut p = PanelPages::default();
        p.toggle(Page::Pomodoro);
        p.toggle(Page::Reminders);
        assert_eq!(p.step(Page::Calendar, 1), Page::Notes);
        assert_eq!(p.step(Page::Notes, -1), Page::Calendar);
        assert_eq!(p.step(Page::Calendar, 2), Page::Media);
        assert_eq!(p.step(Page::Calendar, -3), Page::Calendar);
        assert_eq!(p.step(Page::Cli, 5), Page::Shortcuts);
        assert_eq!(p.step(Page::Media, 0), Page::Media);
    }

    #[test]
    fn from_a_page_that_is_off_the_wheel_goes_to_the_nearest_shown_one() {
        let mut p = PanelPages::default();
        p.toggle(Page::Clipboard);
        p.toggle(Page::System);
        // Opened by its tile while it is off.
        assert_eq!(p.step(Page::Clipboard, 1), Page::Cli);
        assert_eq!(p.step(Page::Clipboard, -1), Page::Media);
        assert_eq!(p.step(Page::System, 2), Page::Shortcuts);
        // Nothing shown past it: it stays.
        p.toggle(Page::Shortcuts);
        p.move_by(Page::System, 1);
        p.move_by(Page::System, 1);
        assert_eq!(p.entries().last().unwrap().0, Page::System);
        assert_eq!(p.step(Page::System, 1), Page::System);
    }

    #[test]
    fn the_wheel_follows_a_custom_order() {
        let mut p = PanelPages::default();
        p.move_by(Page::Shortcuts, -8);
        assert_eq!(p.entries()[0].0, Page::Shortcuts);
        assert_eq!(p.step(Page::Shortcuts, 1), Page::Calendar);
        assert_eq!(p.step(Page::Calendar, -1), Page::Shortcuts);
    }
}
