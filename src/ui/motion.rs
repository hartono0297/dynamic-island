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

/// How visible a panel's content is while its shape is `h` tall on its way
/// between `compact_h` and `rest_h`: fully there by 70 % of the height, the
/// same on the way up and on the way down.
pub fn panel_fade(h: f32, rest_h: f32, compact_h: f32) -> f32 {
    let span = rest_h - compact_h;
    if span <= 0.0 {
        return 1.0;
    }
    ((h - compact_h) / span / 0.7).clamp(0.0, 1.0)
}

/// Where a slide that starts now begins. A slide that runs still has its page
/// `offset` (in panel widths) away from rest; the page on screen keeps that
/// place and the new one comes from `dir` (+1: it follows on the right) beyond
/// it, so nothing jumps. At rest (`offset` 0) it is the plain ±1.
pub fn slide_start(offset: f32, dir: f32) -> f32 {
    (offset + dir).clamp(-2.0, 2.0)
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
}

impl Default for Transit {
    fn default() -> Self {
        Self {
            reveal: None,
            page_fade: None,
            content_alpha: 1.0,
            leaving: None,
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

    #[test]
    fn a_slide_that_starts_mid_slide_continues_from_the_offset() {
        // At rest it is the plain ±1.
        assert_eq!(slide_start(0.0, 1.0), 1.0);
        assert_eq!(slide_start(0.0, -1.0), -1.0);
        // Mid-slide (the page on screen 0.4 panel widths to the right): the
        // page that was drawn at `offset` keeps its place, wherever the next
        // one comes from. The renderer draws the old page at `start - dir`.
        for (offset, dir) in [(0.4, 1.0), (0.4, -1.0), (-0.7, 1.0), (-0.7, -1.0)] {
            let start = slide_start(offset, dir);
            assert!(
                (start - dir - offset).abs() < 1e-6,
                "offset {offset} dir {dir}: the old page jumped"
            );
        }
        // Never further than two panel widths.
        assert_eq!(slide_start(1.0, 1.0), 2.0);
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
}
