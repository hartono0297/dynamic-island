//! Drawing of the open panel's pages, the compact pomodoro, the waiting strip
//! and the page dots. A child module of `render`, so it shares the renderer's
//! private state and drawing helpers. Positions come from `panel_layout`;
//! colours only from the island `Palette`, radii from `theme::radius`.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL,
    D2D1_ELLIPSE, D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, ID2D1Geometry,
};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::core::Result;
use windows_numerics::{Matrix3x2, Vector2};

use super::{CliView, Frame, Progress, Renderer, color, d2d_rect, mix};
use crate::calendar::{CalEvent, CalStatus, EventTime, events_on, range_for};
use crate::cli_panel::{LineKind, RunState, echo_command, line_kind, visible_range};
use crate::config::Style;
use crate::i18n::{self, Key, Lang, text};
use crate::model::{CalView, Hit, MediaInfo, Notice, NoticeKind, Page};
use crate::notes::Note;
use crate::pomodoro::{Phase, PomoStatus, Pomodoro};
use crate::reminders::{Reminder, Repeat, local_time};
use crate::shortcuts;
use crate::theme::{self, Palette, radius};
use crate::ui::geometry::{CLAUDE, ExpandedLayout, Rect, ShapeKind, ShapeRect, shape_for};
use crate::ui::icons::Icon;
use crate::ui::month_grid::{CellCanvas, CellLook, DayState, Filled, day_geometry, paint_day};
use crate::ui::panel_layout::{
    CLI_LINE_H, CLI_TERM_RADIUS, CalendarLayout, CliHead, CliLayout, HOUR_GUTTER, ListLayout,
    PageLayout, PomodoroLayout, RING_STROKE, SEG_PAD, ShortcutsLayout, WEEK_HEAD_H,
    calendar_layout, cli_layout, cli_suggestions, media_panel_layout, media_progress_layout,
    notes_layout, page_dots, pomodoro_compact_layout, pomodoro_compact_layout_robot,
    pomodoro_layout, regions, reminders_layout, shortcuts_layout, tile_parts, waiting_strip,
};

/// Height of the compact shapes the panel grows from.
const COMPACT_H: f32 = 34.0;
/// A heading's line (Fraunces 20) is taller than the 22 px header row: its
/// box grows by this much above and below so descenders are not clipped.
const HEADING_BLEED: f32 = 4.0;
/// Tints over the island: an all-day band, a day view block, a week chip.
const BAND_TINT: f32 = 0.16;
const BLOCK_TINT: f32 = 0.2;
const CHIP_TINT: f32 = 0.14;
/// How far block text moves from the event colour towards the text colour.
const BLOCK_TEXT_MIX: f32 = 0.55;
/// The "Terlewat" tag's fill under its ochre text.
const TAG_TINT: f32 = 0.16;
/// A note card's time, a step quieter than the snippet.
const NOTE_TIME_ALPHA: f32 = 0.75;
/// The round playhead on the Music page's progress bar.
const PROGRESS_KNOB: f32 = 10.0;

/// The CLI page after `.hchip`, `.stop`, `.spin`, `.lockb`, `.sugg` and
/// `.caret` in docs/design/pengingat-cli.html.
const CHIP_PAD_X: f32 = 9.0;
const SPIN: f32 = 12.0;
const SPIN_GAP: f32 = 5.0;
const STOP_SQUARE: f32 = 7.0;
const STOP_GAP: f32 = 5.0;
const STOP_PAD_X: f32 = 10.0;
/// The lock banner hangs from the panel's top edge, this far over the header.
const CLI_LOCK_ABOVE: f32 = 16.0;
const LOCK_H: f32 = 16.0;
const LOCK_PAD_X: f32 = 10.0;
const LOCK_DOT: f32 = 6.0;
const LOCK_GAP: f32 = 5.0;
const LOCK_RADIUS: f32 = 8.0;
/// `›` and the space after it in front of an echoed command.
const CLI_MARK_W: f32 = 14.0;
const SUGGEST_RADIUS: f32 = 12.0;
const SUGGEST_ROW_RADIUS: f32 = 8.0;
const SUGGEST_PAD_X: f32 = 9.0;
const HINT_GAP: f32 = 8.0;
const CARET_W: f32 = 7.0;
const CARET_H: f32 = 14.0;
const FOCUS_W: f32 = 1.5;

/// How a CLI header chip looks: the folder (path ellipsis) and the shell in
/// mono on the raised fill; the run's state tinted.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ChipLook {
    Mono,
    Path,
    Run,
    Ok,
    Err,
}

#[derive(Clone, Debug, PartialEq)]
struct CliChip {
    text: String,
    look: ChipLook,
}

/// The header chips left to right: folder and shell; "Berjalan · 0:42"
/// while a command runs (Stop follows it); folder, the result and the shell
/// once one ended.
fn cli_chips(lang: Lang, cli: &CliView, now: Instant) -> Vec<CliChip> {
    let chip = |text: String, look| CliChip { text, look };
    let folder = chip(cli.cwd.to_owned(), ChipLook::Path);
    let shell = chip(cli.shell.name().to_owned(), ChipLook::Mono);
    match cli.run {
        RunState::Idle => vec![folder, shell],
        RunState::Running { since } => {
            let elapsed = now.saturating_duration_since(since);
            vec![chip(i18n::cli_running(lang, elapsed), ChipLook::Run)]
        }
        RunState::Ended { code, killed, took } => {
            let look = if code == Some(0) && !killed {
                ChipLook::Ok
            } else {
                ChipLook::Err
            };
            let result = chip(i18n::cli_result(lang, code, killed, took), look);
            vec![folder, result, shell]
        }
    }
}

/// The first `n` UTF-16 units of `s` (an EDIT's offsets), never splitting a
/// character.
fn utf16_prefix(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// A track time as the player shows it: "m:ss", or "h:mm:ss" from an hour.
fn track_time(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl Renderer {
    /// A page title in the header row.
    fn heading(&self, title: &str, header: Rect) {
        let area = Rect {
            y: header.y - HEADING_BLEED,
            h: header.h + 2.0 * HEADING_BLEED,
            ..header
        };
        self.text(title, &self.fonts.heading, area, 1.0);
    }

    /// The panel: content laid out for the resting panel shape, clipped to the
    /// current (possibly still morphing) shape and faded in while it grows.
    /// Clickable regions exist only once the shape has settled and no page
    /// is sliding.
    pub(super) fn paint_panel(
        &mut self,
        style: Style,
        shape: ShapeRect,
        frame: &Frame,
    ) -> Result<()> {
        let kind = if frame.page == Page::Cli && frame.cli.tall {
            ShapeKind::PanelTall
        } else {
            ShapeKind::Panel
        };
        let rest = shape_for(style, kind);
        let settled = shape == rest;
        // Content is fully visible by ~70 % of the growth, so the panel reads
        // as open while the shape is still easing into place.
        let fade = if settled {
            1.0
        } else {
            ((shape.h - COMPACT_H) / (rest.h - COMPACT_H) / 0.7).clamp(0.0, 1.0)
        };
        if fade <= 0.0 {
            return Ok(());
        }
        let mask = self.shape_geometry(style, shape)?;
        self.push_mask(&mask, fade);
        let painted = self.paint_panel_content(rest, frame, settled);
        self.set_offset(0.0);
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.PopLayer() };
        painted
    }

    fn paint_panel_content(&mut self, s: ShapeRect, frame: &Frame, settled: bool) -> Result<()> {
        let strip = frame.strip.is_some();
        let page = frame.page;
        let layout = self.page_layout(s, frame, page, strip);
        let slide = frame.page_slide.clamp(-1.0, 1.0);
        if slide != 0.0 {
            // The page it came from sits one panel width further on.
            let index = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
            let from = if slide > 0.0 {
                index.checked_sub(1)
            } else {
                Some(index + 1).filter(|i| *i < Page::ALL.len())
            };
            if let Some(from) = from {
                let old = self.page_layout(s, frame, Page::ALL[from], strip);
                self.set_offset((slide - slide.signum()) * s.w);
                self.paint_page(frame, &old)?;
            }
        }
        self.set_offset(slide * s.w);
        self.paint_page(frame, &layout)?;
        self.set_offset(0.0);

        if let Some(notice) = frame.strip {
            self.paint_strip(s, notice);
        }
        let (dots, active) = page_dots(s, page);
        for (i, dot) in dots.into_iter().enumerate() {
            let p = &self.palette;
            self.set_color(if i == active { p.accent } else { p.dot_off }, 1.0);
            self.fill_round(dot, dot.w / 2.0);
        }
        // No clicks on a page still sliding in: its controls are not where
        // the regions say yet, and the old page's are still under the pointer.
        if settled && slide == 0.0 {
            let mut hits = regions(s, &layout, strip);
            if matches!(layout, PageLayout::Media(_)) && frame.media.is_none() {
                hits.retain(|r| !matches!(r.hit, Hit::Prev | Hit::PlayPause | Hit::Next));
            }
            self.regions.regions = hits;
            // The v1 media buttons must never fire on another page.
            self.regions.buttons = None;
            if let PageLayout::Cli(l) = &layout {
                self.regions.cli_output = Some(Rect {
                    h: l.input.y - l.term.y,
                    ..l.term
                });
            }
        }
        Ok(())
    }

    /// The layout of `page`; the calendar's arrows follow its title, so the
    /// title is measured first.
    fn page_layout(&self, s: ShapeRect, frame: &Frame, page: Page, strip: bool) -> PageLayout {
        match page {
            Page::Calendar => {
                let title = calendar_title(
                    i18n::lang(),
                    frame.cal_view,
                    frame.cal_anchor,
                    frame.cal_selected,
                );
                // Italic overhang past the advance width.
                let title_w = self.text_width(&title, &self.fonts.heading).ceil() + 3.0;
                PageLayout::Calendar(calendar_layout(
                    s,
                    frame.cal_view,
                    frame.cal_anchor,
                    frame.cal_selected,
                    &frame.calendar.events,
                    &frame.calendar.status,
                    frame.day_page,
                    strip,
                    title_w,
                    Some(Local::now().naive_local()),
                ))
            }
            Page::Pomodoro => PageLayout::Pomodoro(pomodoro_layout(s, strip)),
            Page::Reminders => PageLayout::Reminders(reminders_layout(
                s,
                frame.reminders,
                frame.reminders_page,
                strip,
                frame.today,
            )),
            Page::Notes => PageLayout::Notes(notes_layout(s, frame.notes, frame.notes_page, strip)),
            Page::Media => PageLayout::Media(media_panel_layout(s, strip)),
            Page::Cli => {
                let chips = cli_chips(i18n::lang(), &frame.cli, frame.now);
                let head = self.cli_head(&chips, frame.cli.run.running());
                PageLayout::Cli(cli_layout(s, strip, &head))
            }
            Page::Shortcuts => PageLayout::Shortcuts(shortcuts_layout(s, strip)),
        }
    }

    fn paint_page(&mut self, frame: &Frame, layout: &PageLayout) -> Result<()> {
        match layout {
            PageLayout::Calendar(l) => {
                self.paint_calendar(l, frame);
                Ok(())
            }
            PageLayout::Pomodoro(l) => self.paint_pomodoro(l, frame),
            PageLayout::Reminders(l) => {
                self.paint_reminders(l, frame.reminders, frame.today);
                Ok(())
            }
            PageLayout::Notes(l) => {
                self.paint_notes(l, frame.notes, frame.today);
                Ok(())
            }
            PageLayout::Media(l) => {
                self.paint_media(l, frame.media, frame.lyrics_on, frame.progress)
            }
            PageLayout::Cli(l) => self.paint_cli(l, frame),
            PageLayout::Shortcuts(l) => {
                self.paint_shortcuts(l);
                Ok(())
            }
        }
    }

    /// The measured header of the CLI page: each chip's full width and,
    /// while a command runs, the Stop button's.
    fn cli_head(&self, chips: &[CliChip], running: bool) -> CliHead {
        let lang = i18n::lang();
        let widths = chips
            .iter()
            .map(|c| {
                let w = self.text_width(&c.text, self.chip_format(c.look)).ceil();
                let spin = if c.look == ChipLook::Run {
                    SPIN + SPIN_GAP
                } else {
                    0.0
                };
                w + spin + 2.0 * CHIP_PAD_X
            })
            .collect();
        let stop = running.then(|| {
            let w = self.text_width(text(lang, Key::CliStop), &self.fonts.small_medium);
            w.ceil() + STOP_SQUARE + STOP_GAP + 2.0 * STOP_PAD_X
        });
        CliHead {
            chips: widths,
            stop,
        }
    }

    fn chip_format(&self, look: ChipLook) -> &IDWriteTextFormat {
        match look {
            ChipLook::Mono => &self.fonts.mono_small,
            ChipLook::Path => &self.fonts.chip_mono,
            ChipLook::Run | ChipLook::Ok | ChipLook::Err => &self.fonts.small,
        }
    }

    /// The CLI page as in pengingat-cli.html: title and chips, the terminal
    /// with output filling from the bottom, suggestions over the input, and
    /// the input line with its prompt, caret and hint.
    fn paint_cli(&self, l: &CliLayout, frame: &Frame) -> Result<()> {
        let lang = i18n::lang();
        let p = self.palette;
        let cli = &frame.cli;
        self.heading(text(lang, Key::Cli), l.header);
        let chips = cli_chips(lang, cli, frame.now);
        for (chip, r) in chips.iter().zip(&l.chips) {
            self.paint_cli_chip(chip, *r, cli.run, frame.now)?;
        }
        if let Some(stop) = l.stop {
            self.set_color(p.accent, 1.0);
            self.fill_round(stop, stop.h / 2.0);
            let mid = stop.y + stop.h / 2.0;
            let square = Rect {
                x: stop.x + STOP_PAD_X,
                y: mid - STOP_SQUARE / 2.0,
                w: STOP_SQUARE,
                h: STOP_SQUARE,
            };
            self.set_color(p.on_accent, 1.0);
            self.fill_round(square, 1.5);
            let label = Rect {
                x: square.right() + STOP_GAP,
                w: stop.right() - square.right() - STOP_GAP,
                ..stop
            };
            let word = text(lang, Key::CliStop);
            self.text_colored(word, &self.fonts.small_medium, label, p.on_accent, 1.0);
        }
        self.set_color(p.raised, 1.0);
        self.fill_round(l.expand, l.expand.w / 2.0);
        let icon = if cli.tall { Icon::Shrink } else { Icon::Expand };
        self.icon_colored(icon, l.expand, 13.0, p.secondary, 1.0, None);
        if cli.focused && frame.strip.is_none() {
            self.paint_cli_lock(l);
        }

        self.set_color(theme::TERMINAL_BG, 1.0);
        self.fill_round(l.term, CLI_TERM_RADIUS);
        self.paint_cli_output(l, cli);
        if cli.focused && !cli.suggestions.is_empty() {
            self.paint_cli_suggestions(l, cli);
        }
        self.paint_cli_input(l, cli)
    }

    fn paint_cli_chip(&self, chip: &CliChip, r: Rect, run: RunState, now: Instant) -> Result<()> {
        if r.w <= 0.0 {
            return Ok(());
        }
        let p = self.palette;
        let (fill, alpha, ink) = match chip.look {
            ChipLook::Mono | ChipLook::Path => (p.raised, 1.0, p.secondary),
            ChipLook::Run => (p.accent, 0.15, p.accent),
            ChipLook::Ok => (p.sage, 0.15, p.sage),
            ChipLook::Err => (theme::SOFT_RED, 0.14, theme::SOFT_RED),
        };
        self.set_color(fill, alpha);
        self.fill_round(r, r.h / 2.0);
        let mut area = inset(r, CHIP_PAD_X, 0.0);
        if chip.look == ChipLook::Run {
            let turn = match run {
                RunState::Running { since } => now.saturating_duration_since(since).as_secs(),
                _ => 0,
            };
            let center = (area.x + SPIN / 2.0, r.y + r.h / 2.0);
            self.spinner(center, turn)?;
            area.x += SPIN + SPIN_GAP;
            area.w = (area.w - SPIN - SPIN_GAP).max(0.0);
        }
        self.text_colored(&chip.text, self.chip_format(chip.look), area, ink, 1.0);
        Ok(())
    }

    /// The design's `.spin`: a faint ring with a quarter in the accent,
    /// turned a quarter more each second the command runs.
    fn spinner(&self, (cx, cy): (f32, f32), turn: u64) -> Result<()> {
        let target = self.target.as_ref().expect("target ensured by paint");
        let r = SPIN / 2.0 - 1.0;
        let accent = self.palette.accent;
        unsafe {
            target.brush.SetColor(&color(accent, 0.25));
            let ellipse = D2D1_ELLIPSE {
                point: Vector2::new(cx, cy),
                radiusX: r,
                radiusY: r,
            };
            target.rt.DrawEllipse(&ellipse, &target.brush, 2.0, None);
            let start = -FRAC_PI_2 + (turn % 4) as f32 * FRAC_PI_2;
            let end = start + FRAC_PI_2;
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(
                Vector2::new(cx + r * start.cos(), cy + r * start.sin()),
                D2D1_FIGURE_BEGIN_HOLLOW,
            );
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2::new(cx + r * end.cos(), cy + r * end.sin()),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            target.brush.SetColor(&color(accent, 1.0));
            target
                .rt
                .DrawGeometry(&path, &target.brush, 2.0, &self.round_cap);
        }
        Ok(())
    }

    /// "Panel tetap terbuka saat mengetik · Esc untuk keluar", hanging from
    /// the panel's top edge while the input has the keyboard.
    fn paint_cli_lock(&self, l: &CliLayout) {
        let p = self.palette;
        let label = text(i18n::lang(), Key::CliLocked);
        let tw = self.text_width(label, &self.fonts.micro).ceil();
        let w = LOCK_PAD_X * 2.0 + LOCK_DOT + LOCK_GAP + tw;
        let top = l.header.y - CLI_LOCK_ABOVE;
        let x = l.term.x + (l.term.w - w) / 2.0;
        // Rounded at the bottom only: the top corners rise past the edge,
        // where the panel's mask cuts them off.
        self.set_color(p.raised, 1.0);
        self.fill_round(
            Rect {
                x,
                y: top - LOCK_RADIUS,
                w,
                h: LOCK_H + LOCK_RADIUS,
            },
            LOCK_RADIUS,
        );
        let mid = top + LOCK_H / 2.0;
        let dot = Rect {
            x: x + LOCK_PAD_X,
            y: mid - LOCK_DOT / 2.0,
            w: LOCK_DOT,
            h: LOCK_DOT,
        };
        self.set_color(p.accent, 1.0);
        self.fill_round(dot, LOCK_DOT / 2.0);
        let area = Rect {
            x: dot.right() + LOCK_GAP,
            y: top,
            w: tw + 1.0,
            h: LOCK_H,
        };
        self.text2(label, &self.fonts.micro, area);
    }

    /// The output's last lines from the bottom up (`scroll` lines above the
    /// newest), coloured by kind; an empty terminal says where commands run.
    fn paint_cli_output(&self, l: &CliLayout, cli: &CliView) {
        let p = self.palette;
        let lines = cli.output.lines();
        let out = l.output;
        let row = |k: usize| Rect {
            x: out.x,
            y: out.bottom() - (k + 1) as f32 * CLI_LINE_H,
            w: out.w,
            h: CLI_LINE_H,
        };
        if lines.is_empty() {
            let lang = i18n::lang();
            let empty = [
                (text(lang, Key::CliEmpty), &self.fonts.mono),
                (cli.cwd, &self.fonts.mono),
                (text(lang, Key::CliEmptyHint), &self.fonts.small),
            ];
            for (k, (line, format)) in empty.iter().rev().enumerate() {
                self.text_colored(line, format, row(k), p.secondary, 0.8);
            }
            return;
        }
        let shown = visible_range(lines.len(), l.rows, cli.scroll);
        let first = shown.start;
        for (k, i) in shown.rev().enumerate() {
            let line = &lines[i];
            let r = row(k);
            let kind = line_kind(line, cli.echoes.is_echo(cli.output, i));
            match kind {
                LineKind::Command => {
                    if i > first {
                        // The design's `.sep` between one command's output
                        // and the next command.
                        self.set_color(theme::TERMINAL_SEP, 1.0);
                        self.fill_round(Rect { h: 1.0, ..r }, 0.0);
                    }
                    let mark = Rect { w: CLI_MARK_W, ..r };
                    self.text_colored("\u{203A}", &self.fonts.mono, mark, p.accent, 1.0);
                    let rest = Rect {
                        x: r.x + CLI_MARK_W,
                        w: r.w - CLI_MARK_W,
                        ..r
                    };
                    let cmd = echo_command(line);
                    self.text_colored(cmd, &self.fonts.mono, rest, theme::TERMINAL_TEXT, 1.0);
                }
                LineKind::Output => self.text_colored(line, &self.fonts.mono, r, p.secondary, 1.0),
                LineKind::Error => {
                    self.text_colored(line, &self.fonts.mono, r, theme::SOFT_RED, 1.0)
                }
                LineKind::Ok => self.text_colored(line, &self.fonts.mono, r, p.sage, 1.0),
                LineKind::Quiet => self.text_colored(line, &self.fonts.mono, r, p.secondary, 0.7),
            }
        }
    }

    /// Earlier commands matching the input in a raised box above it: the
    /// typed part in the accent, the highlighted row tinted.
    fn paint_cli_suggestions(&self, l: &CliLayout, cli: &CliView) {
        let p = self.palette;
        let (boxed, rows) = cli_suggestions(l, cli.suggestions.len());
        self.set_color(p.raised, 1.0);
        self.fill_round(boxed, SUGGEST_RADIUS);
        self.stroke_round(inset(boxed, 0.5, 0.5), SUGGEST_RADIUS, p.track);
        let typed = cli.input.trim_start();
        let selected = cli.selected.min(cli.suggestions.len().saturating_sub(1));
        for (i, (cmd, r)) in cli.suggestions.iter().zip(rows).enumerate() {
            if i == selected {
                self.set_color(p.accent, 0.16);
                self.fill_round(r, SUGGEST_ROW_RADIUS);
            }
            let area = inset(r, SUGGEST_PAD_X, 0.0);
            let head = &cmd[..typed.len().min(cmd.len())];
            let hw = self.text_width(head, &self.fonts.mono_medium);
            self.text_colored(head, &self.fonts.mono_medium, area, p.accent, 1.0);
            let rest = Rect {
                x: area.x + hw,
                w: (area.w - hw).max(0.0),
                ..area
            };
            let tail = &cmd[head.len()..];
            self.text_colored(tail, &self.fonts.mono, rest, theme::TERMINAL_TEXT, 1.0);
        }
    }

    /// The input line: the rule above it, the focus ring, `›`, the typed
    /// text with its selection, the rest of the highlighted suggestion in
    /// ghost ink, the caret, and the hint at the right.
    fn paint_cli_input(&self, l: &CliLayout, cli: &CliView) -> Result<()> {
        let lang = i18n::lang();
        let p = self.palette;
        let rule = Rect {
            y: l.input.y,
            h: 1.0,
            ..l.input
        };
        self.set_color(theme::TERMINAL_RULE, 1.0);
        self.fill_round(rule, 0.0);
        if cli.focused {
            self.focus_ring(l.input)?;
        }
        self.text_colored("\u{203A}", &self.fonts.mono, l.prompt, p.accent, 1.0);

        let running = cli.run.running();
        let hint = if running {
            Some(Key::CliHintRunning)
        } else if cli.focused && !cli.suggestions.is_empty() {
            Some(Key::CliHintSuggest)
        } else if cli.focused {
            Some(Key::CliHintReady)
        } else {
            None
        };
        let mut field = l.field;
        if let Some(key) = hint {
            let label = text(lang, key);
            let w = self.text_width(label, &self.fonts.micro).ceil() + 1.0;
            let area = Rect {
                x: field.right() - w,
                w,
                ..field
            };
            self.text_colored(label, &self.fonts.micro, area, theme::TERMINAL_HINT, 1.0);
            field.w = (area.x - HINT_GAP - field.x).max(0.0);
        }

        let mid = field.y + field.h / 2.0;
        if cli.input.is_empty() {
            let ghost = if running {
                Some(Key::CliWaiting)
            } else if !cli.focused {
                Some(Key::CliHintIdle)
            } else {
                None
            };
            if let Some(key) = ghost {
                let area = Rect {
                    x: field.x + if cli.focused { CARET_W + 4.0 } else { 0.0 },
                    ..field
                };
                let format = &self.fonts.mono;
                self.text_colored(text(lang, key), format, area, theme::TERMINAL_GHOST, 1.0);
            }
        }
        // Measured in the mono font: offsets are UTF-16, as the EDIT gives them.
        let len = cli.input.encode_utf16().count();
        let (a, b) = (cli.selection.0.min(len), cli.selection.1.min(len));
        let x_of = |n: usize| self.text_width(utf16_prefix(cli.input, n), &self.fonts.mono);
        let caret_x = x_of(b);
        // Long input scrolls left so the caret stays in view.
        let shift = (caret_x + CARET_W - field.w).max(0.0);
        let origin = field.x - shift;
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target
                .rt
                .PushAxisAlignedClip(&d2d_rect(field), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        if a != b && cli.focused {
            let xa = x_of(a);
            self.set_color(p.accent, 0.32);
            self.fill_round(
                Rect {
                    x: origin + xa,
                    y: mid - CARET_H / 2.0,
                    w: caret_x - xa,
                    h: CARET_H,
                },
                2.0,
            );
        }
        let typed = Rect {
            x: origin,
            w: 4096.0,
            ..field
        };
        self.text_colored(
            cli.input,
            &self.fonts.mono,
            typed,
            theme::TERMINAL_TEXT,
            1.0,
        );
        let at_end = b == len;
        if cli.focused && at_end && !cli.input.is_empty() {
            let pick = cli.selected.min(cli.suggestions.len().saturating_sub(1));
            if let Some(rest) = cli
                .suggestions
                .get(pick)
                .and_then(|s| s.strip_prefix(cli.input))
            {
                let area = Rect {
                    x: origin + caret_x,
                    w: 4096.0,
                    ..field
                };
                self.text_colored(rest, &self.fonts.mono, area, theme::TERMINAL_GHOST, 1.0);
            }
        }
        if cli.focused {
            let caret = if at_end {
                Rect {
                    x: origin + caret_x,
                    y: mid - CARET_H / 2.0,
                    w: CARET_W,
                    h: CARET_H,
                }
            } else {
                Rect {
                    x: origin + caret_x - 0.75,
                    y: mid - CARET_H / 2.0,
                    w: 1.5,
                    h: CARET_H,
                }
            };
            self.set_color(p.accent, if at_end { 1.0 } else { 0.95 });
            self.fill_round(caret, 0.0);
            self.cli_caret.set(Some(caret));
        }
        unsafe { target.rt.PopAxisAlignedClip() };
        Ok(())
    }

    /// The design's `.inp.focus`: an inset accent line round the input,
    /// square at the top, following the terminal's bottom corners.
    fn focus_ring(&self, input: Rect) -> Result<()> {
        let target = self.target.as_ref().expect("target ensured by paint");
        let half = FOCUS_W / 2.0;
        let r = inset(input, half, half);
        let radius = CLI_TERM_RADIUS - half;
        let (left, right, top, bottom) = (r.x, r.right(), r.y, r.bottom());
        let corner = |x: f32, y: f32| D2D1_ARC_SEGMENT {
            point: Vector2::new(x, y),
            size: D2D_SIZE_F {
                width: radius,
                height: radius,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        unsafe {
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(left, top), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLine(Vector2::new(right, top));
            sink.AddLine(Vector2::new(right, bottom - radius));
            sink.AddArc(&corner(right - radius, bottom));
            sink.AddLine(Vector2::new(left + radius, bottom));
            sink.AddArc(&corner(left, bottom - radius));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            target.brush.SetColor(&color(self.palette.accent, 0.55));
            target.rt.DrawGeometry(&path, &target.brush, FOCUS_W, None);
        }
        Ok(())
    }

    /// Two rows of tiles: a duotone icon over its label.
    fn paint_shortcuts(&self, l: &ShortcutsLayout) {
        let lang = i18n::lang();
        let heading = text(lang, Key::Shortcuts);
        self.heading(heading, l.header);
        for (s, tile) in l.tiles {
            self.set_color(self.palette.raised, 1.0);
            self.fill_round(tile, radius::TILE);
            let (glyph, label) = tile_parts(tile);
            self.icon(shortcuts::icon(s), glyph, 24.0, 1.0, true);
            let name = text(lang, shortcuts::label(s));
            self.text(name, &self.fonts.eleven_center, label, 1.0);
        }
    }

    fn paint_calendar(&mut self, l: &CalendarLayout, frame: &Frame) {
        let lang = i18n::lang();
        let p = self.palette;
        let snap = frame.calendar;
        let events = &snap.events;
        let title = calendar_title(lang, l.view, frame.cal_anchor, l.selected);
        self.heading(&title, l.header);
        self.icon_colored(Icon::ChevL, l.prev, 18.0, p.secondary, 1.0, None);
        self.icon_colored(Icon::ChevR, l.next, 18.0, p.secondary, 1.0, None);
        if let Some(tag) = l.offline {
            self.set_color(p.track, 1.0);
            self.fill_round(tag, tag.h / 2.0);
            self.text2(text(lang, Key::Offline), &self.fonts.micro_center, tag);
        }

        // View selector: one raised track, the active segment lit.
        let first = l.view_buttons[0].1;
        let last = l.view_buttons[2].1;
        let track = Rect {
            x: first.x - SEG_PAD,
            w: last.right() - first.x + 2.0 * SEG_PAD,
            ..first
        };
        self.set_color(p.raised, 1.0);
        self.fill_round(track, radius::CONTROL);
        for (view, r) in l.view_buttons {
            let on = view == l.view;
            if on {
                self.set_color(p.raised_hi, 1.0);
                self.fill_round(inset(r, 0.0, SEG_PAD), radius::SEGMENT);
            }
            let label = text(
                lang,
                match view {
                    CalView::Day => Key::ViewDay,
                    CalView::Week => Key::ViewWeek,
                    CalView::Month => Key::ViewMonth,
                },
            );
            let ink = if on { p.text } else { p.secondary };
            self.text_colored(label, &self.fonts.small_center, r, ink, 1.0);
        }

        match l.view {
            CalView::Month => self.paint_month(l, frame, lang),
            CalView::Week => self.paint_week(l, frame, lang),
            CalView::Day => self.paint_day(l, events, lang),
        }
        self.paint_add(l, snap.status, lang);
    }

    fn paint_month(&self, l: &CalendarLayout, frame: &Frame, lang: Lang) {
        let p = self.palette;
        let events = &frame.calendar.events;
        // The grid starts on a Monday: the first row names the weekdays.
        for ((d, _), r) in l.month_cells.iter().zip(&l.weekday_labels) {
            self.text2(i18n::day_short(lang, *d), &self.fonts.micro_center, *r);
        }
        for (d, r) in &l.month_cells {
            self.paint_day_cell(*d, *r, frame.today, l.selected, frame.cal_anchor, events);
        }
        // The line between the grid and the list, midway.
        if let Some((_, last)) = l.month_cells.get(6) {
            let x = (last.right() + l.list.x) / 2.0;
            self.set_color(p.track, 1.0);
            self.fill_round(
                Rect {
                    x: x - 0.5,
                    y: l.list.y,
                    w: 1.0,
                    h: l.list.h,
                },
                0.0,
            );
        }
        if let Some(caption) = l.caption {
            let day = i18n::day_title_long(lang, l.selected);
            self.text2(&day, &self.fonts.small, caption);
        }
        for (id, r) in &l.rows {
            let Some(ev) = events.iter().find(|e| &e.id == id) else {
                continue;
            };
            self.set_color(event_color(ev, &p), 1.0);
            self.fill_round(
                Rect {
                    x: r.x,
                    y: r.y + 2.0,
                    w: 3.0,
                    h: r.h - 10.0,
                },
                1.5,
            );
            let text_x = r.x + 12.0;
            let w = r.right() - text_x;
            let title = Rect {
                x: text_x,
                y: r.y,
                w,
                h: 17.0,
            };
            let time = Rect {
                x: text_x,
                y: r.y + 16.0,
                w,
                h: 14.0,
            };
            self.text(ev.shown_title(), &self.fonts.body_medium, title, 1.0);
            self.text2(&event_time_label(lang, ev), &self.fonts.small, time);
        }
        if let Some((n, r)) = l.more {
            self.text2(&more_label(lang, n), &self.fonts.small, r);
        }
    }

    /// A month grid day (`month_grid::paint_day`): today in an accent
    /// circle, the selected day ringed, days of other months dim, a sage dot
    /// under other days with events.
    fn paint_day_cell(
        &self,
        d: NaiveDate,
        r: Rect,
        today: NaiveDate,
        selected: NaiveDate,
        anchor: NaiveDate,
        events: &[CalEvent],
    ) {
        let p = self.palette;
        let look = CellLook {
            filled: Filled::Today,
            ring_w: 1.0,
            ring_ink_accent: false,
            accent: p.accent,
            on_accent: p.on_accent,
            text: p.text,
            dim: p.dot_off,
            dot: p.sage,
            hover: None,
        };
        let state = DayState {
            events: !events_on(events, d).is_empty(),
            ..DayState::of(d, today, selected, anchor)
        };
        paint_day(self, d, day_geometry(r, 3.0, 26.0), state, &look);
    }

    /// Seven columns: the day's name and number (today in the accent), then
    /// its events as chips with a coloured edge; all-day ones as tinted bands.
    fn paint_week(&self, l: &CalendarLayout, frame: &Frame, lang: Lang) {
        let p = self.palette;
        let events = &frame.calendar.events;
        for (d, col) in &l.week_columns {
            self.set_color(p.column, 1.0);
            self.fill_round(*col, radius::COLUMN);
            let name = Rect {
                y: col.y + 6.0,
                h: 12.0,
                ..*col
            };
            let number = Rect {
                y: col.y + 17.0,
                h: 18.0,
                ..*col
            };
            self.text2(i18n::day_short(lang, *d), &self.fonts.micro_center, name);
            let label = d.day().to_string();
            let ink = if *d == frame.today { p.accent } else { p.text };
            self.text_colored(&label, &self.fonts.day_number, number, ink, 1.0);
            if *d == l.selected && *d != frame.today {
                let head = Rect {
                    h: WEEK_HEAD_H,
                    ..*col
                };
                self.stroke_round(inset(head, 3.0, 3.0), radius::SEGMENT, p.accent);
            }
        }
        for (id, r) in &l.rows {
            let Some(ev) = events.iter().find(|e| &e.id == id) else {
                continue;
            };
            let edge = event_color(ev, &p);
            if ev.all_day() {
                self.set_color(edge, CHIP_TINT);
            } else {
                self.set_color(p.raised, 1.0);
            }
            self.fill_round(*r, radius::EVENT_CHIP);
            self.set_color(edge, 1.0);
            self.fill_round(
                Rect {
                    x: r.x + 1.0,
                    y: r.y + 3.0,
                    w: 2.5,
                    h: r.h - 6.0,
                },
                1.25,
            );
            let inner = Rect {
                x: r.x + 7.0,
                w: r.w - 9.0,
                ..*r
            };
            if ev.all_day() {
                self.text(ev.shown_title(), &self.fonts.chip_title, inner, 1.0);
            } else {
                let title = Rect {
                    y: r.y + 3.0,
                    h: 13.0,
                    ..inner
                };
                let time = Rect {
                    y: r.y + 16.0,
                    h: 11.0,
                    ..inner
                };
                self.text(ev.shown_title(), &self.fonts.chip_title, title, 1.0);
                if let EventTime::At(start) = &ev.start {
                    let at = i18n::time_hm(lang, start.naive_local());
                    self.text2(&at, &self.fonts.chip_time, time);
                }
            }
        }
        for (_, n, r) in &l.week_more {
            self.text2(&format!("+{n}"), &self.fonts.micro_center, *r);
        }
    }

    /// All-day bands, then the hour lines with their labels, the events as
    /// tinted blocks, and the "now" line with its dot.
    fn paint_day(&self, l: &CalendarLayout, events: &[CalEvent], lang: Lang) {
        let p = self.palette;
        for (hour, y) in &l.hours {
            let label = Rect {
                x: l.list.x,
                y: y - 6.0,
                w: HOUR_GUTTER - 6.0,
                h: 12.0,
            };
            self.text2(&hour_label(lang, *hour), &self.fonts.micro, label);
            self.set_color(p.track, 1.0);
            self.fill_round(
                Rect {
                    x: l.list.x + HOUR_GUTTER - 4.0,
                    y: y - 0.5,
                    w: l.list.right() - (l.list.x + HOUR_GUTTER - 4.0),
                    h: 1.0,
                },
                0.0,
            );
        }
        for (id, r) in &l.rows {
            let Some(ev) = events.iter().find(|e| &e.id == id) else {
                continue;
            };
            let base = event_color(ev, &p);
            let ink = mix(base, p.text, BLOCK_TEXT_MIX);
            if ev.all_day() {
                self.set_color(base, BAND_TINT);
                self.fill_round(*r, radius::SEGMENT);
                let label = format!("{} · {}", text(lang, Key::AllDay), ev.shown_title());
                let area = inset(*r, 9.0, 0.0);
                self.text_colored(&label, &self.fonts.small, area, ink, 1.0);
                continue;
            }
            self.set_color(base, BLOCK_TINT);
            self.fill_round(*r, radius::BLOCK);
            let inner = inset(*r, 9.0, 0.0);
            let time = event_time_label(lang, ev);
            if r.h >= 30.0 {
                let title = Rect {
                    y: r.y + 3.0,
                    h: 15.0,
                    ..inner
                };
                let at = Rect {
                    y: r.y + 17.0,
                    h: 12.0,
                    ..inner
                };
                self.text_colored(ev.shown_title(), &self.fonts.eleven, title, ink, 1.0);
                self.text_colored(&time, &self.fonts.micro, at, ink, 0.8);
            } else {
                let line = match &ev.start {
                    EventTime::At(t) => {
                        format!(
                            "{} · {}",
                            ev.shown_title(),
                            i18n::time_hm(lang, t.naive_local())
                        )
                    }
                    EventTime::AllDay(_) => ev.shown_title().to_owned(),
                };
                self.text_colored(&line, &self.fonts.eleven, inner, ink, 1.0);
            }
        }
        if let Some(y) = l.now_y {
            let x = l.list.x + HOUR_GUTTER - 6.0;
            self.set_color(p.accent, 1.0);
            self.fill_round(
                Rect {
                    x,
                    y: y - 0.75,
                    w: l.list.right() - x,
                    h: 1.5,
                },
                0.75,
            );
            self.fill_round(
                Rect {
                    x: x - 3.5,
                    y: y - 3.5,
                    w: 7.0,
                    h: 7.0,
                },
                3.5,
            );
        }
        if let Some((n, r)) = l.more {
            self.text2(&more_label(lang, n), &self.fonts.small, r);
        }
    }

    /// "+ Tambah acara" in the accent, and the Google button.
    fn paint_add(&self, l: &CalendarLayout, status: CalStatus, lang: Lang) {
        let p = self.palette;
        let plus = Rect { w: 16.0, ..l.add };
        self.icon_colored(Icon::Plus, plus, 13.0, p.accent, 1.0, None);
        let label = Rect {
            x: l.add.x + 20.0,
            w: l.add.w - 20.0,
            ..l.add
        };
        let add = text(lang, Key::AddEvent);
        self.text_colored(add, &self.fonts.eleven, label, p.accent, 1.0);
        if let Some(r) = l.connect {
            self.set_color(p.accent, 1.0);
            self.fill_round(r, radius::CONTROL);
            let label = text(
                lang,
                if status == CalStatus::Reconnect {
                    Key::ReconnectGoogle
                } else {
                    Key::ConnectGoogle
                },
            );
            self.text_colored(label, &self.fonts.body_center, r, p.on_accent, 1.0);
        }
    }

    /// The big ring with the countdown and the phase; beside it the session
    /// with its dots, reset / the primary start-pause / skip, and what comes
    /// next.
    fn paint_pomodoro(&mut self, l: &PomodoroLayout, frame: &Frame) -> Result<()> {
        let lang = i18n::lang();
        let p = self.palette;
        let pomo = frame.pomodoro;
        let settings = pomo.settings();
        let status = pomo.status();
        let tint = p.phase(pomo.phase());
        self.heading(text(lang, Key::Pomodoro), l.header);
        let durations = i18n::pomo_durations(
            lang,
            settings.focus_min,
            settings.short_break_min,
            settings.long_break_min,
        );
        let right = &self.fonts.body_right;
        self.text_colored(&durations, right, l.durations, p.secondary, 1.0);

        self.ring(
            l.ring_center,
            l.ring_radius,
            RING_STROKE,
            pomo.progress(frame.now),
            tint,
        )?;
        let paused = matches!(status, PomoStatus::Paused { .. });
        let time = mmss(pomo.remaining(frame.now));
        let alpha = if paused { 0.6 } else { 1.0 };
        self.text(&time, &self.fonts.big_time, l.time, alpha);
        let phase = phase_label(lang, pomo).to_uppercase();
        self.text_colored(&phase, &self.fonts.small_center_bold, l.phase, tint, 1.0);

        // "Sesi 2 dari 4" and a dot per session, filled up to this one.
        let n = settings.sessions;
        let k = pomo.session().min(n);
        let session = i18n::session_of(lang, k, n);
        self.text2(&session, &self.fonts.secondary, l.session);
        let dot = 8.0;
        let mut x = l.session.x + self.text_width(&session, &self.fonts.secondary) + 8.0;
        for i in 0..n {
            if x + dot > l.session.right() {
                break;
            }
            let r = Rect {
                x,
                y: l.session.y + (l.session.h - dot) / 2.0,
                w: dot,
                h: dot,
            };
            if i < k {
                self.set_color(tint, 1.0);
                self.fill_round(r, dot / 2.0);
            } else {
                self.stroke_round_width(inset(r, 0.75, 0.75), dot / 2.0, p.dot_off, 1.5);
            }
            x += dot + 5.0;
        }

        for (r, icon) in [(l.reset, Icon::Reset), (l.skip, Icon::Skip)] {
            self.set_color(p.raised, 1.0);
            self.fill_round(r, r.w / 2.0);
            self.icon(icon, r, 18.0, 1.0, false);
        }
        self.set_color(tint, 1.0);
        self.fill_round(l.start_pause, l.start_pause.w / 2.0);
        let main = primary_icon(status);
        self.icon_colored(main, l.start_pause, 24.0, p.on_accent, 1.0, None);

        // "Berikutnya: Istirahat 5 menit", the phase in its colour.
        let (next, minutes) = next_phase(pomo);
        let lead = text(lang, Key::NextUp);
        self.text2(lead, &self.fonts.eleven, l.next_up);
        let after = self.text_width(lead, &self.fonts.eleven) + 4.0;
        let what = i18n::phase_minutes(lang, phase_name(lang, next), minutes);
        let rest = Rect {
            x: l.next_up.x + after,
            w: (l.next_up.w - after).max(0.0),
            ..l.next_up
        };
        let ink = p.phase(next);
        self.text_colored(&what, &self.fonts.button, rest, ink, 1.0);
        Ok(())
    }

    /// "+ Baru": a raised chip with a plus.
    fn paint_new_button(&self, r: Rect, lang: Lang) {
        let label = text(lang, Key::New);
        let w = (self.text_width(label, &self.fonts.button) + 38.0).min(r.w);
        let chip = Rect {
            x: r.right() - w,
            w,
            ..r
        };
        self.set_color(self.palette.raised, 1.0);
        self.fill_round(chip, chip.h / 2.0);
        let plus = Rect {
            x: chip.x + 9.0,
            w: 14.0,
            ..chip
        };
        self.icon(Icon::Plus, plus, 13.0, 1.0, false);
        let area = Rect {
            x: plus.right() + 4.0,
            w: chip.right() - plus.right() - 4.0,
            ..chip
        };
        self.text(label, &self.fonts.button, area, 1.0);
    }

    /// The hint under the header of an empty list.
    fn paint_empty_hint(&self, l: &ListLayout, key: Key, lang: Lang) {
        let hint = Rect {
            y: l.header.bottom() + 8.0,
            h: 20.0,
            ..l.header
        };
        self.text2(text(lang, key), &self.fonts.secondary, hint);
    }

    /// Cards in two columns: the first line as the title, the rest as a
    /// snippet, the time at the foot.
    fn paint_notes(&self, l: &ListLayout, notes: &[Note], today: NaiveDate) {
        let lang = i18n::lang();
        let p = self.palette;
        self.heading(text(lang, Key::Notes), l.header);
        self.paint_new_button(l.new_button, lang);
        if notes.is_empty() {
            self.paint_empty_hint(l, Key::NotesEmpty, lang);
        }
        for (id, r) in &l.rows {
            let Some(note) = notes.iter().find(|n| &n.id == id) else {
                continue;
            };
            self.set_color(p.raised, 1.0);
            self.fill_round(*r, radius::CARD);
            let inner = inset(*r, 13.0, 0.0);
            let title = Rect {
                y: r.y + 9.0,
                h: 18.0,
                ..inner
            };
            self.text(first_line(&note.text), &self.fonts.body_medium, title, 1.0);
            let foot = Rect {
                y: r.bottom() - 22.0,
                h: 14.0,
                ..inner
            };
            let snippet = Rect {
                y: title.bottom() + 2.0,
                h: (foot.y - title.bottom() - 4.0).max(0.0),
                ..inner
            };
            self.text2(&note_snippet(&note.text), &self.fonts.snippet, snippet);
            let when = DateTime::from_timestamp(note.updated, 0)
                .map(|t| note_time_label(lang, t.with_timezone(&Local).naive_local(), today))
                .unwrap_or_default();
            let ink = p.secondary;
            self.text_colored(&when, &self.fonts.micro, foot, ink, NOTE_TIME_ALPHA);
        }
        if let Some((n, r)) = l.more {
            self.text2(&more_label(lang, n), &self.fonts.small, r);
        }
    }

    /// Day groups ("HARI INI", "BESOK", a date) of rows: a ring, the time,
    /// the text, an ochre "Terlewat" tag when overdue, a repeat icon when it
    /// repeats; a hairline under each row.
    fn paint_reminders(&self, l: &ListLayout, reminders: &[Reminder], today: NaiveDate) {
        let lang = i18n::lang();
        let p = self.palette;
        let now = Local::now().timestamp();
        self.heading(text(lang, Key::Reminders), l.header);
        self.paint_new_button(l.new_button, lang);
        if reminders.is_empty() {
            self.paint_empty_hint(l, Key::RemindersEmpty, lang);
        }
        for (day, r) in &l.groups {
            let label = day_label(lang, *day, today).to_uppercase();
            self.text2(&label, &self.fonts.micro, *r);
        }
        for (id, r) in &l.rows {
            let Some(reminder) = reminders.iter().find(|x| &x.id == id) else {
                continue;
            };
            self.set_color(p.column, 1.0);
            self.fill_round(
                Rect {
                    y: r.bottom() - 1.0,
                    h: 1.0,
                    ..*r
                },
                0.0,
            );
            let ring = Rect { w: 20.0, ..*r };
            self.icon_colored(Icon::Ring, ring, 17.0, p.secondary, 1.0, None);
            let (time, line) = reminder_row(lang, reminder);
            let at = Rect {
                x: r.x + 30.0,
                w: 40.0,
                ..*r
            };
            self.text2(&time, &self.fonts.eleven, at);
            // From the right: the repeat icon, then the tag.
            let mut end = r.right();
            if reminder.repeat != Repeat::Once {
                let icon = Rect {
                    x: end - 16.0,
                    w: 16.0,
                    ..*r
                };
                self.icon_colored(Icon::Repeat, icon, 15.0, p.secondary, 1.0, None);
                end = icon.x - 8.0;
            }
            if reminder.due_unix <= now {
                let label = text(lang, Key::Missed);
                let w = self.text_width(label, &self.fonts.micro) + 14.0;
                let tag = Rect {
                    x: end - w,
                    y: r.y + (r.h - 17.0) / 2.0,
                    w,
                    h: 17.0,
                };
                self.set_color(p.ochre, TAG_TINT);
                self.fill_round(tag, tag.h / 2.0);
                self.text_colored(label, &self.fonts.micro_center, tag, p.ochre, 1.0);
                end = tag.x - 8.0;
            }
            let body = Rect {
                x: at.right() + 8.0,
                w: (end - at.right() - 8.0).max(0.0),
                ..*r
            };
            self.text(line, &self.fonts.secondary, body, 1.0);
        }
        if let Some((n, r)) = l.more {
            self.text2(&more_label(lang, n), &self.fonts.small, r);
        }
    }

    /// The big cover; "SEDANG DIPUTAR", the title in Fraunces, the artist;
    /// the progress bar with its times; prev, the round play/pause, next,
    /// and "Lirik".
    fn paint_media(
        &mut self,
        l: &ExpandedLayout,
        media: Option<&MediaInfo>,
        lyrics_on: bool,
        progress: Option<Progress>,
    ) -> Result<()> {
        let p = self.palette;
        self.paint_art(l.art, radius::COVER, media.and_then(|m| m.art.as_deref()))?;
        self.paint_lyrics_button(l.lyrics, lyrics_on);
        let Some(media) = media else {
            self.text(
                i18n::tr(Key::NothingPlaying),
                &self.fonts.artist,
                l.title,
                0.8,
            );
            self.text2(
                i18n::tr(Key::NothingPlayingHint),
                &self.fonts.small,
                l.artist,
            );
            return Ok(());
        };
        let label = i18n::tr(Key::NowPlaying).to_uppercase();
        self.text2(&label, &self.fonts.micro, l.label);
        self.text(&media.title, &self.fonts.song, l.title, 1.0);
        self.text2(&media.artist, &self.fonts.artist, l.artist);
        if let Some(progress) = progress {
            self.paint_progress(l, progress);
        }
        let play = if media.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        let [prev, middle, next] = l.buttons;
        self.icon(Icon::Prev, prev, 20.0, 1.0, false);
        self.set_color(p.accent, 1.0);
        self.fill_round(middle, middle.w / 2.0);
        self.icon_colored(play, middle, 22.0, p.on_accent, 1.0, None);
        self.icon(Icon::Next, next, 20.0, 1.0, false);
        Ok(())
    }

    /// The track and its played part in the accent, a knob at the playhead,
    /// and the elapsed and total times under the ends.
    fn paint_progress(&self, l: &ExpandedLayout, progress: Progress) {
        let Some(fraction) = progress.fraction() else {
            return;
        };
        let p = self.palette;
        let m = media_progress_layout(l);
        let bar = m.bar;
        self.set_color(p.track, 1.0);
        self.fill_round(bar, bar.h / 2.0);
        let fill = Rect {
            w: bar.w * fraction,
            ..bar
        };
        self.set_color(p.accent, 1.0);
        if fill.w > 0.0 {
            self.fill_round(fill, bar.h / 2.0);
        }
        let knob = Rect {
            x: fill.right() - PROGRESS_KNOB / 2.0,
            y: bar.y + (bar.h - PROGRESS_KNOB) / 2.0,
            w: PROGRESS_KNOB,
            h: PROGRESS_KNOB,
        };
        self.fill_round(knob, PROGRESS_KNOB / 2.0);
        let elapsed = track_time(progress.position_ms.min(progress.end_ms));
        self.text2(&elapsed, &self.fonts.track_time, m.elapsed);
        let total = track_time(progress.end_ms);
        self.text2(&total, &self.fonts.track_time_right, m.total);
    }

    /// "Lirik" with a quote mark: outlined in the accent while lyrics show in
    /// the notch, quiet otherwise.
    fn paint_lyrics_button(&self, r: Rect, on: bool) {
        let p = self.palette;
        let ink = if on { p.accent } else { p.secondary };
        if on {
            self.set_color(p.accent, TAG_TINT);
            self.fill_round(r, r.h / 2.0);
        }
        self.stroke_round(
            inset(r, 0.5, 0.5),
            r.h / 2.0,
            if on { p.accent } else { p.dot_off },
        );
        let label = i18n::tr(Key::LyricsButton);
        let w = 14.0 + 5.0 + self.text_width(label, &self.fonts.button);
        let x = r.x + (r.w - w) / 2.0;
        let icon = Rect { x, w: 14.0, ..r };
        self.icon_colored(Icon::Quote, icon, 12.0, ink, 1.0, None);
        let area = Rect {
            x: icon.right() + 5.0,
            w: (r.right() - icon.right() - 5.0).max(0.0),
            ..r
        };
        self.text_colored(label, &self.fonts.button, area, ink, 1.0);
    }

    /// Orange dot and "Claude butuh respon · {project}", or an accent dot and
    /// "{reminder} · 09.30".
    fn paint_strip(&self, s: ShapeRect, notice: &Notice) {
        let r = waiting_strip(s);
        self.set_color(self.palette.raised, 1.0);
        self.fill_round(r, radius::CONTROL);
        let dot = match notice.kind {
            NoticeKind::Reminder { .. } => self.palette.accent,
            _ => CLAUDE,
        };
        self.set_color(dot, 1.0);
        self.fill_round(
            Rect {
                x: r.x + 10.0,
                y: r.y + r.h / 2.0 - 4.0,
                w: 8.0,
                h: 8.0,
            },
            4.0,
        );
        let text = if notice.body.is_empty() {
            notice.title.clone()
        } else {
            format!("{} · {}", notice.title, notice.body)
        };
        let area = Rect {
            x: r.x + 26.0,
            w: r.w - 36.0,
            ..r
        };
        self.text(&text, &self.fonts.secondary, area, 1.0);
    }

    /// Small ring, `mm:ss`, then "Fokus 2/4", "Istirahat" or "Siap · klik".
    pub(super) fn paint_pomodoro_compact(&mut self, shape: ShapeRect, frame: &Frame) -> Result<()> {
        let pomo = frame.pomodoro;
        let (center, radius, text) = match frame.robot {
            Some(robot) => {
                let (slot, center, radius, text) = pomodoro_compact_layout_robot(shape);
                self.paint_robot(slot, robot);
                (center, radius, text)
            }
            None => pomodoro_compact_layout(shape),
        };
        let tint = self.palette.phase(pomo.phase());
        self.ring(center, radius, 3.0, pomo.progress(frame.now), tint)?;
        let paused = matches!(pomo.status(), PomoStatus::Paused { .. });
        let time = Rect { w: 42.0, ..text };
        let label = Rect {
            x: time.right() + 4.0,
            w: (text.right() - time.right() - 4.0).max(0.0),
            ..text
        };
        self.text(
            &mmss(pomo.remaining(frame.now)),
            &self.fonts.title,
            time,
            if paused { 0.6 } else { 1.0 },
        );
        let lang = i18n::lang();
        self.text2(&compact_label(lang, pomo), &self.fonts.eleven, label);
        Ok(())
    }

    /// Track plus a clockwise arc from 12 o'clock covering `progress`.
    fn ring(
        &self,
        (cx, cy): (f32, f32),
        r: f32,
        stroke: f32,
        progress: f32,
        rgb: u32,
    ) -> Result<()> {
        let target = self.target.as_ref().expect("target ensured by paint");
        let ellipse = D2D1_ELLIPSE {
            point: Vector2::new(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            target.brush.SetColor(&color(self.palette.track, 1.0));
            target.rt.DrawEllipse(&ellipse, &target.brush, stroke, None);
        }
        let p = progress.clamp(0.0, 1.0);
        if p <= 0.0 {
            return Ok(());
        }
        unsafe {
            target.brush.SetColor(&color(rgb, 1.0));
            if p >= 0.999 {
                target.rt.DrawEllipse(&ellipse, &target.brush, stroke, None);
                return Ok(());
            }
            let angle = -FRAC_PI_2 + TAU * p;
            let path = self.d2d.CreatePathGeometry()?;
            let sink = path.Open()?;
            sink.BeginFigure(Vector2::new(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2::new(cx + r * angle.cos(), cy + r * angle.sin()),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if p > 0.5 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            target
                .rt
                .DrawGeometry(&path, &target.brush, stroke, &self.round_cap);
        }
        Ok(())
    }

    /// Everything drawn until `PopLayer` is clipped to `mask` at `opacity`.
    fn push_mask(&self, mask: &ID2D1Geometry, opacity: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F {
                left: -f32::MAX,
                top: -f32::MAX,
                right: f32::MAX,
                bottom: f32::MAX,
            },
            geometricMask: ManuallyDrop::new(Some(mask.clone())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe { target.rt.PushLayer(&params, &target.layer) };
        // The layer keeps its own reference while pushed.
        drop(ManuallyDrop::into_inner(params.geometricMask));
    }

    /// Shifts everything drawn next `dx` logical px to the right.
    fn set_offset(&self, dx: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe { target.rt.SetTransform(&Matrix3x2::translation(dx, 0.0)) };
    }

    /// A 1 px outline.
    fn stroke_round(&self, r: Rect, radius: f32, rgb: u32) {
        self.stroke_round_width(r, radius, rgb, 1.0);
    }

    fn stroke_round_width(&self, r: Rect, radius: f32, rgb: u32, width: f32) {
        let target = self.target.as_ref().expect("target ensured by paint");
        unsafe {
            target.brush.SetColor(&color(rgb, 1.0));
            target.rt.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: d2d_rect(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                &target.brush,
                width,
                None,
            );
        }
    }
}

impl CellCanvas for Renderer {
    fn disc(&self, circle: Rect, rgb: u32) {
        self.set_color(rgb, 1.0);
        self.fill_round(circle, circle.w / 2.0);
    }

    fn ring(&self, circle: Rect, width: f32, rgb: u32) {
        let half = width / 2.0;
        self.stroke_round_width(inset(circle, half, half), circle.w / 2.0, rgb, width);
    }

    fn number(&self, text: &str, area: Rect, rgb: u32, bold: bool) {
        let format = if bold {
            &self.fonts.cell_bold
        } else {
            &self.fonts.cell
        };
        self.text_colored(text, format, area, rgb, 1.0);
    }
}

/// The calendar's title: the month, the week's range or the day.
fn calendar_title(lang: Lang, view: CalView, anchor: NaiveDate, selected: NaiveDate) -> String {
    match view {
        CalView::Month => i18n::month_title(lang, anchor),
        CalView::Week => i18n::week_title(lang, range_for(CalView::Week, anchor).0),
        CalView::Day => i18n::day_title_long(lang, selected),
    }
}

/// An event's own colour, else sage for all-day events and the accent for
/// timed ones.
fn event_color(ev: &CalEvent, p: &Palette) -> u32 {
    ev.color
        .unwrap_or(if ev.all_day() { p.sage } else { p.accent })
}

fn inset(r: Rect, dx: f32, dy: f32) -> Rect {
    Rect {
        x: r.x + dx,
        y: r.y + dy,
        w: (r.w - 2.0 * dx).max(0.0),
        h: (r.h - 2.0 * dy).max(0.0),
    }
}

fn phase_name(lang: Lang, phase: Phase) -> &'static str {
    text(
        lang,
        match phase {
            Phase::Focus => Key::Focus,
            Phase::ShortBreak => Key::Break,
            Phase::LongBreak => Key::LongBreak,
        },
    )
}

/// The phase after the current one and its minutes: a break after focus
/// (the long one after the last session), focus after a break.
fn next_phase(pomo: &Pomodoro) -> (Phase, u32) {
    let s = pomo.settings();
    match pomo.phase() {
        Phase::Focus if pomo.session() >= s.sessions => (Phase::LongBreak, s.long_break_min),
        Phase::Focus => (Phase::ShortBreak, s.short_break_min),
        Phase::ShortBreak | Phase::LongBreak => (Phase::Focus, s.focus_min),
    }
}

/// "+N lagi", or "Kembali ke atas" on the last chunk (`n` = 0).
fn more_label(lang: Lang, n: usize) -> String {
    if n == 0 {
        text(lang, Key::BackToTop).into()
    } else {
        i18n::more_count(lang, n)
    }
}

/// "09.00" (Indonesian) or "09:00": an hour line's label; 24 for the end of
/// the day.
fn hour_label(lang: Lang, hour: u32) -> String {
    let sep = match lang {
        Lang::Id => '.',
        Lang::En => ':',
    };
    format!("{hour:02}{sep}00")
}

/// "10.00 – 11.30", "Seharian", "Seharian · 24 Okt – 26 Okt", or both dates
/// for a timed event spanning days.
fn event_time_label(lang: Lang, ev: &CalEvent) -> String {
    let date = |d| i18n::short_date(lang, d);
    let hm = |t| i18n::time_hm(lang, t);
    match (&ev.start, &ev.end) {
        (EventTime::AllDay(s), EventTime::AllDay(e)) => {
            let all_day = text(lang, Key::AllDay);
            let last = e.pred_opt().unwrap_or(*s);
            if last <= *s {
                all_day.into()
            } else {
                format!("{all_day} · {} – {}", date(*s), date(last))
            }
        }
        (EventTime::At(s), EventTime::At(e)) => {
            let (s, e) = (s.naive_local(), e.naive_local());
            if s.date() == e.date() {
                format!("{} – {}", hm(s), hm(e))
            } else {
                format!(
                    "{} {} – {} {}",
                    date(s.date()),
                    hm(s),
                    date(e.date()),
                    hm(e)
                )
            }
        }
        _ => String::new(),
    }
}

/// "hari ini 09.12", "kemarin", "23 Okt", or "23 Okt 2025" in another year.
fn note_time_label(lang: Lang, updated: NaiveDateTime, today: NaiveDate) -> String {
    let d = updated.date();
    if d == today {
        format!(
            "{} {}",
            text(lang, Key::Today),
            i18n::time_hm(lang, updated)
        )
    } else if Some(d) == today.pred_opt() {
        text(lang, Key::Yesterday).into()
    } else if d.year() == today.year() {
        i18n::short_date(lang, d)
    } else {
        format!("{} {}", i18n::short_date(lang, d), d.year())
    }
}

/// A reminder row's time ("09.30") and text (its first line).
fn reminder_row(lang: Lang, r: &Reminder) -> (String, &str) {
    let time = local_time(r.due_unix)
        .map(|due| i18n::time_hm(lang, due))
        .unwrap_or_default();
    (time, first_line(&r.text))
}

/// A day group's heading: "Hari ini", "Besok", "Sel, 27 Okt", or
/// "3 Jan 2027" in another year.
fn day_label(lang: Lang, d: NaiveDate, today: NaiveDate) -> String {
    if d == today {
        text(lang, Key::DayToday).into()
    } else if Some(d) == today.succ_opt() {
        text(lang, Key::DayTomorrow).into()
    } else if d.year() == today.year() {
        format!(
            "{}, {}",
            i18n::day_short(lang, d),
            i18n::short_date(lang, d)
        )
    } else {
        format!("{} {}", i18n::short_date(lang, d), d.year())
    }
}

/// The first non-blank line, trimmed.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// A note card's snippet: the non-blank lines after the first, joined.
fn note_snippet(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whole seconds rounded up, so a fresh 25-minute phase shows "25:00".
fn mmss(d: Duration) -> String {
    let secs = d.as_millis().div_ceil(1000);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn compact_label(lang: Lang, pomo: &Pomodoro) -> String {
    if pomo.status() == PomoStatus::Ready {
        return text(lang, Key::ReadyClick).into();
    }
    match pomo.phase() {
        Phase::Focus => {
            let n = pomo.settings().sessions;
            let focus = text(lang, Key::Focus);
            format!("{focus} {}/{n}", pomo.session().min(n))
        }
        Phase::ShortBreak | Phase::LongBreak => text(lang, Key::Break).into(),
    }
}

/// Under the countdown: the phase, marked when paused or waiting.
fn phase_label(lang: Lang, pomo: &Pomodoro) -> String {
    let name = phase_name(lang, pomo.phase());
    match pomo.status() {
        PomoStatus::Paused { .. } => format!("{name} · {}", text(lang, Key::Paused)),
        PomoStatus::Ready => format!("{name} {}", text(lang, Key::Ready)),
        PomoStatus::Idle | PomoStatus::Running { .. } => name.into(),
    }
}

/// The primary button's icon: pause while running; start, resume or start
/// the next phase otherwise.
fn primary_icon(status: PomoStatus) -> Icon {
    match status {
        PomoStatus::Running { .. } => Icon::Pause,
        PomoStatus::Idle | PomoStatus::Ready | PomoStatus::Paused { .. } => Icon::Play,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pomodoro::PomoSettings;
    use crate::theme::{Accent, Surface, island};
    use chrono::{NaiveTime, TimeZone};
    use std::time::Instant;

    #[test]
    fn utf16_offsets_map_to_whole_characters() {
        assert_eq!(utf16_prefix("git status", 3), "git");
        assert_eq!(utf16_prefix("git", 0), "");
        assert_eq!(utf16_prefix("git", 9), "git");
        // "é" is one unit, "😀" two: an offset inside it stops before it.
        assert_eq!(utf16_prefix("é😀x", 1), "é");
        assert_eq!(utf16_prefix("é😀x", 2), "é");
        assert_eq!(utf16_prefix("é😀x", 3), "é😀");
    }

    fn cli_view<'a>(
        output: &'a crate::cli::Output,
        echoes: &'a crate::cli_panel::Echoes,
        run: RunState,
    ) -> CliView<'a> {
        CliView {
            output,
            echoes,
            scroll: 0,
            run,
            shell: crate::cli::Shell::PowerShell,
            cwd: "D:\\Proyek",
            input: "",
            selection: (0, 0),
            focused: true,
            suggestions: Vec::new(),
            selected: 0,
            tall: false,
        }
    }

    #[test]
    fn cli_chips_follow_the_run() {
        let (output, echoes) = Default::default();
        let t0 = Instant::now();
        let looks = |run| {
            cli_chips(
                Lang::Id,
                &cli_view(&output, &echoes, run),
                t0 + Duration::from_secs(42),
            )
            .into_iter()
            .map(|c| (c.text, c.look))
            .collect::<Vec<_>>()
        };
        let folder = ("D:\\Proyek".to_owned(), ChipLook::Path);
        let shell = ("PowerShell".to_owned(), ChipLook::Mono);
        assert_eq!(looks(RunState::Idle), [folder.clone(), shell.clone()]);
        assert_eq!(
            looks(RunState::Running { since: t0 }),
            [("Berjalan · 0:42".to_owned(), ChipLook::Run)]
        );
        let ended = |code, killed| RunState::Ended {
            code,
            killed,
            took: Duration::from_millis(2_100),
        };
        assert_eq!(
            looks(ended(Some(0), false)),
            [
                folder.clone(),
                ("selesai · 2,1 dtk".to_owned(), ChipLook::Ok),
                shell.clone()
            ]
        );
        assert_eq!(
            looks(ended(Some(1), false))[1],
            ("exit 1 · 2,1 dtk".to_owned(), ChipLook::Err)
        );
        assert_eq!(looks(ended(Some(0), true))[1].1, ChipLook::Err, "stopped");
    }

    #[test]
    fn track_time_reads_like_a_player() {
        assert_eq!(track_time(0), "0:00");
        assert_eq!(track_time(999), "0:00", "whole seconds, rounded down");
        assert_eq!(track_time(102_000), "1:42");
        assert_eq!(track_time(238_500), "3:58");
        assert_eq!(track_time(600_000), "10:00");
        assert_eq!(track_time(3_599_999), "59:59");
        assert_eq!(track_time(3_600_000), "1:00:00");
        assert_eq!(track_time(3_723_000), "1:02:03");
        assert_eq!(track_time(36_000_000), "10:00:00");
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(d: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
        d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    fn event(start: EventTime, end: EventTime) -> CalEvent {
        CalEvent {
            id: "e".into(),
            calendar_id: "primary".into(),
            title: "e".into(),
            start,
            end,
            description: String::new(),
            color: None,
            editable: true,
        }
    }

    fn local(t: NaiveDateTime) -> EventTime {
        EventTime::At(Local.from_local_datetime(&t).earliest().unwrap())
    }

    #[test]
    fn event_times() {
        let l = Lang::Id;
        let d = date(2026, 10, 25);
        let one = event(EventTime::AllDay(d), EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(event_time_label(l, &one), "Seharian");
        let multi = event(
            EventTime::AllDay(date(2026, 10, 24)),
            EventTime::AllDay(date(2026, 10, 27)),
        );
        assert_eq!(event_time_label(l, &multi), "Seharian · 24 Okt – 26 Okt");
        let timed = event(local(at(d, 10, 0)), local(at(d, 11, 30)));
        assert_eq!(event_time_label(l, &timed), "10.00 – 11.30");
        let night = event(local(at(d, 22, 0)), local(at(date(2026, 10, 26), 2, 0)));
        assert_eq!(event_time_label(l, &night), "25 Okt 22.00 – 26 Okt 02.00");
    }

    #[test]
    fn event_times_in_english() {
        let l = Lang::En;
        let d = date(2026, 10, 25);
        let one = event(EventTime::AllDay(d), EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(event_time_label(l, &one), "All day");
        let multi = event(
            EventTime::AllDay(date(2026, 10, 24)),
            EventTime::AllDay(date(2026, 10, 27)),
        );
        assert_eq!(event_time_label(l, &multi), "All day · 24 Oct – 26 Oct");
        let night = event(local(at(d, 22, 0)), local(at(date(2026, 10, 26), 2, 0)));
        assert_eq!(event_time_label(l, &night), "25 Oct 22:00 – 26 Oct 02:00");
    }

    #[test]
    fn event_colors_default_by_kind() {
        let p = island(Accent::Plum, Surface::Espresso);
        let d = date(2026, 10, 25);
        let all_day = event(EventTime::AllDay(d), EventTime::AllDay(date(2026, 10, 26)));
        assert_eq!(event_color(&all_day, &p), p.sage);
        let timed = event(local(at(d, 10, 0)), local(at(d, 11, 0)));
        assert_eq!(event_color(&timed, &p), p.accent);
        let own = CalEvent {
            color: Some(0x123456),
            ..timed
        };
        assert_eq!(event_color(&own, &p), 0x123456);
    }

    #[test]
    fn note_times_relative_to_today() {
        let l = Lang::Id;
        let today = date(2026, 10, 25);
        assert_eq!(
            note_time_label(l, at(today, 9, 12), today),
            "hari ini 09.12"
        );
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 24), 23, 0), today),
            "kemarin"
        );
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 23), 8, 0), today),
            "23 Okt"
        );
        assert_eq!(
            note_time_label(l, at(date(2025, 10, 23), 8, 0), today),
            "23 Okt 2025"
        );
    }

    #[test]
    fn note_times_in_english() {
        let l = Lang::En;
        let today = date(2026, 10, 25);
        assert_eq!(note_time_label(l, at(today, 9, 12), today), "Today 09:12");
        assert_eq!(
            note_time_label(l, at(date(2026, 10, 24), 23, 0), today),
            "Yesterday"
        );
        assert_eq!(
            note_time_label(l, at(date(2025, 10, 23), 8, 0), today),
            "23 Oct 2025"
        );
    }

    #[test]
    fn note_cards_split_title_and_snippet() {
        let text = "\n  Ide fasad  \nKisi-kisi kayu vertikal,\n\n  jarak 8 cm.\n";
        assert_eq!(first_line(text), "Ide fasad");
        assert_eq!(note_snippet(text), "Kisi-kisi kayu vertikal, jarak 8 cm.");
        assert_eq!(note_snippet("Hanya judul"), "");
        assert_eq!(note_snippet(""), "");
    }

    #[test]
    fn reminder_days_relative_to_today() {
        let today = date(2026, 10, 26);
        assert_eq!(day_label(Lang::Id, today, today), "Hari ini");
        assert_eq!(day_label(Lang::Id, date(2026, 10, 27), today), "Besok");
        assert_eq!(
            day_label(Lang::Id, date(2026, 10, 30), today),
            "Jum, 30 Okt"
        );
        assert_eq!(
            day_label(Lang::Id, date(2026, 10, 25), today),
            "Min, 25 Okt"
        );
        assert_eq!(day_label(Lang::Id, date(2027, 1, 3), today), "3 Jan 2027");
        assert_eq!(day_label(Lang::En, today, today), "Today");
        assert_eq!(day_label(Lang::En, date(2026, 10, 27), today), "Tomorrow");
        assert_eq!(
            day_label(Lang::En, date(2026, 10, 30), today),
            "Fri, 30 Oct"
        );
    }

    #[test]
    fn reminder_rows_show_time_and_first_line() {
        let today = date(2026, 10, 26);
        let unix = |t: NaiveDateTime| {
            Local
                .from_local_datetime(&t)
                .earliest()
                .unwrap()
                .timestamp()
        };
        let r = Reminder {
            id: "r".into(),
            text: "\nMinum obat\nsetelah makan".into(),
            due_unix: unix(at(today, 8, 5)),
            repeat: Repeat::Daily,
            done: false,
            days: Vec::new(),
        };
        assert_eq!(reminder_row(Lang::Id, &r), ("08.05".into(), "Minum obat"));
        assert_eq!(reminder_row(Lang::En, &r), ("08:05".into(), "Minum obat"));
    }

    #[test]
    fn hour_labels_follow_the_language() {
        assert_eq!(hour_label(Lang::Id, 9), "09.00");
        assert_eq!(hour_label(Lang::En, 13), "13:00");
        assert_eq!(hour_label(Lang::Id, 24), "24.00");
    }

    #[test]
    fn calendar_titles_per_view() {
        let d = date(2026, 10, 26);
        assert_eq!(
            calendar_title(Lang::Id, CalView::Month, d, d),
            "Oktober 2026"
        );
        assert_eq!(
            calendar_title(Lang::Id, CalView::Week, d, d),
            "26 Okt – 1 Nov 2026"
        );
        assert_eq!(
            calendar_title(Lang::Id, CalView::Day, d, d),
            "Senin, 26 Oktober"
        );
    }

    #[test]
    fn more_lines() {
        assert_eq!(more_label(Lang::Id, 0), "Kembali ke atas");
        assert_eq!(more_label(Lang::Id, 4), "+4 lagi");
        assert_eq!(more_label(Lang::En, 0), "Back to top");
        assert_eq!(more_label(Lang::En, 4), "+4 more");
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line("\n  \n  Beli tinta  \nlain"), "Beli tinta");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn mmss_rounds_up_and_allows_long_phases() {
        assert_eq!(mmss(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(mmss(Duration::from_millis(1_121_400)), "18:42");
        assert_eq!(mmss(Duration::from_secs(120 * 60)), "120:00");
        assert_eq!(mmss(Duration::ZERO), "00:00");
    }

    #[test]
    fn pomodoro_labels_by_status() {
        let l = Lang::Id;
        let pal = island(Accent::Terracotta, Surface::Espresso);
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        assert_eq!(primary_icon(p.status()), Icon::Play);
        assert_eq!(pal.phase(p.phase()), pal.accent);
        assert_eq!(next_phase(&p), (Phase::ShortBreak, 5));
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Fokus 1/4");
        assert_eq!(primary_icon(p.status()), Icon::Pause);
        assert_eq!(phase_label(l, &p), "Fokus");
        p.start_pause(t0);
        assert_eq!(primary_icon(p.status()), Icon::Play);
        assert_eq!(phase_label(l, &p), "Fokus · dijeda");
        p.skip(t0);
        assert_eq!(compact_label(l, &p), "Siap · klik");
        assert_eq!(phase_label(l, &p), "Istirahat siap");
        assert_eq!(primary_icon(p.status()), Icon::Play);
        assert_eq!(pal.phase(p.phase()), pal.sage);
        assert_eq!(next_phase(&p), (Phase::Focus, 25));
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Istirahat");
    }

    #[test]
    fn the_last_session_leads_to_the_long_break() {
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        // Focus 1, break, focus 2, break, focus 3, break, focus 4.
        for _ in 0..6 {
            p.skip(t0);
        }
        assert_eq!((p.phase(), p.session()), (Phase::Focus, 4));
        assert_eq!(next_phase(&p), (Phase::LongBreak, 15));
    }

    #[test]
    fn pomodoro_labels_in_english() {
        let l = Lang::En;
        let t0 = Instant::now();
        let mut p = Pomodoro::new(PomoSettings::default());
        p.start_pause(t0);
        assert_eq!(compact_label(l, &p), "Focus 1/4");
        p.start_pause(t0);
        assert_eq!(phase_label(l, &p), "Focus · paused");
        p.skip(t0);
        assert_eq!(compact_label(l, &p), "Ready · click");
        assert_eq!(phase_label(l, &p), "Break ready");
    }
}
