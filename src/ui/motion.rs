//! The rules of every transition inside the open panel, as pure functions:
//! which page change slides, fades or stays instant, which sub-view switch
//! gets a short `Reveal`, how a slide that starts mid-slide continues, and
//! how the morph eases with Windows' animations off. The renderer and `App`
//! only apply what is decided here. Nothing in this file runs a timer: every
//! transition rides the existing `FrameClock` for as long as it lasts
//! (`docs/memory/panel-linger-and-motion.md`).

use std::time::Instant;

use chrono::NaiveDate;

use super::geometry::ease_out_back;
use crate::clean::CleanMode;
use crate::model::{CalView, Page};
use crate::state::MolePhase;

/// A sub-view `Reveal`: the new content fades in over this long.
pub const REVEAL_MS: u64 = 160;
/// How far the new content travels while it fades in, in logical px.
pub const REVEAL_OFFSET: f32 = 8.0;
/// A page change that is not a slide: the two pages cross-fade this long.
pub const PAGE_FADE_MS: u64 = 160;

/// A cross-fade at `progress` (0..1) as the renderer gets it: `None` once it
/// has reached its end, so its last frame is drawn settled and registers the
/// page's click regions (the frame clock stops right after it).
pub fn page_fade_frame(progress: f32) -> Option<f32> {
    (progress < 1.0).then_some(progress)
}

/// Whether a panel frame registers its click regions: only the open panel's
/// settled frame, with no slide or cross-fade running (the controls are not
/// where the regions say yet, and the old page's are still under the pointer).
/// A cross-fade at its end (`t >= 1`) counts as none.
pub fn panel_regions_wanted(settled: bool, closing: bool, slide: f32, fade: Option<f32>) -> bool {
    settled && !closing && slide == 0.0 && fade.and_then(page_fade_frame).is_none()
}

/// Cubic ease-out: fast start, gentle landing, no overshoot.
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// How the shape morph eases: with a slight overshoot, or plain ease-out
/// when Windows' client-area animations are off.
pub fn morph_ease(reduced: bool, t: f32) -> f32 {
    if reduced {
        ease_out_cubic(t)
    } else {
        ease_out_back(t)
    }
}

/// How visible the content of a morphing shape is: invisible while the
/// shape is only starting, fully there for the last third.
pub fn content_alpha(progress: f32) -> f32 {
    let u = ((progress - 0.3) / 0.7).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// `content_alpha` for a fade that starts at `from` (a morph retargeted
/// while its content was fading in carries on from there, never back to 0).
pub fn content_alpha_from(from: f32, progress: f32) -> f32 {
    let from = from.clamp(0.0, 1.0);
    from + (1.0 - from) * content_alpha(progress)
}

/// The height of the compact shapes: where a panel's content is invisible
/// unless the morph says otherwise (`fade_floor`).
pub const COMPACT_H: f32 = 34.0;

/// How visible a panel's content is while its shape is `h` tall on its way
/// between `floor_h` (the height of the view it grows from or shrinks into:
/// compact, a notice, a screenshot notice) and `rest_h`: invisible at
/// `floor_h`, fully there by 70 % of the way up, the same on the way up and
/// on the way down. So the closing panel reaches 0 exactly when its morph
/// ends, whatever it shrinks into.
pub fn panel_fade(h: f32, rest_h: f32, floor_h: f32) -> f32 {
    let span = rest_h - floor_h;
    if span <= 0.0 {
        return 1.0;
    }
    ((h - floor_h) / span / 0.7).clamp(0.0, 1.0)
}

/// The height at which the panel's content is invisible during a morph that
/// starts at `shape_h` (a retarget) and heads for `to_h`.
///
/// - Not heading for a panel (it closes into a compact view, a notice or a
///   screenshot notice): the target's height, so the fade ends with the morph.
/// - Opening or resizing a panel: the floor it already had while the panel
///   was on screen (`carried`), else the height of the visible view it grows
///   from, else the compact height (it comes from nothing).
pub fn fade_floor(
    to_panel: bool,
    to_h: f32,
    from_panel: bool,
    carried: f32,
    from_visible: bool,
    shape_h: f32,
) -> f32 {
    if !to_panel {
        to_h
    } else if from_panel {
        carried
    } else if from_visible {
        shape_h.max(COMPACT_H)
    } else {
        COMPACT_H
    }
}

/// The most pages a slide keeps on their way out.
pub const SLIDE_TRAIL: usize = 5;
/// The farthest the page coming in starts, in panel widths. Only a long
/// fast spin compounds past it; the strip then jumps by the excess.
pub const SLIDE_MAX: f32 = 4.0;

/// The pages of a slide as one strip: the page coming in starts `from`
/// panel widths away from rest and eases to 0, and the pages going out sit
/// a whole number of widths from it (`trail`), so they move with it. A slide
/// that starts during another continues the same strip: nothing on screen
/// moves or disappears, whichever way the new page comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideStrip {
    /// Where the page coming in starts, in panel widths (+: on the right).
    pub from: f32,
    /// The pages going out, each with its place in panel widths from the
    /// page coming in. Only those that show at some point of the slide.
    pub trail: [Option<(Page, i8)>; SLIDE_TRAIL],
}

impl SlideStrip {
    /// A slide to a page that comes from `dir` (+1: it follows on the right),
    /// starting while the page on screen (`current`, the one the last slide
    /// brought in) is `offset` widths from rest; `prev` is the slide that
    /// runs, `None` at rest. The new page sits one width beyond `current`,
    /// so `current` and the pages still going out keep their places.
    pub fn next(prev: Option<&SlideStrip>, offset: f32, current: Page, dir: f32) -> Self {
        let step: i8 = if dir < 0.0 { -1 } else { 1 };
        let raw = offset + f32::from(step);
        let from = raw.clamp(-SLIDE_MAX, SLIDE_MAX);
        // Shown at some point while the page coming in eases from `from` to 0.
        let (lo, hi) = if from < 0.0 { (from, 0.0) } else { (0.0, from) };
        let shows = |rel: i8| {
            let rel = f32::from(rel);
            rel + lo < 1.0 && rel + hi > -1.0
        };
        let older = prev
            .into_iter()
            .flat_map(|p| p.trail.into_iter().flatten())
            .map(|(page, rel)| (page, rel.saturating_sub(step)));
        let mut trail = [None; SLIDE_TRAIL];
        // The new page takes slot 0; a page going out there is covered by it.
        for (slot, entry) in trail.iter_mut().zip(
            std::iter::once((current, -step))
                .chain(older)
                .filter(|&(_, rel)| rel != 0 && shows(rel)),
        ) {
            *slot = Some(entry);
        }
        Self { from, trail }
    }

    /// The pages going out that show while the page coming in is `offset`
    /// widths from rest, each with its x in panel widths. Never more than
    /// two: the pages are a whole width apart.
    pub fn visible(&self, offset: f32) -> [Option<(Page, f32)>; 2] {
        let mut out = [None; 2];
        let shown = self
            .trail
            .into_iter()
            .flatten()
            .map(|(page, rel)| (page, offset + f32::from(rel)))
            .filter(|&(_, x)| x.abs() < 1.0);
        for (slot, entry) in out.iter_mut().zip(shown) {
            *slot = Some(entry);
        }
        out
    }
}

/// What a page change looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMotion {
    /// Instant: the panel was not open before or after, nothing changed, or
    /// Windows' animations are off.
    None,
    /// The pages slide.
    Slide,
    /// The pages cross-fade (the search comes and goes over a page).
    Fade,
}

/// Decides how `before` becomes `after`. Only a change while the panel
/// showed before **and** after animates: the opening and closing morphs carry
/// the rest. The search is not in the order of pages, so it fades.
pub fn page_motion(
    before: Page,
    after: Page,
    panel_before: bool,
    panel_after: bool,
    animations: bool,
) -> PageMotion {
    if !animations || before == after || !panel_before || !panel_after {
        return PageMotion::None;
    }
    if before == Page::Search || after == Page::Search {
        PageMotion::Fade
    } else {
        PageMotion::Slide
    }
}

/// Which way a `Reveal` travels: the new content starts `REVEAL_OFFSET` away
/// on the opposite side and fades in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealDir {
    /// A later date or page: it comes from the right.
    Forward,
    /// An earlier one: from the left.
    Back,
    /// Something opens or gets finer (the cleaner, a day): from below.
    Open,
    /// Something closes or gets coarser: from above.
    Close,
    /// No travel, only the fade.
    Fade,
}

impl RevealDir {
    /// Where the content starts, from its place (x, y).
    pub fn start_offset(self) -> (f32, f32) {
        match self {
            RevealDir::Forward => (REVEAL_OFFSET, 0.0),
            RevealDir::Back => (-REVEAL_OFFSET, 0.0),
            RevealDir::Open => (0.0, REVEAL_OFFSET),
            RevealDir::Close => (0.0, -REVEAL_OFFSET),
            RevealDir::Fade => (0.0, 0.0),
        }
    }
}

/// What a reveal draws at one moment: the content's opacity and offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RevealFrame {
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
}

/// A sub-view switch fading in. It starts from the opacity the content it
/// replaces had when it was cut short (`from_alpha`), so a second switch
/// during one never blinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reveal {
    pub start: Instant,
    pub dir: RevealDir,
    pub from_alpha: f32,
}

impl Reveal {
    pub fn new(start: Instant, dir: RevealDir, from_alpha: f32) -> Self {
        Self {
            start,
            dir,
            from_alpha: from_alpha.clamp(0.0, 1.0),
        }
    }

    /// 0 at the start, 1 once `REVEAL_MS` has passed.
    pub fn progress(&self, now: Instant) -> f32 {
        let ms = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        (ms / REVEAL_MS as f32).clamp(0.0, 1.0)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    pub fn frame(&self, now: Instant) -> RevealFrame {
        reveal_frame(self.dir, self.from_alpha, self.progress(now))
    }
}

/// The reveal at `progress` (0..1): opacity eases out from `from_alpha`, and
/// the offset closes with the same curve.
pub fn reveal_frame(dir: RevealDir, from_alpha: f32, progress: f32) -> RevealFrame {
    let e = ease_out_cubic(progress);
    let (dx, dy) = dir.start_offset();
    RevealFrame {
        alpha: from_alpha + (1.0 - from_alpha) * e,
        dx: dx * (1.0 - e),
        dy: dy * (1.0 - e),
    }
}

/// What the renderer needs besides the page itself to draw a transition.
/// All at rest by default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transit {
    /// The page content fading in over a sub-view switch.
    pub reveal: Option<RevealFrame>,
    /// 0..1 while two pages cross-fade (the page before at `1 - t`).
    pub page_fade: Option<f32>,
    /// Opacity of a compact view's or notice's content while its shape
    /// morphs (1 at rest).
    pub content_alpha: f32,
    /// The panel closing: the page it showed, its content fading out with
    /// the shape's height (`panel_fade`) while the shape shrinks.
    pub leaving: Option<Page>,
    /// The height at which a panel's content is invisible during this morph
    /// (`fade_floor`): the closing panel fades to the target's height.
    pub fade_floor_h: f32,
}

impl Default for Transit {
    fn default() -> Self {
        Self {
            reveal: None,
            page_fade: None,
            content_alpha: 1.0,
            leaving: None,
            fade_floor_h: COMPACT_H,
        }
    }
}

/// What differs inside a page, for deciding a `Reveal`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubView {
    pub cal_view: CalView,
    pub cal_anchor: NaiveDate,
    /// "+N lagi" clicks on the day view.
    pub day_page: usize,
    pub notes_page: usize,
    pub reminders_page: usize,
    /// The cleaner over the System page and its mode.
    pub cleaner: Option<CleanMode>,
    /// Which stage Mole is at (`mole_code`), and whether its log shows.
    pub mole: u8,
    pub mole_log: bool,
    pub cli_menu: bool,
}

/// Mole's stage without its payload: a new dry-run with other options
/// (same stage) is not a new view.
pub fn mole_code(phase: MolePhase) -> u8 {
    match phase {
        MolePhase::Unknown => 0,
        MolePhase::Detecting => 1,
        MolePhase::Missing => 2,
        MolePhase::Scanning(_) => 3,
        MolePhase::Previewed { .. } => 4,
        MolePhase::Running => 5,
        MolePhase::Finished { .. } => 6,
    }
}

fn cal_rank(view: CalView) -> u8 {
    match view {
        CalView::Month => 0,
        CalView::Week => 1,
        CalView::Day => 2,
    }
}

/// What the panel looked like around one applied event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    /// The view was the panel.
    pub panel: bool,
    pub page: Page,
    pub sub: SubView,
}

/// What an applied event changes on screen inside the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    None,
    Slide,
    Fade,
    Reveal(RevealDir),
}

/// The transition for `before` becoming `after`. A page change wins over a
/// sub-view switch; with animations off there is none.
pub fn transition(before: &Snap, after: &Snap, animations: bool) -> Transition {
    match page_motion(
        before.page,
        after.page,
        before.panel,
        after.panel,
        animations,
    ) {
        PageMotion::Slide => return Transition::Slide,
        PageMotion::Fade => return Transition::Fade,
        PageMotion::None => {}
    }
    if !animations || !before.panel || !after.panel || before.page != after.page {
        return Transition::None;
    }
    match reveal_dir(&before.sub, &after.sub, after.page) {
        Some(dir) => Transition::Reveal(dir),
        None => Transition::None,
    }
}

/// The direction of the sub-view switch on `page`, `None` when the content
/// stays as it was.
pub fn reveal_dir(before: &SubView, after: &SubView, page: Page) -> Option<RevealDir> {
    match page {
        Page::Calendar => {
            if before.cal_view != after.cal_view {
                // A finer range opens from below, a coarser one closes.
                return Some(if cal_rank(after.cal_view) > cal_rank(before.cal_view) {
                    RevealDir::Open
                } else {
                    RevealDir::Close
                });
            }
            if before.cal_anchor != after.cal_anchor {
                return Some(if after.cal_anchor > before.cal_anchor {
                    RevealDir::Forward
                } else {
                    RevealDir::Back
                });
            }
            (before.day_page != after.day_page).then_some(RevealDir::Forward)
        }
        Page::Notes => (before.notes_page != after.notes_page).then_some(RevealDir::Forward),
        Page::Reminders => {
            (before.reminders_page != after.reminders_page).then_some(RevealDir::Forward)
        }
        Page::System => match (before.cleaner, after.cleaner) {
            (None, Some(_)) => Some(RevealDir::Open),
            (Some(_), None) => Some(RevealDir::Close),
            (Some(a), Some(b)) if a != b => Some(RevealDir::Open),
            (Some(_), Some(_))
                if before.mole != after.mole || before.mole_log != after.mole_log =>
            {
                // Mole's next stage or its log: the content changes in
                // place, so it only fades.
                Some(RevealDir::Fade)
            }
            _ => None,
        },
        Page::Cli => match (before.cli_menu, after.cli_menu) {
            // Only the menu appearing reveals; it closes at once.
            (false, true) => Some(RevealDir::Open),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sub() -> SubView {
        SubView {
            cal_view: CalView::Week,
            cal_anchor: NaiveDate::from_ymd_opt(2026, 10, 29).expect("date"),
            day_page: 0,
            notes_page: 0,
            reminders_page: 0,
            cleaner: None,
            mole: 0,
            mole_log: false,
            cli_menu: false,
        }
    }

    fn snap(panel: bool, page: Page, sub: SubView) -> Snap {
        Snap { panel, page, sub }
    }

    #[test]
    fn reveal_runs_from_nothing_to_whole_in_160_ms() {
        let t0 = Instant::now();
        let r = Reveal::new(t0, RevealDir::Open, 0.0);
        let start = r.frame(t0);
        assert_eq!(start.alpha, 0.0);
        assert_eq!((start.dx, start.dy), (0.0, REVEAL_OFFSET));
        assert!(!r.done(t0 + Duration::from_millis(159)));
        let end = r.frame(t0 + Duration::from_millis(REVEAL_MS));
        assert!(r.done(t0 + Duration::from_millis(REVEAL_MS)));
        assert_eq!((end.alpha, end.dx, end.dy), (1.0, 0.0, 0.0));
        // Ease-out: well past half the fade by half the time.
        let half = r.frame(t0 + Duration::from_millis(REVEAL_MS / 2));
        assert!(half.alpha > 0.8 && half.alpha < 1.0, "{}", half.alpha);
        assert!(half.dy < REVEAL_OFFSET / 4.0);
    }

    #[test]
    fn reveal_travels_in_the_direction_of_the_change() {
        assert_eq!(RevealDir::Forward.start_offset(), (REVEAL_OFFSET, 0.0));
        assert_eq!(RevealDir::Back.start_offset(), (-REVEAL_OFFSET, 0.0));
        assert_eq!(RevealDir::Open.start_offset(), (0.0, REVEAL_OFFSET));
        assert_eq!(RevealDir::Close.start_offset(), (0.0, -REVEAL_OFFSET));
        assert_eq!(RevealDir::Fade.start_offset(), (0.0, 0.0));
    }

    #[test]
    fn a_reveal_cut_short_starts_from_the_opacity_it_had() {
        let t0 = Instant::now();
        let first = Reveal::new(t0, RevealDir::Forward, 0.0);
        let mid = first.frame(t0 + Duration::from_millis(40)).alpha;
        let second = Reveal::new(t0 + Duration::from_millis(40), RevealDir::Forward, mid);
        assert!((second.frame(t0 + Duration::from_millis(40)).alpha - mid).abs() < 1e-6);
        assert_eq!(
            second
                .frame(t0 + Duration::from_millis(40 + REVEAL_MS))
                .alpha,
            1.0
        );
    }

    #[test]
    fn no_transition_is_longer_than_the_morph() {
        assert!(REVEAL_MS <= crate::ui::geometry::ANIM_MS);
        assert!(PAGE_FADE_MS <= crate::ui::geometry::ANIM_MS);
    }

    #[test]
    fn the_last_cross_fade_frame_is_settled() {
        assert_eq!(page_fade_frame(0.0), Some(0.0));
        assert_eq!(page_fade_frame(0.5), Some(0.5));
        assert_eq!(page_fade_frame(1.0), None);
        // Its end frame registers the page's regions, a frame inside it not.
        assert!(panel_regions_wanted(true, false, 0.0, Some(1.0)));
        assert!(panel_regions_wanted(true, false, 0.0, None));
        assert!(!panel_regions_wanted(true, false, 0.0, Some(0.99)));
        assert!(!panel_regions_wanted(true, false, 0.2, None));
        assert!(!panel_regions_wanted(true, true, 0.0, None));
        assert!(!panel_regions_wanted(false, false, 0.0, None));
    }

    /// Where `page` is drawn at `offset`, `None` when it is not.
    fn drawn_at(strip: &SlideStrip, offset: f32, page: Page) -> Option<f32> {
        strip
            .visible(offset)
            .into_iter()
            .flatten()
            .find(|(p, _)| *p == page)
            .map(|(_, x)| x)
    }

    #[test]
    fn a_slide_from_rest_brings_the_page_in_beside_the_old_one() {
        let s = SlideStrip::next(None, 0.0, Page::Calendar, 1.0);
        assert_eq!(s.from, 1.0);
        // The old page sits one width to the left of the new one.
        assert_eq!(drawn_at(&s, 1.0, Page::Calendar), Some(0.0));
        assert_eq!(drawn_at(&s, 0.4, Page::Calendar), Some(-0.6));
        // At rest it is gone.
        assert_eq!(drawn_at(&s, 0.0, Page::Calendar), None);
        let back = SlideStrip::next(None, 0.0, Page::Media, -1.0);
        assert_eq!(back.from, -1.0);
        assert_eq!(drawn_at(&back, -0.4, Page::Media), Some(0.6));
    }

    #[test]
    fn a_slide_that_starts_mid_slide_keeps_every_page_on_screen_in_place() {
        // A to B, two notches 20 ms apart: B is 0.75 in, A still covers
        // three quarters of the panel at -0.25.
        let (a, b) = (Page::Calendar, Page::Media);
        let first = SlideStrip::next(None, 0.0, a, 1.0);
        let offset = 0.75;
        assert_eq!(drawn_at(&first, offset, a), Some(-0.25));
        // The next notch, the same way: C comes in beyond B. Neither A nor B
        // moves or disappears.
        let second = SlideStrip::next(Some(&first), offset, b, 1.0);
        assert_eq!(second.from, 1.75);
        assert_eq!(drawn_at(&second, second.from, a), Some(-0.25));
        assert_eq!(drawn_at(&second, second.from, b), Some(0.75));
        // Halfway, B covers the middle and C comes in after it.
        let mid = drawn_at(&second, 0.9, b).expect("B shows");
        assert!((mid + 0.1).abs() < 1e-5, "{mid}");
        assert_eq!(drawn_at(&second, 0.9, a), None);
        // A notch back instead: A comes back where it is, B stays beside it.
        let back = SlideStrip::next(Some(&first), offset, b, -1.0);
        assert_eq!(back.from, -0.25);
        assert_eq!(drawn_at(&back, back.from, b), Some(0.75));
        // A is the page coming in now (drawn as the page itself, not twice).
        assert_eq!(drawn_at(&back, back.from, a), None);
    }

    #[test]
    fn a_reversal_after_two_notches_comes_back_on_the_right_side() {
        // Two quick notches: C is 1.5 away on the right, B at 0.5.
        let (a, b, c) = (Page::Calendar, Page::Media, Page::Notes);
        let first = SlideStrip::next(None, 0.0, a, 1.0);
        let second = SlideStrip::next(Some(&first), 0.5, b, 1.0);
        assert_eq!(drawn_at(&second, 1.5, b), Some(0.5));
        // Back again: B comes in from where it is (0.5), not from the left;
        // C, off screen at 1.5, is not drawn.
        let back = SlideStrip::next(Some(&second), 1.5, c, -1.0);
        assert_eq!(back.from, 0.5);
        assert_eq!(drawn_at(&back, 0.5, c), None);
        // A, beyond B on the left, slides on with it.
        assert_eq!(drawn_at(&back, 0.5, a), Some(-0.5));
    }

    #[test]
    fn a_long_spin_stays_bounded() {
        let pages = [Page::Calendar, Page::Media, Page::Notes, Page::Reminders];
        let mut strip: Option<SlideStrip> = None;
        let mut offset = 0.0;
        for i in 0..20 {
            let next = SlideStrip::next(strip.as_ref(), offset, pages[i % pages.len()], 1.0);
            assert!(next.from <= SLIDE_MAX);
            // Never more than two pages going out on screen.
            assert!(next.visible(next.from).into_iter().flatten().count() <= 2);
            offset = next.from * 0.9;
            strip = Some(next);
        }
    }

    #[test]
    fn page_changes_slide_only_while_the_panel_shows() {
        use PageMotion::*;
        assert_eq!(
            page_motion(Page::Calendar, Page::Media, true, true, true),
            Slide
        );
        // The Clipboard tile, a disk warning's "Buka": a page change like any.
        assert_eq!(
            page_motion(Page::Shortcuts, Page::Clipboard, true, true, true),
            Slide
        );
        assert_eq!(
            page_motion(Page::Calendar, Page::System, true, true, true),
            Slide
        );
        // Nothing changed, or the panel was not (or is no longer) there.
        assert_eq!(
            page_motion(Page::Media, Page::Media, true, true, true),
            None
        );
        assert_eq!(
            page_motion(Page::Calendar, Page::Media, false, true, true),
            None
        );
        assert_eq!(
            page_motion(Page::Calendar, Page::Media, true, false, true),
            None
        );
        // The search fades in and out, never slides.
        assert_eq!(
            page_motion(Page::Media, Page::Search, true, true, true),
            Fade
        );
        assert_eq!(
            page_motion(Page::Search, Page::Media, true, true, true),
            Fade
        );
        // Windows' animations off: instant.
        assert_eq!(
            page_motion(Page::Calendar, Page::Media, true, true, false),
            None
        );
        assert_eq!(
            page_motion(Page::Media, Page::Search, true, true, false),
            None
        );
    }

    #[test]
    fn a_calendar_range_change_reveals_in_its_direction() {
        let before = sub();
        let at = |f: fn(&mut SubView)| {
            let mut after = before;
            f(&mut after);
            transition(
                &snap(true, Page::Calendar, before),
                &snap(true, Page::Calendar, after),
                true,
            )
        };
        let later = |s: &mut SubView| s.cal_anchor = s.cal_anchor + chrono::Days::new(7);
        let earlier = |s: &mut SubView| s.cal_anchor = s.cal_anchor - chrono::Days::new(7);
        assert_eq!(at(later), Transition::Reveal(RevealDir::Forward));
        assert_eq!(at(earlier), Transition::Reveal(RevealDir::Back));
        // Week to day opens from below, week to month closes.
        assert_eq!(
            at(|s| s.cal_view = CalView::Day),
            Transition::Reveal(RevealDir::Open)
        );
        assert_eq!(
            at(|s| s.cal_view = CalView::Month),
            Transition::Reveal(RevealDir::Close)
        );
        assert_eq!(
            at(|s| s.day_page = 1),
            Transition::Reveal(RevealDir::Forward)
        );
        assert_eq!(at(|_| {}), Transition::None);
    }

    #[test]
    fn the_cleaner_and_mole_stages_reveal() {
        let before = sub();
        let mut open = before;
        open.cleaner = Some(CleanMode::Own);
        let page = Page::System;
        assert_eq!(
            transition(&snap(true, page, before), &snap(true, page, open), true),
            Transition::Reveal(RevealDir::Open)
        );
        assert_eq!(
            transition(&snap(true, page, open), &snap(true, page, before), true),
            Transition::Reveal(RevealDir::Close)
        );
        let mut mole = open;
        mole.cleaner = Some(CleanMode::Mole);
        mole.mole = mole_code(MolePhase::Detecting);
        let mut scanned = mole;
        scanned.mole = mole_code(MolePhase::Running);
        assert_eq!(
            transition(&snap(true, page, mole), &snap(true, page, scanned), true),
            Transition::Reveal(RevealDir::Fade)
        );
        // The same stage again (an option ticked, a new dry-run) is not a view.
        assert_eq!(
            transition(&snap(true, page, scanned), &snap(true, page, scanned), true),
            Transition::None
        );
    }

    #[test]
    fn a_page_change_wins_over_a_sub_view_switch_and_off_means_instant() {
        let a = sub();
        let mut b = a;
        b.cal_anchor = b.cal_anchor + chrono::Days::new(1);
        assert_eq!(
            transition(
                &snap(true, Page::Calendar, a),
                &snap(true, Page::Media, b),
                true
            ),
            Transition::Slide
        );
        assert_eq!(
            transition(
                &snap(true, Page::Calendar, a),
                &snap(true, Page::Calendar, b),
                false
            ),
            Transition::None
        );
        // The panel opening is the morph's: no reveal inside it.
        assert_eq!(
            transition(
                &snap(false, Page::Calendar, a),
                &snap(true, Page::Calendar, b),
                true
            ),
            Transition::None
        );
    }

    #[test]
    fn reduced_motion_morphs_with_a_plain_ease_out() {
        // Plain: never past the target.
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            assert!(morph_ease(true, t) <= 1.0 + 1e-6, "t {t}");
        }
        assert_eq!(morph_ease(true, 0.0), 0.0);
        assert_eq!(morph_ease(true, 1.0), 1.0);
        // The normal one overshoots, as it always did.
        assert!(morph_ease(false, 0.8) > 1.0);
        assert_eq!(morph_ease(false, 0.5), ease_out_back(0.5));
    }

    #[test]
    fn panel_content_fades_the_same_way_up_and_down() {
        let (rest, compact) = (290.0, 34.0);
        assert_eq!(panel_fade(compact, rest, compact), 0.0);
        assert_eq!(panel_fade(rest, rest, compact), 1.0);
        // Fully there by 70 % of the height, whichever way it moves.
        assert_eq!(
            panel_fade(compact + 0.7 * (rest - compact), rest, compact),
            1.0
        );
        let mid = panel_fade(100.0, rest, compact);
        assert!(mid > 0.0 && mid < 1.0);
    }

    #[test]
    fn the_closing_fade_reaches_zero_at_the_morphs_target() {
        use crate::ui::geometry::SHOT_H;
        let rest = 290.0;
        // Compact (34), Notice (76) and NoticeShot (SHOT_H): whatever the
        // panel shrinks into, it is fully there at its own height, falls
        // all the way and is gone exactly where the morph ends.
        for target in [COMPACT_H, 76.0, SHOT_H] {
            assert_eq!(panel_fade(rest, rest, target), 1.0, "{target}");
            assert_eq!(panel_fade(target, rest, target), 0.0, "{target}");
            let mut last = 1.0;
            for i in 0..=100 {
                let h = rest + (target - rest) * i as f32 / 100.0;
                let a = panel_fade(h, rest, target);
                assert!(a <= last, "{target}: {a} after {last}");
                last = a;
            }
            assert_eq!(last, 0.0, "{target}");
            // Overshooting the target stays at 0.
            assert_eq!(panel_fade(target - 3.0, rest, target), 0.0, "{target}");
        }
        // The old fixed floor left the Notice's morph at about 23 %.
        assert!(panel_fade(76.0, rest, COMPACT_H) > 0.2);
    }

    #[test]
    fn the_fade_floor_follows_the_morphs_target() {
        use crate::ui::geometry::SHOT_H;
        // Closing into a view: its height.
        assert_eq!(fade_floor(false, 76.0, true, 34.0, true, 200.0), 76.0);
        assert_eq!(fade_floor(false, SHOT_H, true, 34.0, true, 200.0), SHOT_H);
        assert_eq!(fade_floor(false, 34.0, true, 76.0, true, 200.0), 34.0);
        // A panel resizing keeps the floor it opened with.
        assert_eq!(fade_floor(true, 440.0, true, 76.0, true, 290.0), 76.0);
        // Opening from a notice starts its content at 0 from there.
        assert_eq!(fade_floor(true, 290.0, false, 34.0, true, 76.0), 76.0);
        // From a compact view or from nothing: the compact height.
        assert_eq!(fade_floor(true, 290.0, false, 34.0, true, 34.0), COMPACT_H);
        assert_eq!(fade_floor(true, 290.0, false, 76.0, false, 90.0), COMPACT_H);
    }

    #[test]
    fn compact_content_fades_in_with_its_morph() {
        assert_eq!(content_alpha(0.0), 0.0);
        assert_eq!(content_alpha(0.3), 0.0);
        assert_eq!(content_alpha(1.0), 1.0);
        let mut last = 0.0;
        for i in 0..=20 {
            let a = content_alpha(i as f32 / 20.0);
            assert!(a >= last);
            last = a;
        }
    }

    #[test]
    fn a_retargeted_fade_carries_on_from_where_it_was() {
        assert_eq!(content_alpha_from(0.0, 0.5), content_alpha(0.5));
        assert_eq!(content_alpha_from(0.6, 0.0), 0.6);
        assert_eq!(content_alpha_from(0.6, 0.3), 0.6);
        assert_eq!(content_alpha_from(0.6, 1.0), 1.0);
        let mut last = 0.6;
        for i in 0..=20 {
            let a = content_alpha_from(0.6, i as f32 / 20.0);
            assert!(a >= last);
            last = a;
        }
    }
}
