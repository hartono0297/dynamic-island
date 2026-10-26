//! Layout of the settings window in logical pixels (96 DPI): where each card,
//! text and control of a section goes, and what a click on it means. Pure,
//! so it is tested in both languages; `settings.rs` only draws what it says.
//!
//! Text widths come from a `Measure`: DirectWrite in the window, a
//! deliberately generous estimate in tests. Controls are sized to their
//! labels and pushed to the right edge of their card; row labels take the
//! space left over, so the tests check that the labels fit with the estimate.

use crate::calendar::CalStatus;
use crate::claude::install::HookState;
use crate::cli::Shell;
use crate::config::Style;
use crate::i18n::{Key, Lang, text, version_label};
use crate::model::PomoField;
use crate::pomodoro::{self, PomoSettings};
use crate::theme::{Accent, Surface, WindowTheme};

pub use super::geometry::Rect;

/// Client area.
pub const CLIENT_W: f32 = 720.0;
pub const CLIENT_H: f32 = 520.0;
pub const SIDEBAR_W: f32 = 184.0;
const PAD: f32 = 24.0;
const CARD_GAP: f32 = 12.0;
const ROW_H: f32 = 52.0;
pub const CARD_RADIUS: f32 = 12.0;
/// Horizontal padding inside a card.
const INSET: f32 = 16.0;
const CONTENT_X: f32 = SIDEBAR_W + PAD;
const CONTENT_W: f32 = CLIENT_W - CONTENT_X - PAD;
const TITLE_Y: f32 = 20.0;
const TITLE_H: f32 = 32.0;
const FIRST_CARD_Y: f32 = TITLE_Y + TITLE_H + 16.0;
/// Segmented controls and steppers.
pub const CONTROL_H: f32 = 28.0;
pub const BUTTON_H: f32 = 32.0;
/// Inner padding of a segmented control's track.
const SEG_PAD: f32 = 3.0;
const SEG_TEXT_PAD: f32 = 14.0;
const SEG_MIN_W: f32 = 64.0;
/// Segments share one width unless that makes the control wider than this.
const SEG_EQUAL_MAX: f32 = 280.0;
pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 22.0;
pub const SWATCH: f32 = 24.0;
const SWATCH_GAP: f32 = 12.0;
/// Swatches take clicks a little outside the circle.
const SWATCH_SLOP: f32 = 4.0;
pub const STEP_BUTTON: f32 = 28.0;
const STEP_VALUE_W: f32 = 64.0;
const BUTTON_PAD: f32 = 16.0;
const BUTTON_MIN_W: f32 = 88.0;
const LINK_PAD: f32 = 8.0;
/// Space taken by the glyph after a link's label.
pub const LINK_GLYPH_W: f32 = 18.0;
const GAP: f32 = 8.0;
const NAV_X: f32 = 8.0;
const NAV_W: f32 = SIDEBAR_W - 2.0 * NAV_X;
const NAV_Y: f32 = 72.0;
const NAV_H: f32 = 36.0;
const NAV_GAP: f32 = 4.0;
const PREVIEW_H: f32 = 112.0;
const FIELD_ROW_H: f32 = 44.0;
const FIELD_H: f32 = 32.0;
/// A status dot before a row's description.
const DOT: f32 = 8.0;
const DOT_GAP: f32 = 6.0;
const BATTERY_RANGE: (u8, u8, u8) = (5, 50, 5);

pub const REPO_URL: &str = "https://github.com/hartono0297/dynamic-island";
pub const RELEASES_URL: &str = "https://github.com/hartono0297/dynamic-island/releases";
pub const GUIDE_URL: &str = "https://github.com/hartono0297/dynamic-island#setup-google-calendar";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    General,
    Pomodoro,
    Integrations,
    About,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Appearance,
        Section::General,
        Section::Pomodoro,
        Section::Integrations,
        Section::About,
    ];

    pub fn key(self) -> Key {
        match self {
            Section::Appearance => Key::SecAppearance,
            Section::General => Key::SecGeneral,
            Section::Pomodoro => Key::SecPomodoro,
            Section::Integrations => Key::SecIntegrations,
            Section::About => Key::SecAbout,
        }
    }

    /// Segoe Fluent Icons glyph in the sidebar.
    pub fn glyph(self) -> &'static str {
        match self {
            Section::Appearance => "\u{E790}",
            Section::General => "\u{E713}",
            Section::Pomodoro => "\u{E916}",
            Section::Integrations => "\u{E71B}",
            Section::About => "\u{E946}",
        }
    }

    pub fn index(self) -> usize {
        Section::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// "appearance", "general", ... (also accepts the Indonesian names), for
    /// the debug `--settings` mode.
    #[cfg(any(test, debug_assertions))]
    pub fn from_name(name: &str) -> Option<Section> {
        match name.trim().to_ascii_lowercase().as_str() {
            "appearance" | "tampilan" => Some(Section::Appearance),
            "general" | "umum" => Some(Section::General),
            "pomodoro" => Some(Section::Pomodoro),
            "integrations" | "integrasi" => Some(Section::Integrations),
            "about" | "tentang" => Some(Section::About),
            _ => None,
        }
    }
}

/// Text styles; `settings.rs` maps each to a DirectWrite format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Brand,
    Nav,
    Title,
    Label,
    Desc,
    Control,
    Value,
    AboutName,
    AboutVersion,
}

impl Font {
    pub const ALL: [Font; 9] = [
        Font::Brand,
        Font::Nav,
        Font::Title,
        Font::Label,
        Font::Desc,
        Font::Control,
        Font::Value,
        Font::AboutName,
        Font::AboutVersion,
    ];

    pub fn size(self) -> f32 {
        match self {
            Font::Brand | Font::Nav | Font::Label => 14.0,
            Font::Title | Font::AboutName => 20.0,
            Font::Desc => 12.0,
            Font::Control | Font::Value | Font::AboutVersion => 13.0,
        }
    }

    pub fn semibold(self) -> bool {
        matches!(
            self,
            Font::Brand | Font::Title | Font::Value | Font::AboutName
        )
    }
}

pub trait Measure {
    /// Width of `text` on one line in `font`, in logical pixels.
    fn width(&self, text: &str, font: Font) -> f32;
}

/// A generous width estimate (wider than Segoe UI draws nearly any text), so
/// a layout that fits with it also fits on screen.
pub struct Estimate;

impl Measure for Estimate {
    fn width(&self, text: &str, font: Font) -> f32 {
        let per_char = if font.semibold() { 0.62 } else { 0.58 };
        text.chars().count() as f32 * font.size() * per_char
    }
}

/// What a click on a control does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Nav(Section),
    Style(Style),
    Accent(Accent),
    Surface(Surface),
    WindowMode(WindowTheme),
    Lang(Lang),
    Autostart,
    IdleClock,
    Lyrics,
    Shell(Shell),
    Battery(i32),
    DndHelp,
    Pomo(PomoField, i32),
    GoogleSave,
    GoogleConnect,
    GoogleDisconnect,
    GoogleGuide,
    HooksInstall,
    HooksRemove,
    Repo,
    Releases,
    DataFolder,
}

/// Colour role of a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Good,
    Warn,
    Bad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub rect: Rect,
    pub text: String,
    pub font: Font,
    pub tone: Tone,
    pub align: Align,
}

/// Which segmented control: animations follow it across layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Style,
    Surface,
    Window,
    Lang,
    Shell,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub hit: Hit,
    pub rect: Rect,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    /// The second click of "Putuskan": confirm in red.
    Confirm,
    /// Accent text without a fill, followed by an "open" glyph.
    Link,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub hit: Hit,
    pub rect: Rect,
    pub label: String,
    pub kind: ButtonKind,
    pub enabled: bool,
}

/// A native text field the window places over the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    ClientId,
    ClientSecret,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Segmented {
        group: Group,
        track: Rect,
        items: Vec<Segment>,
        selected: usize,
    },
    Toggle {
        hit: Hit,
        rect: Rect,
        on: bool,
    },
    /// `−  value  +`; a side is `None` at the end of the range or while locked.
    Stepper {
        minus: (Rect, Option<Hit>),
        plus: (Rect, Option<Hit>),
        value: Text,
        enabled: bool,
    },
    Swatches {
        items: Vec<(Accent, Rect)>,
        selected: Accent,
    },
    Button(Button),
}

impl Control {
    /// Everything clickable in the control, with the area that takes the click.
    pub fn hits(&self) -> Vec<(Hit, Rect)> {
        match self {
            Control::Segmented { items, .. } => items.iter().map(|s| (s.hit, s.rect)).collect(),
            Control::Toggle { hit, rect, .. } => vec![(*hit, *rect)],
            Control::Stepper { minus, plus, .. } => [minus, plus]
                .into_iter()
                .filter_map(|(rect, hit)| hit.map(|h| (h, *rect)))
                .collect(),
            Control::Swatches { items, .. } => items
                .iter()
                .map(|(a, r)| (Hit::Accent(*a), inflate(*r, SWATCH_SLOP)))
                .collect(),
            Control::Button(b) if b.enabled => vec![(b.hit, b.rect)],
            Control::Button(_) => Vec::new(),
        }
    }

    /// The area the control draws in.
    #[cfg(test)]
    pub fn bounds(&self) -> Rect {
        match self {
            Control::Segmented { track, .. } => *track,
            Control::Toggle { rect, .. } => *rect,
            Control::Stepper { minus, plus, .. } => union(minus.0, plus.0),
            Control::Swatches { items, .. } => items
                .iter()
                .map(|(_, r)| inflate(*r, SWATCH_SLOP))
                .reduce(union)
                .unwrap_or_default(),
            Control::Button(b) => b.rect,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Nav {
    pub section: Section,
    pub pill: Rect,
    pub glyph: Rect,
    pub label: Text,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub logo: Rect,
    pub brand: Option<Text>,
    pub nav: Vec<Nav>,
    pub title: Option<Text>,
    pub cards: Vec<Rect>,
    /// 1 px lines between the rows of a card.
    pub dividers: Vec<Rect>,
    pub texts: Vec<Text>,
    pub controls: Vec<Control>,
    /// Status dots before a description.
    pub dots: Vec<(Rect, Tone)>,
    /// The live island preview (Appearance).
    pub preview: Option<Rect>,
    /// The large logo (About).
    pub about_logo: Option<Rect>,
    /// Info glyph before a note (Pomodoro while locked).
    pub note_glyph: Option<Rect>,
    /// Rounded field backgrounds; the native edit sits inside.
    pub fields: Vec<(Field, Rect)>,
    /// The accent row's description, which names the swatch under the pointer.
    pub accent_desc: Option<usize>,
}

impl Layout {
    /// Every clickable area: sidebar rows, then the section's controls.
    pub fn hits(&self) -> Vec<(Hit, Rect)> {
        let nav = self.nav.iter().map(|n| (Hit::Nav(n.section), n.pill));
        nav.chain(self.controls.iter().flat_map(Control::hits))
            .collect()
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits()
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(h, _)| h)
    }

    /// The segmented control of `group`, if this section has one.
    #[cfg(test)]
    pub fn segmented(&self, group: Group) -> Option<(&Rect, &[Segment], usize)> {
        self.controls.iter().find_map(|c| match c {
            Control::Segmented {
                group: g,
                track,
                items,
                selected,
            } if *g == group => Some((track, items.as_slice(), *selected)),
            _ => None,
        })
    }
}

/// Google Calendar as the settings window shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoogleState {
    pub status: CalStatus,
    /// The fields differ from the saved config.
    pub dirty: bool,
    /// The Client ID field has text.
    pub has_id: bool,
    /// "Putuskan" was clicked once and waits for the second click.
    pub confirm: bool,
}

/// Everything a layout depends on besides the section.
#[derive(Clone, Debug, PartialEq)]
pub struct State<'a> {
    pub lang: Lang,
    pub style: Style,
    pub accent: Accent,
    pub surface: Surface,
    pub window: WindowTheme,
    /// The idle island shows the clock.
    pub idle_clock: bool,
    /// Synced lyrics in the notch.
    pub lyrics: bool,
    /// The CLI page's shell.
    pub shell: Shell,
    pub autostart: bool,
    pub battery_threshold: u8,
    pub has_battery: bool,
    pub pomodoro: PomoSettings,
    /// A phase runs or is paused: the steppers are off.
    pub pomodoro_locked: bool,
    pub google: GoogleState,
    /// A short message beside the Google buttons.
    pub google_note: Option<(&'a str, Tone)>,
    /// `None` when Claude Code's settings could not be read.
    pub hooks: Option<HookState>,
    /// The outcome of the last hook button, shown instead of the status.
    pub hook_note: Option<(&'a str, Tone)>,
    pub version: &'a str,
}

pub fn accent_key(a: Accent) -> Key {
    match a {
        Accent::Terracotta => Key::AccentTerracotta,
        Accent::Sage => Key::AccentSage,
        Accent::Ochre => Key::AccentOchre,
        Accent::Clay => Key::AccentClay,
        Accent::Olive => Key::AccentOlive,
        Accent::Plum => Key::AccentPlum,
    }
}

pub fn status_key(s: CalStatus) -> (Key, Tone) {
    match s {
        CalStatus::NotConfigured => (Key::StatusNotConfigured, Tone::Secondary),
        CalStatus::NeedsLogin => (Key::StatusNeedsLogin, Tone::Warn),
        CalStatus::Reconnect => (Key::StatusReconnect, Tone::Warn),
        CalStatus::Ok => (Key::StatusConnected, Tone::Good),
        CalStatus::Offline => (Key::StatusOffline, Tone::Secondary),
    }
}

/// The value a stepper shows: "25 mnt", "4", "20%".
pub fn pomo_value(lang: Lang, field: PomoField, v: u32) -> String {
    match field {
        PomoField::Sessions => v.to_string(),
        _ => format!("{v} {}", text(lang, Key::MinutesShort)),
    }
}

pub fn pomo_get(p: &PomoSettings, field: PomoField) -> u32 {
    match field {
        PomoField::Focus => p.focus_min,
        PomoField::ShortBreak => p.short_break_min,
        PomoField::LongBreak => p.long_break_min,
        PomoField::Sessions => p.sessions,
    }
}

/// The battery threshold one step up (`dir > 0`) or down, kept in 5–50.
pub fn battery_step(threshold: u8, dir: i32) -> u8 {
    let (min, max, step) = BATTERY_RANGE;
    let stepped = if dir > 0 {
        threshold.saturating_add(step)
    } else {
        threshold.saturating_sub(step)
    };
    // A hand-edited value off the 5 % grid snaps onto it.
    (stepped / step * step).clamp(min, max)
}

fn inflate(r: Rect, by: f32) -> Rect {
    Rect {
        x: r.x - by,
        y: r.y - by,
        w: r.w + 2.0 * by,
        h: r.h + 2.0 * by,
    }
}

#[cfg(test)]
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: a.right().max(b.right()) - x,
        h: a.bottom().max(b.bottom()) - y,
    }
}

struct Builder<'a> {
    m: &'a dyn Measure,
    lang: Lang,
    l: Layout,
    /// Top of the next card.
    y: f32,
}

impl Builder<'_> {
    fn t(&self, key: Key) -> String {
        text(self.lang, key).to_owned()
    }

    fn w(&self, s: &str, font: Font) -> f32 {
        self.m.width(s, font)
    }

    fn card(&mut self, h: f32) -> Rect {
        let r = Rect {
            x: CONTENT_X,
            y: self.y,
            w: CONTENT_W,
            h,
        };
        self.l.cards.push(r);
        self.y += h + CARD_GAP;
        r
    }

    fn divider(&mut self, card: Rect, y: f32) {
        self.l.dividers.push(Rect {
            x: card.x + INSET,
            y: y - 0.5,
            w: card.w - 2.0 * INSET,
            h: 1.0,
        });
    }

    fn text(&mut self, rect: Rect, s: String, font: Font, tone: Tone, align: Align) -> usize {
        self.l.texts.push(Text {
            rect,
            text: s,
            font,
            tone,
            align,
        });
        self.l.texts.len() - 1
    }

    /// A row's label and description, left of `control_x`. With `dot`, the
    /// description starts after a status dot in its tone. Returns the
    /// description's index in `texts`.
    fn row_text(
        &mut self,
        card: Rect,
        y: f32,
        label: String,
        desc: Option<(String, Tone)>,
        dot: bool,
        control_x: f32,
    ) -> Option<usize> {
        let x = card.x + INSET;
        let w = (control_x - 2.0 * GAP - x).max(0.0);
        let Some((desc, tone)) = desc else {
            let rect = Rect {
                x,
                y: y + 16.0,
                w,
                h: 20.0,
            };
            self.text(rect, label, Font::Label, Tone::Primary, Align::Left);
            return None;
        };
        let label_rect = Rect {
            x,
            y: y + 7.0,
            w,
            h: 20.0,
        };
        self.text(label_rect, label, Font::Label, Tone::Primary, Align::Left);
        let mut dx = 0.0;
        if dot {
            self.l.dots.push((
                Rect {
                    x,
                    y: y + 28.0 + (16.0 - DOT) / 2.0,
                    w: DOT,
                    h: DOT,
                },
                tone,
            ));
            dx = DOT + DOT_GAP;
        }
        let desc_rect = Rect {
            x: x + dx,
            y: y + 28.0,
            w: (w - dx).max(0.0),
            h: 16.0,
        };
        let text_tone = if dot { Tone::Secondary } else { tone };
        Some(self.text(desc_rect, desc, Font::Desc, text_tone, Align::Left))
    }

    /// A segmented control right-aligned at `right`, centered on `cy`.
    fn segmented(
        &mut self,
        group: Group,
        right: f32,
        cy: f32,
        items: Vec<(Hit, String)>,
        selected: usize,
    ) -> f32 {
        let natural: Vec<f32> = items
            .iter()
            .map(|(_, s)| (self.w(s, Font::Control) + 2.0 * SEG_TEXT_PAD).max(SEG_MIN_W))
            .collect();
        let widest = natural.iter().copied().fold(0.0, f32::max);
        let equal = widest * items.len() as f32 + 2.0 * SEG_PAD <= SEG_EQUAL_MAX;
        let widths: Vec<f32> = if equal {
            vec![widest; items.len()]
        } else {
            natural
        };
        let total = widths.iter().sum::<f32>() + 2.0 * SEG_PAD;
        let track = Rect {
            x: right - total,
            y: cy - CONTROL_H / 2.0,
            w: total,
            h: CONTROL_H,
        };
        let mut x = track.x + SEG_PAD;
        let segs = items
            .into_iter()
            .zip(widths)
            .map(|((hit, label), w)| {
                let rect = Rect {
                    x,
                    y: track.y + SEG_PAD,
                    w,
                    h: CONTROL_H - 2.0 * SEG_PAD,
                };
                x += w;
                Segment { hit, rect, label }
            })
            .collect();
        self.l.controls.push(Control::Segmented {
            group,
            track,
            items: segs,
            selected,
        });
        track.x
    }

    fn toggle(&mut self, hit: Hit, right: f32, cy: f32, on: bool) -> f32 {
        let rect = Rect {
            x: right - TOGGLE_W,
            y: cy - TOGGLE_H / 2.0,
            w: TOGGLE_W,
            h: TOGGLE_H,
        };
        self.l.controls.push(Control::Toggle { hit, rect, on });
        rect.x
    }

    #[allow(clippy::too_many_arguments)]
    fn stepper(
        &mut self,
        right: f32,
        cy: f32,
        value: String,
        down: Hit,
        up: Hit,
        can: (bool, bool),
        enabled: bool,
    ) -> f32 {
        let total = 2.0 * STEP_BUTTON + STEP_VALUE_W;
        let y = cy - STEP_BUTTON / 2.0;
        let x = right - total;
        let button = |x| Rect {
            x,
            y,
            w: STEP_BUTTON,
            h: STEP_BUTTON,
        };
        let value = Text {
            rect: Rect {
                x: x + STEP_BUTTON,
                y,
                w: STEP_VALUE_W,
                h: STEP_BUTTON,
            },
            text: value,
            font: Font::Value,
            tone: Tone::Primary,
            align: Align::Center,
        };
        self.l.controls.push(Control::Stepper {
            minus: (button(x), (enabled && can.0).then_some(down)),
            plus: (
                button(right - STEP_BUTTON),
                (enabled && can.1).then_some(up),
            ),
            value,
            enabled,
        });
        x
    }

    fn button_w(&self, labels: &[&str], kind: ButtonKind) -> f32 {
        let widest = labels
            .iter()
            .map(|s| self.w(s, Font::Control))
            .fold(0.0, f32::max);
        match kind {
            ButtonKind::Link => widest + 2.0 * LINK_PAD + LINK_GLYPH_W,
            _ => (widest + 2.0 * BUTTON_PAD).max(BUTTON_MIN_W),
        }
    }

    /// Buttons right-aligned at `right` in the order given, centered on `cy`.
    /// Each is `(hit, label, kind, enabled, width)`. Returns the left edge.
    fn buttons(
        &mut self,
        right: f32,
        cy: f32,
        items: Vec<(Hit, String, ButtonKind, bool, f32)>,
    ) -> f32 {
        let total: f32 =
            items.iter().map(|i| i.4).sum::<f32>() + GAP * (items.len().max(1) - 1) as f32;
        let mut x = right - total;
        let left = x;
        for (hit, label, kind, enabled, w) in items {
            let h = if kind == ButtonKind::Link {
                CONTROL_H
            } else {
                BUTTON_H
            };
            self.l.controls.push(Control::Button(Button {
                hit,
                rect: Rect {
                    x,
                    y: cy - h / 2.0,
                    w,
                    h,
                },
                label,
                kind,
                enabled,
            }));
            x += w + GAP;
        }
        left
    }

    fn sidebar(&mut self) {
        self.l.logo = Rect {
            x: 20.0,
            y: 22.0,
            w: 28.0,
            h: 28.0,
        };
        self.l.brand = Some(Text {
            rect: Rect {
                x: 56.0,
                y: 22.0,
                w: SIDEBAR_W - 56.0 - 4.0,
                h: 28.0,
            },
            text: "Dynamic Island".into(),
            font: Font::Brand,
            tone: Tone::Primary,
            align: Align::Left,
        });
        for (i, s) in Section::ALL.into_iter().enumerate() {
            let pill = Rect {
                x: NAV_X,
                y: NAV_Y + i as f32 * (NAV_H + NAV_GAP),
                w: NAV_W,
                h: NAV_H,
            };
            let glyph = Rect {
                x: pill.x + 10.0,
                y: pill.y,
                w: 24.0,
                h: NAV_H,
            };
            let label_x = glyph.right() + 8.0;
            let label = Text {
                rect: Rect {
                    x: label_x,
                    y: pill.y,
                    w: pill.right() - 8.0 - label_x,
                    h: NAV_H,
                },
                text: self.t(s.key()),
                font: Font::Nav,
                tone: Tone::Primary,
                align: Align::Left,
            };
            self.l.nav.push(Nav {
                section: s,
                pill,
                glyph,
                label,
            });
        }
    }
}

const RIGHT: f32 = CONTENT_X + CONTENT_W - INSET;

/// The whole window for `section`.
pub fn layout(section: Section, s: &State, m: &dyn Measure) -> Layout {
    let mut b = Builder {
        m,
        lang: s.lang,
        l: Layout::default(),
        y: FIRST_CARD_Y,
    };
    b.sidebar();
    b.l.title = Some(Text {
        rect: Rect {
            x: CONTENT_X,
            y: TITLE_Y,
            w: CONTENT_W,
            h: TITLE_H,
        },
        text: b.t(section.key()),
        font: Font::Title,
        tone: Tone::Primary,
        align: Align::Left,
    });
    match section {
        Section::Appearance => appearance(&mut b, s),
        Section::General => general(&mut b, s),
        Section::Pomodoro => pomodoro_page(&mut b, s),
        Section::Integrations => integrations(&mut b, s),
        Section::About => about(&mut b, s),
    }
    b.l
}

fn index_of<T: PartialEq>(all: &[T], v: &T) -> usize {
    all.iter().position(|x| x == v).unwrap_or(0)
}

fn appearance(b: &mut Builder, s: &State) {
    let preview = b.card(PREVIEW_H);
    b.l.preview = Some(preview);
    let card = b.card(5.0 * ROW_H);
    let rows: [f32; 5] = std::array::from_fn(|i| card.y + i as f32 * ROW_H);
    let cy = |y: f32| y + ROW_H / 2.0;

    let styles = [
        (Style::Pill, Key::StylePill),
        (Style::Notch, Key::StyleNotch),
    ];
    let items = styles
        .iter()
        .map(|(v, k)| (Hit::Style(*v), b.t(*k)))
        .collect();
    let selected = index_of(&[Style::Pill, Style::Notch], &s.style);
    let x = b.segmented(Group::Style, RIGHT, cy(rows[0]), items, selected);
    let desc = Some((b.t(Key::IslandStyleDesc), Tone::Secondary));
    b.row_text(card, rows[0], b.t(Key::IslandStyle), desc, false, x);

    let total = Accent::ALL.len() as f32 * SWATCH + (Accent::ALL.len() - 1) as f32 * SWATCH_GAP;
    let x0 = RIGHT - total;
    let items = Accent::ALL
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let r = Rect {
                x: x0 + i as f32 * (SWATCH + SWATCH_GAP),
                y: cy(rows[1]) - SWATCH / 2.0,
                w: SWATCH,
                h: SWATCH,
            };
            (*a, r)
        })
        .collect();
    b.l.controls.push(Control::Swatches {
        items,
        selected: s.accent,
    });
    let desc = Some((b.t(accent_key(s.accent)), Tone::Secondary));
    b.l.accent_desc = b.row_text(
        card,
        rows[1],
        b.t(Key::AccentColor),
        desc,
        false,
        x0 - SWATCH_SLOP,
    );
    b.divider(card, rows[1]);

    let surfaces = [
        (Surface::Espresso, Key::SurfaceEspresso),
        (Surface::Graphite, Key::SurfaceGraphite),
    ];
    let items = surfaces
        .iter()
        .map(|(v, k)| (Hit::Surface(*v), b.t(*k)))
        .collect();
    let x = b.segmented(
        Group::Surface,
        RIGHT,
        cy(rows[2]),
        items,
        index_of(Surface::ALL, &s.surface),
    );
    let desc = Some((b.t(Key::IslandSurfaceDesc), Tone::Secondary));
    b.row_text(card, rows[2], b.t(Key::IslandSurface), desc, false, x);
    b.divider(card, rows[2]);

    let modes = [
        (WindowTheme::System, Key::ModeSystem),
        (WindowTheme::Dark, Key::ModeDark),
        (WindowTheme::Light, Key::ModeLight),
    ];
    let items = modes
        .iter()
        .map(|(v, k)| (Hit::WindowMode(*v), b.t(*k)))
        .collect();
    let x = b.segmented(
        Group::Window,
        RIGHT,
        cy(rows[3]),
        items,
        index_of(WindowTheme::ALL, &s.window),
    );
    let desc = Some((b.t(Key::WindowModeDesc), Tone::Secondary));
    b.row_text(card, rows[3], b.t(Key::WindowMode), desc, false, x);
    b.divider(card, rows[3]);

    let x = b.toggle(Hit::IdleClock, RIGHT, cy(rows[4]), s.idle_clock);
    let desc = Some((b.t(Key::IdleClockDesc), Tone::Secondary));
    b.row_text(card, rows[4], b.t(Key::IdleClock), desc, false, x);
    b.divider(card, rows[4]);
}

fn general(b: &mut Builder, s: &State) {
    let card = b.card(6.0 * ROW_H);
    let rows: [f32; 6] = std::array::from_fn(|i| card.y + i as f32 * ROW_H);
    let cy = |y: f32| y + ROW_H / 2.0;

    let items = vec![
        (Hit::Lang(Lang::Id), b.t(Key::LangIndonesian)),
        (Hit::Lang(Lang::En), b.t(Key::LangEnglish)),
    ];
    let selected = usize::from(s.lang == Lang::En);
    let x = b.segmented(Group::Lang, RIGHT, cy(rows[0]), items, selected);
    let desc = Some((b.t(Key::LanguageDesc), Tone::Secondary));
    b.row_text(card, rows[0], b.t(Key::Language), desc, false, x);

    let x = b.toggle(Hit::Autostart, RIGHT, cy(rows[1]), s.autostart);
    let desc = Some((b.t(Key::AutostartDesc), Tone::Secondary));
    b.row_text(card, rows[1], b.t(Key::Autostart), desc, false, x);
    b.divider(card, rows[1]);

    let (min, max, _) = BATTERY_RANGE;
    let t = s.battery_threshold;
    let x = b.stepper(
        RIGHT,
        cy(rows[2]),
        format!("{t}%"),
        Hit::Battery(-1),
        Hit::Battery(1),
        (t > min, t < max),
        true,
    );
    let desc = if s.has_battery {
        (b.t(Key::BatteryAlertDesc), Tone::Secondary)
    } else {
        (b.t(Key::NoBattery), Tone::Secondary)
    };
    b.row_text(card, rows[2], b.t(Key::BatteryAlert), Some(desc), false, x);
    b.divider(card, rows[2]);

    let label = b.t(Key::ShowHow);
    let w = b.button_w(&[&label], ButtonKind::Secondary);
    let item = (Hit::DndHelp, label, ButtonKind::Secondary, true, w);
    let x = b.buttons(RIGHT, cy(rows[3]), vec![item]);
    let desc = Some((b.t(Key::PopupsHelpDesc), Tone::Secondary));
    b.row_text(card, rows[3], b.t(Key::PopupsHelp), desc, false, x);
    b.divider(card, rows[3]);

    // The description doubles as the privacy note.
    let x = b.toggle(Hit::Lyrics, RIGHT, cy(rows[4]), s.lyrics);
    let desc = Some((b.t(Key::LyricsInNotchDesc), Tone::Secondary));
    b.row_text(card, rows[4], b.t(Key::LyricsInNotch), desc, false, x);
    b.divider(card, rows[4]);

    let items = vec![
        (Hit::Shell(Shell::PowerShell), b.t(Key::ShellPowerShell)),
        (Hit::Shell(Shell::Cmd), b.t(Key::ShellCmd)),
    ];
    let selected = index_of(&Shell::ALL, &s.shell);
    let x = b.segmented(Group::Shell, RIGHT, cy(rows[5]), items, selected);
    let desc = Some((b.t(Key::ShellCliDesc), Tone::Secondary));
    b.row_text(card, rows[5], b.t(Key::ShellCli), desc, false, x);
    b.divider(card, rows[5]);
}

fn pomodoro_page(b: &mut Builder, s: &State) {
    let card = b.card(4.0 * ROW_H);
    let fields = [
        (PomoField::Focus, Key::Focus, Key::FocusDesc),
        (PomoField::ShortBreak, Key::Break, Key::BreakDesc),
        (PomoField::LongBreak, Key::LongBreak, Key::LongBreakDesc),
        (PomoField::Sessions, Key::Sessions, Key::SessionsDesc),
    ];
    for (i, (field, label, desc)) in fields.into_iter().enumerate() {
        let y = card.y + i as f32 * ROW_H;
        let v = pomo_get(&s.pomodoro, field);
        let (min, max, _) = pomodoro::range(field);
        let x = b.stepper(
            RIGHT,
            y + ROW_H / 2.0,
            pomo_value(b.lang, field, v),
            Hit::Pomo(field, -1),
            Hit::Pomo(field, 1),
            (v > min, v < max),
            !s.pomodoro_locked,
        );
        let desc = Some((b.t(desc), Tone::Secondary));
        b.row_text(card, y, b.t(label), desc, false, x);
        if i > 0 {
            b.divider(card, y);
        }
    }
    if s.pomodoro_locked {
        let y = card.bottom() + 12.0;
        b.l.note_glyph = Some(Rect {
            x: CONTENT_X + 4.0,
            y,
            w: 16.0,
            h: 18.0,
        });
        let rect = Rect {
            x: CONTENT_X + 26.0,
            y,
            w: CONTENT_W - 26.0,
            h: 18.0,
        };
        let note = b.t(Key::PomodoroLocked);
        b.text(rect, note, Font::Desc, Tone::Secondary, Align::Left);
    }
}

fn integrations(b: &mut Builder, s: &State) {
    let g = s.google;
    let card = b.card(ROW_H + 2.0 * FIELD_ROW_H + 8.0 + ROW_H);

    // Header: name and status, the setup guide on the right.
    let guide = b.t(Key::SetupGuide);
    let w = b.button_w(&[&guide], ButtonKind::Link);
    let item = (Hit::GoogleGuide, guide, ButtonKind::Link, true, w);
    let x = b.buttons(RIGHT + LINK_PAD, card.y + ROW_H / 2.0, vec![item]);
    let (key, tone) = status_key(g.status);
    let desc = Some((b.t(key), tone));
    b.row_text(card, card.y, b.t(Key::GoogleCalendar), desc, true, x);
    b.divider(card, card.y + ROW_H);

    // Client ID and secret: label column, then the field.
    let labels = [b.t(Key::ClientId), b.t(Key::ClientSecret)];
    let label_w = labels
        .iter()
        .map(|l| b.w(l, Font::Label))
        .fold(0.0, f32::max);
    let field_x = card.x + INSET + label_w + 2.0 * GAP;
    let top = card.y + ROW_H + 4.0;
    for (i, (field, label)) in [Field::ClientId, Field::ClientSecret]
        .into_iter()
        .zip(labels)
        .enumerate()
    {
        let y = top + i as f32 * FIELD_ROW_H;
        let rect = Rect {
            x: card.x + INSET,
            y: y + (FIELD_ROW_H - 20.0) / 2.0,
            w: label_w + GAP,
            h: 20.0,
        };
        b.text(rect, label, Font::Label, Tone::Primary, Align::Left);
        b.l.fields.push((
            field,
            Rect {
                x: field_x,
                y: y + (FIELD_ROW_H - FIELD_H) / 2.0,
                w: RIGHT - field_x,
                h: FIELD_H,
            },
        ));
    }
    let buttons_y = top + 2.0 * FIELD_ROW_H + 4.0;
    b.divider(card, buttons_y);

    // Buttons, with a short message on their left.
    let (save, connect) = (b.t(Key::Save), b.t(Key::Connect));
    let (disconnect, sure) = (b.t(Key::Disconnect), b.t(Key::ConfirmShort));
    // A login to forget: connected, offline, or one Google stopped accepting.
    let can_disconnect = matches!(
        g.status,
        CalStatus::Ok | CalStatus::Offline | CalStatus::Reconnect
    );
    let needs_login = matches!(g.status, CalStatus::NeedsLogin | CalStatus::Reconnect);
    let kind = |primary: bool| {
        if primary {
            ButtonKind::Primary
        } else {
            ButtonKind::Secondary
        }
    };
    // Connected already: nothing to connect.
    let can_connect =
        g.has_id && (g.status != CalStatus::NotConfigured || g.dirty) && g.status != CalStatus::Ok;
    let items = vec![
        (
            Hit::GoogleSave,
            save.clone(),
            kind(g.dirty),
            g.dirty,
            b.button_w(&[&save], ButtonKind::Secondary),
        ),
        (
            Hit::GoogleConnect,
            connect.clone(),
            kind(!g.dirty && can_connect && needs_login),
            can_connect,
            b.button_w(&[&connect], ButtonKind::Secondary),
        ),
        (
            Hit::GoogleDisconnect,
            if g.confirm {
                sure.clone()
            } else {
                disconnect.clone()
            },
            if g.confirm {
                ButtonKind::Confirm
            } else {
                ButtonKind::Secondary
            },
            can_disconnect,
            b.button_w(&[&disconnect, &sure], ButtonKind::Secondary),
        ),
    ];
    let row_cy = buttons_y + ROW_H / 2.0;
    let x = b.buttons(RIGHT, row_cy, items);
    if let Some((note, tone)) = s.google_note {
        let rect = Rect {
            x: card.x + INSET,
            y: row_cy - 9.0,
            w: (x - 2.0 * GAP - card.x - INSET).max(0.0),
            h: 18.0,
        };
        b.text(rect, note.to_owned(), Font::Desc, tone, Align::Left);
    }

    // Claude Code hooks.
    let card = b.card(ROW_H);
    let (install, remove) = (b.t(Key::InstallHooks), b.t(Key::RemoveHooks));
    // Unreadable settings (`None`): both stay on, so a click reports why.
    let complete = s.hooks.is_some_and(|h| h.complete);
    let any = s.hooks.is_none_or(|h| h.any);
    let items = vec![
        (
            Hit::HooksInstall,
            install.clone(),
            kind(!complete),
            !complete,
            b.button_w(&[&install], ButtonKind::Secondary),
        ),
        (
            Hit::HooksRemove,
            remove.clone(),
            ButtonKind::Secondary,
            any,
            b.button_w(&[&remove], ButtonKind::Secondary),
        ),
    ];
    let x = b.buttons(RIGHT, card.y + ROW_H / 2.0, items);
    let desc = match s.hook_note {
        Some((note, tone)) => (note.to_owned(), tone),
        None => match s.hooks {
            Some(h) if h.complete => (b.t(Key::HooksOn), Tone::Good),
            Some(_) => (b.t(Key::HooksOff), Tone::Secondary),
            None => (b.t(Key::HooksUnknown), Tone::Warn),
        },
    };
    b.row_text(card, card.y, b.t(Key::ClaudeCode), Some(desc), true, x);
}

fn about(b: &mut Builder, s: &State) {
    let logo = 64.0;
    b.l.about_logo = Some(Rect {
        x: CONTENT_X + (CONTENT_W - logo) / 2.0,
        y: FIRST_CARD_Y + 4.0,
        w: logo,
        h: logo,
    });
    let line = |y, h| Rect {
        x: CONTENT_X,
        y,
        w: CONTENT_W,
        h,
    };
    let name_y = FIRST_CARD_Y + 4.0 + logo + 10.0;
    b.text(
        line(name_y, 28.0),
        "Dynamic Island".into(),
        Font::AboutName,
        Tone::Primary,
        Align::Center,
    );
    let version = version_label(b.lang, s.version);
    b.text(
        line(name_y + 28.0, 18.0),
        version,
        Font::AboutVersion,
        Tone::Secondary,
        Align::Center,
    );
    let tagline = b.t(Key::AboutTagline);
    b.text(
        line(name_y + 46.0, 18.0),
        tagline,
        Font::AboutVersion,
        Tone::Secondary,
        Align::Center,
    );
    b.y = name_y + 46.0 + 18.0 + 20.0;

    let card = b.card(3.0 * ROW_H);
    let rows = [
        (
            Hit::Repo,
            Key::SourceCode,
            "hartono0297/dynamic-island".to_owned(),
            Key::OpenGithub,
        ),
        (
            Hit::Releases,
            Key::Updates,
            b.t(Key::UpdatesDesc),
            Key::SeeReleases,
        ),
        (
            Hit::DataFolder,
            Key::DataFolder,
            r"%APPDATA%\DynamicIsland".to_owned(),
            Key::OpenDataFolder,
        ),
    ];
    // One width for the three buttons, so they line up.
    let labels: Vec<String> = rows.iter().map(|r| b.t(r.3)).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let w = b.button_w(&refs, ButtonKind::Secondary);
    for (i, ((hit, label, desc, _), button)) in rows.into_iter().zip(labels).enumerate() {
        let y = card.y + i as f32 * ROW_H;
        let item = (hit, button, ButtonKind::Secondary, true, w);
        let x = b.buttons(RIGHT, y + ROW_H / 2.0, vec![item]);
        b.row_text(card, y, b.t(label), Some((desc, Tone::Secondary)), false, x);
        if i > 0 {
            b.divider(card, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states() -> Vec<State<'static>> {
        let mut out = Vec::new();
        for lang in [Lang::Id, Lang::En] {
            for (status, dirty, has_id, confirm) in [
                (CalStatus::NotConfigured, false, false, false),
                (CalStatus::NotConfigured, true, true, false),
                (CalStatus::NeedsLogin, false, true, false),
                (CalStatus::Reconnect, true, true, false),
                (CalStatus::Ok, false, true, false),
                (CalStatus::Ok, false, true, true),
                (CalStatus::Offline, false, true, false),
            ] {
                for (hooks, locked, battery) in [
                    (Some((true, true)), false, true),
                    (Some((false, true)), false, true),
                    (Some((false, false)), true, false),
                    (None, true, true),
                ] {
                    let hooks = hooks.map(|(complete, any)| HookState { complete, any });
                    out.push(State {
                        lang,
                        style: Style::Notch,
                        accent: Accent::Ochre,
                        surface: Surface::Graphite,
                        window: WindowTheme::System,
                        idle_clock: locked,
                        lyrics: !locked,
                        shell: if locked {
                            Shell::Cmd
                        } else {
                            Shell::PowerShell
                        },
                        autostart: true,
                        battery_threshold: 50,
                        has_battery: battery,
                        pomodoro: PomoSettings {
                            focus_min: 120,
                            short_break_min: 30,
                            long_break_min: 60,
                            sessions: 12,
                        },
                        pomodoro_locked: locked,
                        google: GoogleState {
                            status,
                            dirty,
                            has_id,
                            confirm,
                        },
                        google_note: None,
                        hooks,
                        hook_note: None,
                        version: "2.1.0",
                    });
                }
            }
        }
        // Every fixed message beside the buttons, in both languages.
        let notes = [
            Key::Saved,
            Key::CheckBrowser,
            Key::Disconnected,
            Key::HooksOn,
            Key::HooksAlready,
            Key::HooksRemoved,
            Key::HooksNone,
        ];
        let base = out[0].clone();
        for lang in [Lang::Id, Lang::En] {
            for key in notes {
                out.push(State {
                    lang,
                    google_note: Some((text(lang, key), Tone::Good)),
                    hook_note: Some((text(lang, key), Tone::Good)),
                    ..base.clone()
                });
            }
        }
        out
    }

    fn all_layouts() -> Vec<(Section, Lang, Layout)> {
        let mut out = Vec::new();
        for s in states() {
            for section in Section::ALL {
                out.push((section, s.lang, layout(section, &s, &Estimate)));
            }
        }
        out
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x - 0.01
            && inner.y >= outer.y - 0.01
            && inner.right() <= outer.right() + 0.01
            && inner.bottom() <= outer.bottom() + 0.01
    }

    const CLIENT: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: CLIENT_W,
        h: CLIENT_H,
    };

    #[test]
    fn hit_regions_never_overlap() {
        for (section, lang, l) in all_layouts() {
            let hits = l.hits();
            for (i, (ha, a)) in hits.iter().enumerate() {
                for (hb, b) in &hits[i + 1..] {
                    assert!(
                        !overlaps(*a, *b),
                        "{section:?} {lang:?}: {ha:?} overlaps {hb:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn controls_and_fields_sit_inside_their_cards() {
        for (section, lang, l) in all_layouts() {
            for c in &l.controls {
                let bounds = c.bounds();
                assert!(
                    l.cards.iter().any(|card| inside(bounds, *card)),
                    "{section:?} {lang:?}: {c:?} outside every card"
                );
            }
            for (f, r) in &l.fields {
                assert!(
                    l.cards.iter().any(|card| inside(*r, *card)),
                    "{section:?} {lang:?}: {f:?}"
                );
            }
            for card in &l.cards {
                assert!(inside(*card, CLIENT), "{section:?}: card {card:?}");
                assert!(card.x >= SIDEBAR_W + PAD - 0.01);
            }
            for n in &l.nav {
                assert!(n.pill.right() <= SIDEBAR_W, "{n:?}");
            }
        }
    }

    #[test]
    fn controls_do_not_overlap_each_other_or_texts() {
        for (section, lang, l) in all_layouts() {
            let mut boxes: Vec<(String, Rect)> = l
                .controls
                .iter()
                .map(|c| (format!("{c:?}"), c.bounds()))
                .collect();
            boxes.extend(l.fields.iter().map(|(f, r)| (format!("{f:?}"), *r)));
            for (i, (na, a)) in boxes.iter().enumerate() {
                for (nb, b) in &boxes[i + 1..] {
                    assert!(!overlaps(*a, *b), "{section:?} {lang:?}: {na} / {nb}");
                }
                for t in &l.texts {
                    assert!(
                        !overlaps(*a, t.rect),
                        "{section:?} {lang:?}: {na} covers {:?}",
                        t.text
                    );
                }
            }
        }
    }

    /// Every text fits its box with the generous estimate, in both languages.
    #[test]
    fn texts_fit_in_both_languages() {
        let m = Estimate;
        for (section, lang, l) in all_layouts() {
            let mut texts: Vec<&Text> = l.texts.iter().collect();
            texts.extend(l.title.iter());
            texts.extend(l.brand.iter());
            texts.extend(l.nav.iter().map(|n| &n.label));
            for c in &l.controls {
                if let Control::Stepper { value, .. } = c {
                    texts.push(value);
                }
            }
            for t in texts {
                let w = m.width(&t.text, t.font);
                assert!(
                    w <= t.rect.w + 0.01,
                    "{section:?} {lang:?}: {:?} needs {w:.0} px, has {:.0}",
                    t.text,
                    t.rect.w
                );
                assert!(t.rect.h >= t.font.size(), "{:?} too short", t.text);
                assert!(inside(t.rect, CLIENT), "{:?}", t.text);
            }
            for c in &l.controls {
                let labels: Vec<(&str, Rect, f32)> = match c {
                    Control::Segmented { items, .. } => items
                        .iter()
                        .map(|s| (s.label.as_str(), s.rect, 0.0))
                        .collect(),
                    Control::Button(b) if b.kind == ButtonKind::Link => {
                        vec![(b.label.as_str(), b.rect, LINK_GLYPH_W)]
                    }
                    Control::Button(b) => vec![(b.label.as_str(), b.rect, 0.0)],
                    _ => Vec::new(),
                };
                for (label, rect, extra) in labels {
                    let w = m.width(label, Font::Control) + extra;
                    assert!(w <= rect.w, "{section:?} {lang:?}: {label:?}");
                }
            }
            // The accent description names any swatch under the pointer.
            if let Some(i) = l.accent_desc {
                for a in Accent::ALL {
                    let name = text(lang, accent_key(*a));
                    assert!(m.width(name, Font::Desc) <= l.texts[i].rect.w);
                }
            }
        }
    }

    #[test]
    fn everything_fits_in_the_window_height() {
        for (section, lang, l) in all_layouts() {
            let bottom = l
                .cards
                .iter()
                .map(Rect::bottom)
                .chain(l.texts.iter().map(|t| t.rect.bottom()))
                .fold(0.0, f32::max);
            assert!(bottom <= CLIENT_H - PAD, "{section:?} {lang:?}: {bottom}");
        }
    }

    #[test]
    fn hits_follow_state() {
        let s = &states()[0];
        let l = layout(Section::Appearance, s, &Estimate);
        let (_, items, selected) = l.segmented(Group::Style).unwrap();
        assert_eq!(items[selected].hit, Hit::Style(Style::Notch));
        let r = items[0].rect;
        assert_eq!(
            l.hit(r.x + r.w / 2.0, r.y + r.h / 2.0),
            Some(Hit::Style(Style::Pill))
        );
        // Nav rows switch sections.
        let nav = &l.nav[3];
        assert_eq!(
            l.hit(nav.pill.x + 4.0, nav.pill.y + 4.0),
            Some(Hit::Nav(Section::Integrations))
        );
        // Locked steppers take no clicks; at the end of a range one side is off.
        let locked = State {
            pomodoro_locked: true,
            ..s.clone()
        };
        let l = layout(Section::Pomodoro, &locked, &Estimate);
        assert!(!l.hits().iter().any(|(h, _)| matches!(h, Hit::Pomo(..))));
        assert!(l.note_glyph.is_some());
        let free = State {
            pomodoro_locked: false,
            ..s.clone()
        };
        let l = layout(Section::Pomodoro, &free, &Estimate);
        let pomo: Vec<Hit> = l
            .hits()
            .into_iter()
            .map(|(h, _)| h)
            .filter(|h| matches!(h, Hit::Pomo(..)))
            .collect();
        // Every value is at its maximum: only the minus buttons work.
        assert_eq!(pomo.len(), 4);
        assert!(pomo.iter().all(|h| matches!(h, Hit::Pomo(_, -1))));
    }

    #[test]
    fn google_buttons_follow_the_status() {
        let base = states()[0].clone();
        let enabled = |g: GoogleState| -> Vec<Hit> {
            let s = State {
                google: g,
                ..base.clone()
            };
            let l = layout(Section::Integrations, &s, &Estimate);
            l.hits()
                .into_iter()
                .map(|(h, _)| h)
                .filter(|h| {
                    matches!(
                        h,
                        Hit::GoogleSave | Hit::GoogleConnect | Hit::GoogleDisconnect
                    )
                })
                .collect()
        };
        let g = |status, dirty, has_id| GoogleState {
            status,
            dirty,
            has_id,
            confirm: false,
        };
        assert!(enabled(g(CalStatus::NotConfigured, false, false)).is_empty());
        assert_eq!(
            enabled(g(CalStatus::NotConfigured, true, true)),
            [Hit::GoogleSave, Hit::GoogleConnect]
        );
        assert_eq!(
            enabled(g(CalStatus::NeedsLogin, false, true)),
            [Hit::GoogleConnect]
        );
        // Connected: nothing to connect, only to forget.
        assert_eq!(
            enabled(g(CalStatus::Ok, false, true)),
            [Hit::GoogleDisconnect]
        );
        // A refused login can be redone or forgotten.
        assert_eq!(
            enabled(g(CalStatus::Reconnect, false, true)),
            [Hit::GoogleConnect, Hit::GoogleDisconnect]
        );
        assert_eq!(
            enabled(g(CalStatus::Offline, false, true)),
            [Hit::GoogleConnect, Hit::GoogleDisconnect]
        );
    }

    #[test]
    fn hook_buttons_follow_what_is_installed() {
        let base = states()[0].clone();
        // Buttons in drawing order (left to right) with whether they take clicks.
        let buttons = |hooks: Option<HookState>| -> Vec<(Hit, bool)> {
            let s = State {
                hooks,
                ..base.clone()
            };
            let l = layout(Section::Integrations, &s, &Estimate);
            let mut out: Vec<(f32, Hit, bool)> = l
                .controls
                .iter()
                .filter_map(|c| match c {
                    Control::Button(b) if matches!(b.hit, Hit::HooksInstall | Hit::HooksRemove) => {
                        Some((b.rect.x, b.hit, b.enabled))
                    }
                    _ => None,
                })
                .collect();
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.into_iter().map(|(_, h, e)| (h, e)).collect()
        };
        let state = |complete, any| Some(HookState { complete, any });
        assert_eq!(
            buttons(state(true, true)),
            [(Hit::HooksInstall, false), (Hit::HooksRemove, true)]
        );
        assert_eq!(
            buttons(state(false, true)),
            [(Hit::HooksInstall, true), (Hit::HooksRemove, true)],
            "a partial install can be completed or removed"
        );
        assert_eq!(
            buttons(state(false, false)),
            [(Hit::HooksInstall, true), (Hit::HooksRemove, false)]
        );
        // Unreadable settings: both stay clickable, so a click says why.
        assert_eq!(
            buttons(None),
            [(Hit::HooksInstall, true), (Hit::HooksRemove, true)]
        );
    }

    #[test]
    fn idle_clock_toggle_shows_the_setting() {
        for on in [true, false] {
            let s = State {
                idle_clock: on,
                ..states()[0].clone()
            };
            let l = layout(Section::Appearance, &s, &Estimate);
            let toggle = l.controls.iter().find_map(|c| match c {
                Control::Toggle { hit, rect, on } if *hit == Hit::IdleClock => Some((*rect, *on)),
                _ => None,
            });
            let (rect, shown) = toggle.expect("idle clock toggle in Appearance");
            assert_eq!(shown, on);
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            assert_eq!(l.hit(cx, cy), Some(Hit::IdleClock));
        }
    }

    #[test]
    fn lyrics_toggle_in_general_with_privacy_note() {
        for lang in [Lang::Id, Lang::En] {
            for on in [true, false] {
                let s = State {
                    lang,
                    lyrics: on,
                    ..states()[0].clone()
                };
                let l = layout(Section::General, &s, &Estimate);
                let toggle = l.controls.iter().find_map(|c| match c {
                    Control::Toggle { hit, rect, on } if *hit == Hit::Lyrics => Some((*rect, *on)),
                    _ => None,
                });
                let (rect, shown) = toggle.expect("lyrics toggle in General");
                assert_eq!(shown, on);
                let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                assert_eq!(l.hit(cx, cy), Some(Hit::Lyrics));
                for key in [Key::LyricsInNotch, Key::LyricsInNotchDesc] {
                    assert!(
                        l.texts.iter().any(|t| t.text == text(lang, key)),
                        "{lang:?} {key:?}"
                    );
                }
            }
        }
        // Not in Appearance, which is full.
        let l = layout(Section::Appearance, &states()[0], &Estimate);
        assert!(!l.hits().iter().any(|(h, _)| *h == Hit::Lyrics));
    }

    #[test]
    fn shell_choice_is_the_last_row_of_general() {
        for lang in [Lang::Id, Lang::En] {
            for shell in Shell::ALL {
                let s = State {
                    lang,
                    shell,
                    ..states()[0].clone()
                };
                let l = layout(Section::General, &s, &Estimate);
                let (track, items, selected) = l.segmented(Group::Shell).expect("Shell CLI");
                assert_eq!(items[selected].hit, Hit::Shell(shell));
                let hits: Vec<Hit> = items.iter().map(|i| i.hit).collect();
                assert_eq!(
                    hits,
                    [Hit::Shell(Shell::PowerShell), Hit::Shell(Shell::Cmd)]
                );
                let r = items[1].rect;
                assert_eq!(
                    l.hit(r.x + r.w / 2.0, r.y + r.h / 2.0),
                    Some(Hit::Shell(Shell::Cmd))
                );
                // Below every other control of the card.
                for c in &l.controls {
                    if !matches!(
                        c,
                        Control::Segmented {
                            group: Group::Shell,
                            ..
                        }
                    ) {
                        assert!(c.bounds().bottom() <= track.y, "{c:?}");
                    }
                }
                for key in [Key::ShellCli, Key::ShellCliDesc] {
                    assert!(l.texts.iter().any(|t| t.text == text(lang, key)));
                }
            }
        }
        let l = layout(Section::Appearance, &states()[0], &Estimate);
        assert!(l.segmented(Group::Shell).is_none());
    }

    #[test]
    fn battery_steps_by_five_within_range() {
        assert_eq!(battery_step(20, 1), 25);
        assert_eq!(battery_step(20, -1), 15);
        assert_eq!(battery_step(50, 1), 50);
        assert_eq!(battery_step(5, -1), 5);
        // Off-grid hand edits snap.
        assert_eq!(battery_step(22, 1), 25);
        assert_eq!(battery_step(22, -1), 15);
        assert_eq!(battery_step(90, -1), 50);
    }

    #[test]
    fn section_names() {
        assert_eq!(Section::from_name("Integrasi"), Some(Section::Integrations));
        assert_eq!(Section::from_name("about"), Some(Section::About));
        assert_eq!(Section::from_name("x"), None);
        for s in Section::ALL {
            assert_eq!(Section::ALL[s.index()], s);
        }
    }
}
