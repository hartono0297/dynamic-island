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
use crate::cli_folders::CliStart;
use crate::clipboard::store::{ITEMS_STEP, MAX_ITEMS, MIN_ITEMS};
use crate::config::{
    ClipHotkey, ClipboardSettings, DISK_WARN_RANGE, RAM_WARN_RANGE, SHOT_SECS_RANGE,
    ScreenshotSettings, ShotIgnore, Style,
};
use crate::i18n::{Key, Lang, text, version_label};
use crate::model::{Page, PomoField};
use crate::pages::PanelPages;
use crate::pomodoro::{self, PomoSettings};
use crate::theme::{Accent, Surface, WindowTheme};
use crate::ui::icons::Icon;
use crate::ui::robot::{Motion, SLEEP_MAX, SLEEP_MIN};

pub use super::geometry::Rect;

/// Client area.
pub const CLIENT_W: f32 = 720.0;
pub const CLIENT_H: f32 = 520.0;
pub const SIDEBAR_W: f32 = 184.0;
const PAD: f32 = 24.0;
const CARD_GAP: f32 = 12.0;
const ROW_H: f32 = 52.0;

// Studio radius scale (docs/design/studio-ui.html): card 14, control 10
// (segmented track, sidebar item, text field), the segment inside a track 8,
// and chips, buttons, toggles and stepper buttons fully rounded. `FULL` is
// clamped to half the shape's height when drawn.
pub const CARD_RADIUS: f32 = 14.0;
pub const CONTROL_RADIUS: f32 = 10.0;
pub const SEGMENT_RADIUS: f32 = 8.0;
pub const FULL_RADIUS: f32 = 999.0;

/// Horizontal padding inside a card.
const INSET: f32 = 16.0;
const CONTENT_X: f32 = SIDEBAR_W + PAD;
const CONTENT_W: f32 = CLIENT_W - CONTENT_X - PAD;
const TITLE_Y: f32 = 20.0;
const TITLE_H: f32 = 32.0;
const FIRST_CARD_Y: f32 = TITLE_Y + TITLE_H + 16.0;
/// A section taller than the window scrolls under the title, from here down.
pub const VIEW_TOP: f32 = TITLE_Y + TITLE_H + 8.0;
/// One wheel notch scrolls one row.
const WHEEL_STEP: f32 = ROW_H;
/// The scrollbar thumb at the content's right edge.
const SCROLLBAR_W: f32 = 3.0;
/// Segmented controls and steppers.
pub const CONTROL_H: f32 = 28.0;
pub const BUTTON_H: f32 = 30.0;
/// Inner padding of a segmented control's track.
const SEG_PAD: f32 = 2.0;
const SEG_TEXT_PAD: f32 = 14.0;
const SEG_MIN_W: f32 = 64.0;
/// Segments share one width unless that makes the control wider than this.
const SEG_EQUAL_MAX: f32 = 280.0;
pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 22.0;
pub const SWATCH: f32 = 22.0;
const SWATCH_GAP: f32 = 10.0;
/// Swatches take clicks a little outside the circle; the selection ring
/// (a 2 px gap and a 1.5 px line) stays inside this margin.
const SWATCH_SLOP: f32 = 4.0;
pub const SWATCH_RING_GAP: f32 = 2.0;
pub const SWATCH_RING_W: f32 = 1.5;
pub const STEP_BUTTON: f32 = 28.0;
const STEP_VALUE_W: f32 = 64.0;
const BUTTON_PAD: f32 = 16.0;
const BUTTON_MIN_W: f32 = 80.0;
/// An icon-only button (the About links).
const ICON_BUTTON: f32 = 32.0;
pub const EXTERNAL_GLYPH: f32 = 16.0;
const LINK_PAD: f32 = 8.0;
/// Space taken by the glyph after a link's label.
pub const LINK_GLYPH_W: f32 = 18.0;
const GAP: f32 = 8.0;
/// A description that wraps: each further line adds this to its row.
const DESC_LINE_H: f32 = 14.0;
const LOGO: f32 = 28.0;
const NAV_X: f32 = 10.0;
const NAV_W: f32 = SIDEBAR_W - 2.0 * NAV_X;
const NAV_Y: f32 = 72.0;
const NAV_H: f32 = 36.0;
const NAV_GAP: f32 = 2.0;
const PREVIEW_H: f32 = 112.0;
const FIELD_ROW_H: f32 = 44.0;
const FIELD_H: f32 = 32.0;
/// The uppercase label above a card (Integrations).
const EYEBROW_H: f32 = 18.0;
const EYEBROW_GAP: f32 = 8.0;
/// A status chip: dot, gap, text, with this padding on each side.
pub const CHIP_H: f32 = 20.0;
const CHIP_PAD: f32 = 8.0;
pub const DOT: f32 = 6.0;
const DOT_GAP: f32 = 5.0;
const ABOUT_LOGO: f32 = 64.0;
const BATTERY_RANGE: (u8, u8, u8) = (5, 50, 5);

pub const REPO_URL: &str = "https://github.com/hartono0297/dynamic-island";
pub const RELEASES_URL: &str = "https://github.com/hartono0297/dynamic-island/releases";
pub const GUIDE_URL: &str = "https://github.com/hartono0297/dynamic-island#setup-google-calendar";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Appearance,
    General,
    Clipboard,
    Pomodoro,
    Integrations,
    About,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Appearance,
        Section::General,
        Section::Clipboard,
        Section::Pomodoro,
        Section::Integrations,
        Section::About,
    ];

    pub fn key(self) -> Key {
        match self {
            Section::Appearance => Key::SecAppearance,
            Section::General => Key::SecGeneral,
            Section::Clipboard => Key::SecClipboard,
            Section::Pomodoro => Key::SecPomodoro,
            Section::Integrations => Key::SecIntegrations,
            Section::About => Key::SecAbout,
        }
    }

    /// The section's icon in the sidebar.
    pub fn icon(self) -> Icon {
        match self {
            Section::Appearance => Icon::Palette,
            Section::General => Icon::Gear,
            Section::Clipboard => Icon::Clipboard,
            Section::Pomodoro => Icon::Timer,
            Section::Integrations => Icon::Link,
            Section::About => Icon::Info,
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
            "clipboard" => Some(Section::Clipboard),
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
    /// The uppercase label above a card ("GOOGLE CALENDAR").
    Eyebrow,
}

impl Font {
    pub const ALL: [Font; 10] = [
        Font::Brand,
        Font::Nav,
        Font::Title,
        Font::Label,
        Font::Desc,
        Font::Control,
        Font::Value,
        Font::AboutName,
        Font::AboutVersion,
        Font::Eyebrow,
    ];

    /// Studio scale: the display style (DM Sans 600 upright, tight
    /// tracking) at 20 for titles and the About name and 16 for the brand,
    /// DM Sans 12 for text and 10.5 for descriptions.
    pub fn size(self) -> f32 {
        match self {
            Font::Brand => 16.0,
            Font::Title | Font::AboutName => 20.0,
            Font::Nav | Font::Label | Font::Control | Font::Value | Font::AboutVersion => 12.0,
            Font::Desc | Font::Eyebrow => 10.5,
        }
    }

    /// The display style: DM Sans 600 upright with tight (negative)
    /// tracking. The renderer reads this to pick the tracking; the face and
    /// weight are the same DM Sans as the body text.
    pub fn display(self) -> bool {
        matches!(self, Font::Brand | Font::Title | Font::AboutName)
    }

    /// CSS-style weight: 400, 500 or 600 (the weights embedded).
    pub fn weight(self) -> u16 {
        match self {
            Font::Brand | Font::Title | Font::AboutName | Font::Eyebrow => 600,
            Font::Label => 500,
            Font::Nav | Font::Desc | Font::Control | Font::Value | Font::AboutVersion => 400,
        }
    }

    /// Numbers line up (tabular figures): the stepper values.
    pub fn tabular(self) -> bool {
        self == Font::Value
    }
}

pub trait Measure {
    /// Width of `text` on one line in `font`, in logical pixels.
    fn width(&self, text: &str, font: Font) -> f32;
}

/// A generous width estimate (wider than DM Sans or the Segoe UI Variable
/// fallback draw nearly any text, the display style included), so a layout
/// that fits with it also fits on screen.
pub struct Estimate;

impl Measure for Estimate {
    fn width(&self, text: &str, font: Font) -> f32 {
        let per_char = if font == Font::Eyebrow {
            // All capitals.
            0.8
        } else if font.weight() >= 500 {
            // Medium and semibold, the display style included. Measured
            // from DMSans-600.ttf with the display style's -2.5% tracking,
            // the titles and brand take 0.45 to 0.57 em per character, and
            // the Segoe UI Variable Display semibold fallback is about as
            // wide; 0.66 leaves over 15% for wide letters (m, w) and
            // rounding.
            0.66
        } else {
            0.62
        };
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
    /// "Folder awal CLI": Ikuti Explorer or Folder terakhir.
    CliStart(CliStart),
    Shell(Shell),
    /// "Asisten di notch".
    Robot,
    /// "Animasi": Penuh, Hemat, Mati.
    Motion(Motion),
    /// "Tidur setelah", one step down (−1) or up.
    RobotSleep(i32),
    /// "Sapaan pagi".
    Greeting,
    /// Settings → Clipboard: "Riwayat clipboard", "Jumlah item" (−1 / +1),
    /// "Simpan setelah restart", "Hapus saat layar dikunci", "Pintasan",
    /// "Disalin" for every copy, and "Hapus riwayat".
    ClipEnabled,
    ClipItems(i32),
    ClipPersist,
    ClipClearOnLock,
    ClipHotkey(ClipHotkey),
    ClipFeedbackAll,
    ClipClear,
    /// Settings → Clipboard → Screenshot: "Preview di notch", "Jika
    /// diabaikan" (Simpan / Hapus) and "Lama tampil" (−1 / +1).
    ShotPreview,
    ShotIgnore(ShotIgnore),
    ShotSecs(i32),
    Battery(i32),
    /// "Peringatan disk" / "Peringatan RAM", one step down (−1) or up.
    DiskWarn(i32),
    RamWarn(i32),
    /// "Lindungi cache Cargo & Autodesk di Mole".
    MoleProtect,
    /// "Halaman panel": a page's toggle, and its order stepper (−1 earlier,
    /// +1 later).
    PageOn(Page),
    PageMove(Page, i32),
    DndHelp,
    Pomo(PomoField, i32),
    GoogleSave,
    GoogleConnect,
    GoogleDisconnect,
    GoogleGuide,
    /// "Lanjutan: pakai Client ID sendiri", opening or closing the fields.
    GoogleAdvanced,
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
    CliStart,
    Shell,
    Motion,
    ClipHotkey,
    ShotIgnore,
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
    /// Like `Link`, but it opens a part of the page in place: a chevron
    /// (right while closed, down while open, the bool) instead of the glyph.
    Disclosure(bool),
    /// Only the "open" glyph; the label names it but is not drawn.
    External,
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
    /// Off (`enabled` false): faded, no clicks.
    Segmented {
        group: Group,
        track: Rect,
        items: Vec<Segment>,
        selected: usize,
        enabled: bool,
    },
    Toggle {
        hit: Hit,
        rect: Rect,
        on: bool,
        enabled: bool,
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
            Control::Segmented { enabled: false, .. } | Control::Toggle { enabled: false, .. } => {
                Vec::new()
            }
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

    /// Moves the control `dy` down (a scrolled section).
    fn shift(&mut self, dy: f32) {
        let down = |r: &mut Rect| r.y += dy;
        match self {
            Control::Segmented { track, items, .. } => {
                down(track);
                for s in items {
                    down(&mut s.rect);
                }
            }
            Control::Toggle { rect, .. } => down(rect),
            Control::Stepper {
                minus, plus, value, ..
            } => {
                down(&mut minus.0);
                down(&mut plus.0);
                down(&mut value.rect);
            }
            Control::Swatches { items, .. } => {
                for (_, r) in items {
                    down(r);
                }
            }
            Control::Button(b) => down(&mut b.rect),
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

/// A fully rounded status chip: a dot and a short text in the tone's colour
/// on a faint wash of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub rect: Rect,
    pub dot: Rect,
    pub label: Text,
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
    /// Status chips (Integrations).
    pub chips: Vec<Chip>,
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
    /// A section taller than the window: the area its content scrolls in
    /// (clipped to it, clicks only inside it), how far it can scroll, and
    /// the scroll applied.
    pub viewport: Option<Rect>,
    pub scroll_max: f32,
    pub scroll: f32,
}

impl Layout {
    /// Every clickable area: sidebar rows, then the section's controls (cut
    /// to the viewport of a scrolled section, so nothing hidden takes clicks).
    pub fn hits(&self) -> Vec<(Hit, Rect)> {
        let nav = self.nav.iter().map(|n| (Hit::Nav(n.section), n.pill));
        let view = self.viewport;
        let controls = self
            .controls
            .iter()
            .flat_map(Control::hits)
            .filter_map(move |(h, r)| match view {
                Some(v) => clip(r, v).map(|r| (h, r)),
                None => Some((h, r)),
            });
        nav.chain(controls).collect()
    }

    /// The scrollbar thumb of a scrolled section.
    pub fn scrollbar(&self) -> Option<Rect> {
        let v = self.viewport.filter(|_| self.scroll_max > 0.0)?;
        let content = v.h + self.scroll_max;
        let h = (v.h * v.h / content).max(24.0);
        let y = v.y + (v.h - h) * (self.scroll / self.scroll_max);
        Some(Rect {
            x: CLIENT_W - SCROLLBAR_W - 5.0,
            y: y + 4.0,
            w: SCROLLBAR_W,
            h: h - 8.0,
        })
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
                ..
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
    /// This build carries Vale's own Google client: one click signs in.
    pub built_in: bool,
    /// "Lanjutan" is open: the user's own Client ID and secret show.
    pub advanced: bool,
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
    /// Bata in the notch, how it moves, when it sleeps, the greeting.
    pub robot: bool,
    pub robot_motion: Motion,
    pub robot_sleep: u32,
    pub greeting: bool,
    /// How far a section taller than the window is scrolled (clamped).
    pub scroll: f32,
    /// Synced lyrics in the notch.
    pub lyrics: bool,
    /// The CLI page's shell.
    pub shell: Shell,
    /// Where the CLI page starts when it opens.
    pub cli_start: CliStart,
    pub autostart: bool,
    pub battery_threshold: u8,
    pub has_battery: bool,
    /// The System page's warnings: GB free and percent used.
    pub disk_warn_gb: u32,
    pub ram_warn_percent: u8,
    /// Mole's whitelist gets the Cargo and Autodesk caches before its
    /// first preview.
    pub mole_protect: bool,
    pub pomodoro: PomoSettings,
    /// A phase runs or is paused: the steppers are off.
    pub pomodoro_locked: bool,
    pub google: GoogleState,
    /// A short message beside the Google buttons.
    pub google_note: Option<(&'a str, Tone)>,
    /// "Hapus riwayat" was clicked once and waits for the second click.
    pub clip_confirm: bool,
    /// `None` when Claude Code's settings could not be read.
    pub hooks: Option<HookState>,
    /// The outcome of the last hook button, shown instead of the status.
    pub hook_note: Option<(&'a str, Tone)>,
    /// Settings → Clipboard, and whether the chosen hotkey could not be
    /// registered.
    pub clip: ClipboardSettings,
    pub clip_hotkey_failed: bool,
    /// Settings → Clipboard → Screenshot.
    pub shot: ScreenshotSettings,
    /// "Halaman panel": the pages' order and which the wheel shows.
    pub pages: PanelPages,
    pub version: &'a str,
    /// The data folder in use, as `data_dir::label` names it.
    pub data_dir: &'a str,
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

/// One step of `(min, max, step)` up (`dir > 0`) or down, snapped onto the
/// step's grid (from `min`) and kept in range.
fn grid_step(v: u32, dir: i32, (min, max, step): (u32, u32, u32)) -> u32 {
    let v = v.clamp(min, max);
    let on_grid = min + (v - min) / step * step;
    let stepped = if dir > 0 {
        on_grid + step
    } else if on_grid < v {
        on_grid
    } else {
        on_grid.saturating_sub(step)
    };
    stepped.clamp(min, max)
}

/// The disk warning one step up or down: 5–100 GB by 5.
pub fn disk_warn_step(gb: u32, dir: i32) -> u32 {
    grid_step(gb, dir, DISK_WARN_RANGE)
}

/// The RAM warning one step up or down: 70–98 %.
pub fn ram_warn_step(percent: u8, dir: i32) -> u8 {
    let (min, max, step) = RAM_WARN_RANGE;
    let range = (u32::from(min), u32::from(max), u32::from(step));
    grid_step(u32::from(percent), dir, range) as u8
}

/// "10 GB": the disk warning stepper's value.
pub fn disk_warn_value(gb: u32) -> String {
    format!("{gb} GB")
}

/// The scroll after `wheel` (raw delta, +120 per notch away from the user,
/// which scrolls up) from `scroll`, kept in 0..=`max`.
pub fn scroll_by(scroll: f32, wheel: i32, max: f32) -> f32 {
    (scroll - wheel as f32 / 120.0 * WHEEL_STEP).clamp(0.0, max.max(0.0))
}

/// The part of `r` inside `v`, if any.
fn clip(r: Rect, v: Rect) -> Option<Rect> {
    let x = r.x.max(v.x);
    let y = r.y.max(v.y);
    let right = r.right().min(v.right());
    let bottom = r.bottom().min(v.bottom());
    (right > x && bottom > y).then_some(Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    })
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

    /// A hairline across the whole card, between two rows.
    fn divider(&mut self, card: Rect, y: f32) {
        self.l.dividers.push(Rect {
            x: card.x + 1.0,
            y: y - 0.5,
            w: card.w - 2.0,
            h: 1.0,
        });
    }

    /// An uppercase label above the next card, in the space it takes.
    fn eyebrow(&mut self, key: Key, w: f32) {
        let rect = Rect {
            x: CONTENT_X + 2.0,
            y: self.y,
            w,
            h: EYEBROW_H,
        };
        let label = text(self.lang, key).to_uppercase();
        self.text(rect, label, Font::Eyebrow, Tone::Secondary, Align::Left);
        self.y += EYEBROW_H + EYEBROW_GAP;
    }

    /// A status chip starting at `x`, centred on `cy`, at most `max_w` wide.
    fn chip(&mut self, x: f32, cy: f32, label: String, tone: Tone, max_w: f32) {
        let text_w = self.w(&label, Font::Desc);
        let w = (2.0 * CHIP_PAD + DOT + DOT_GAP + text_w).min(max_w.max(0.0));
        let rect = Rect {
            x,
            y: cy - CHIP_H / 2.0,
            w,
            h: CHIP_H,
        };
        let dot = Rect {
            x: x + CHIP_PAD,
            y: cy - DOT / 2.0,
            w: DOT,
            h: DOT,
        };
        let text_x = dot.right() + DOT_GAP;
        let label = Text {
            rect: Rect {
                x: text_x,
                y: rect.y,
                w: (rect.right() - CHIP_PAD - text_x).max(0.0),
                h: CHIP_H,
            },
            text: label,
            font: Font::Desc,
            tone,
            align: Align::Left,
        };
        self.l.chips.push(Chip { rect, dot, label });
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

    /// A row's label and description, left of `control_x`. With `chip`, the
    /// description is a status chip in its tone. Returns the description's
    /// index in `texts` (none for a chip).
    fn row_text(
        &mut self,
        card: Rect,
        y: f32,
        label: String,
        desc: Option<(String, Tone)>,
        chip: bool,
        control_x: f32,
    ) -> Option<usize> {
        let x = card.x + INSET;
        let w = row_text_w(card, control_x);
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
        if chip {
            let label_rect = Rect {
                x,
                y: y + 5.0,
                w,
                h: 20.0,
            };
            self.text(label_rect, label, Font::Label, Tone::Primary, Align::Left);
            self.chip(x, y + 25.0 + CHIP_H / 2.0, desc, tone, w);
            return None;
        }
        let label_rect = Rect {
            x,
            y: y + 7.0,
            w,
            h: 20.0,
        };
        self.text(label_rect, label, Font::Label, Tone::Primary, Align::Left);
        let desc_rect = Rect {
            x,
            y: y + 28.0,
            w,
            h: 16.0,
        };
        Some(self.text(desc_rect, desc, Font::Desc, tone, Align::Left))
    }

    /// `row_text` with a description already broken into `lines` (`wrap`):
    /// each line one `DESC_LINE_H` under the last, in a row
    /// `wrapped_row_h(lines.len())` tall.
    fn row_text_lines(
        &mut self,
        card: Rect,
        y: f32,
        label: String,
        lines: Vec<String>,
        tone: Tone,
        control_x: f32,
    ) {
        let x = card.x + INSET;
        let w = row_text_w(card, control_x);
        let label_rect = Rect {
            x,
            y: y + 7.0,
            w,
            h: 20.0,
        };
        self.text(label_rect, label, Font::Label, Tone::Primary, Align::Left);
        for (i, line) in lines.into_iter().enumerate() {
            let rect = Rect {
                x,
                y: y + 28.0 + i as f32 * DESC_LINE_H,
                w,
                h: 16.0,
            };
            self.text(rect, line, Font::Desc, tone, Align::Left);
        }
    }

    /// `s` broken at spaces into lines no wider than `w` in `font`; a word
    /// wider than `w` keeps a line of its own.
    fn wrap(&self, s: &str, font: Font, w: f32) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for word in s.split_whitespace() {
            match lines.last_mut() {
                Some(line) if self.w(&format!("{line} {word}"), font) <= w => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push(word.to_owned()),
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    /// The width a segmented control of `items` takes (see `segmented_if`).
    fn segmented_w(&self, items: &[(Hit, String)]) -> f32 {
        self.segment_widths(items).iter().sum::<f32>() + 2.0 * SEG_PAD
    }

    /// Each segment's width: all the widest one's, unless that runs past
    /// `SEG_EQUAL_MAX`; then each its own.
    fn segment_widths(&self, items: &[(Hit, String)]) -> Vec<f32> {
        let natural: Vec<f32> = items
            .iter()
            .map(|(_, s)| (self.w(s, Font::Control) + 2.0 * SEG_TEXT_PAD).max(SEG_MIN_W))
            .collect();
        let widest = natural.iter().copied().fold(0.0, f32::max);
        let equal = widest * items.len() as f32 + 2.0 * SEG_PAD <= SEG_EQUAL_MAX;
        if equal {
            vec![widest; items.len()]
        } else {
            natural
        }
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
        self.segmented_if(group, right, cy, items, selected, true)
    }

    /// `segmented`, faded and taking no clicks unless `enabled`.
    fn segmented_if(
        &mut self,
        group: Group,
        right: f32,
        cy: f32,
        items: Vec<(Hit, String)>,
        selected: usize,
        enabled: bool,
    ) -> f32 {
        let widths = self.segment_widths(&items);
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
            enabled,
        });
        track.x
    }

    fn toggle(&mut self, hit: Hit, right: f32, cy: f32, on: bool) -> f32 {
        self.toggle_if(hit, right, cy, on, true)
    }

    fn toggle_if(&mut self, hit: Hit, right: f32, cy: f32, on: bool, enabled: bool) -> f32 {
        let rect = Rect {
            x: right - TOGGLE_W,
            y: cy - TOGGLE_H / 2.0,
            w: TOGGLE_W,
            h: TOGGLE_H,
        };
        self.l.controls.push(Control::Toggle {
            hit,
            rect,
            on,
            enabled,
        });
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
            ButtonKind::Link | ButtonKind::Disclosure(_) => widest + 2.0 * LINK_PAD + LINK_GLYPH_W,
            ButtonKind::External => ICON_BUTTON,
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
            let h = match kind {
                ButtonKind::Link | ButtonKind::Disclosure(_) => CONTROL_H,
                ButtonKind::External => ICON_BUTTON,
                _ => BUTTON_H,
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
            x: NAV_X + 8.0,
            y: 22.0,
            w: LOGO,
            h: LOGO,
        };
        let brand_x = self.l.logo.right() + 8.0;
        self.l.brand = Some(Text {
            // No fill or divider marks the sidebar's edge, so the brand may
            // run into the gutter, up to half a gap before the content
            // column. DM Sans 600 draws "Vale" at 16 px about
            // 40 px wide; the estimate is well inside what the sidebar has.
            rect: Rect {
                x: brand_x,
                y: 22.0,
                w: CONTENT_X - GAP / 2.0 - brand_x,
                h: LOGO,
            },
            text: "Vale".into(),
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
        Section::Clipboard => clipboard_page(&mut b, s),
        Section::Pomodoro => pomodoro_page(&mut b, s),
        Section::Integrations => integrations(&mut b, s),
        Section::About => about(&mut b, s),
    }
    scroll(&mut b.l, s.scroll);
    b.l
}

/// A section whose cards run past the window's bottom margin scrolls: its
/// content (not the sidebar or title) moves up by `scroll`, kept in range,
/// inside a viewport from `VIEW_TOP` to the window's bottom.
fn scroll(l: &mut Layout, scroll: f32) {
    let bottom = l.cards.iter().map(Rect::bottom).fold(0.0, f32::max);
    let max = (bottom - (CLIENT_H - PAD)).max(0.0);
    if max <= 0.0 {
        return;
    }
    let dy = -scroll.clamp(0.0, max);
    l.viewport = Some(Rect {
        x: SIDEBAR_W,
        y: VIEW_TOP,
        w: CLIENT_W - SIDEBAR_W,
        h: CLIENT_H - VIEW_TOP,
    });
    l.scroll_max = max;
    l.scroll = -dy;
    let down = |r: &mut Rect| r.y += dy;
    for r in l.cards.iter_mut().chain(l.dividers.iter_mut()) {
        down(r);
    }
    for t in &mut l.texts {
        down(&mut t.rect);
    }
    for c in &mut l.controls {
        c.shift(dy);
    }
    for c in &mut l.chips {
        down(&mut c.rect);
        down(&mut c.dot);
        down(&mut c.label.rect);
    }
    for (_, r) in &mut l.fields {
        down(r);
    }
    for r in [&mut l.preview, &mut l.about_logo, &mut l.note_glyph]
        .into_iter()
        .flatten()
    {
        down(r);
    }
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

    robot_card(b, s);
    pages_card(b, s);
}

fn page_key(p: Page) -> Key {
    match p {
        Page::Calendar => Key::PageCalendar,
        Page::Pomodoro => Key::PagePomodoro,
        Page::Reminders => Key::PageReminders,
        Page::Notes => Key::PageNotes,
        Page::Media => Key::PageMedia,
        Page::Clipboard => Key::PageClipboard,
        Page::System => Key::PageSystem,
        Page::Cli => Key::PageCli,
        Page::Shortcuts => Key::PageShortcuts,
    }
}

/// "Halaman panel": one row per page in the owner's order, its order stepper
/// (− earlier, + later; the position in between) and its toggle. The last
/// page still shown cannot be switched off.
fn pages_card(b: &mut Builder, s: &State) {
    b.eyebrow(Key::PanelPages, CONTENT_W - 4.0);
    let entries = s.pages.entries();
    let card = b.card(entries.len() as f32 * ROW_H);
    let shown = s.pages.visible().len();
    let last = entries.len() - 1;
    for (i, (page, on)) in entries.iter().enumerate() {
        let y = card.y + i as f32 * ROW_H;
        let cy = y + ROW_H / 2.0;
        let locked = *on && shown == 1;
        let x = b.toggle_if(Hit::PageOn(*page), RIGHT, cy, *on, !locked);
        let x = b.stepper(
            x - GAP,
            cy,
            (i + 1).to_string(),
            Hit::PageMove(*page, -1),
            Hit::PageMove(*page, 1),
            (i > 0, i < last),
            true,
        );
        let desc = (i == 0).then(|| (b.t(Key::PageOrderDesc), Tone::Secondary));
        b.row_text(card, y, b.t(page_key(*page)), desc, false, x);
        if i > 0 {
            b.divider(card, y);
        }
    }
}

/// The width left for a row's label and description, left of `control_x`.
fn row_text_w(card: Rect, control_x: f32) -> f32 {
    (control_x - 2.0 * GAP - (card.x + INSET)).max(0.0)
}

/// A row whose description takes `lines` lines.
fn wrapped_row_h(lines: usize) -> f32 {
    ROW_H + lines.saturating_sub(1) as f32 * DESC_LINE_H
}

/// Bata's four rows under Tampilan; with the robot off the other three
/// stay visible but faded and take no clicks. The Animasi description wraps
/// beside its three segments (the design's full sentence), and that row
/// grows by a line.
fn robot_card(b: &mut Builder, s: &State) {
    let modes = [
        (Motion::Full, Key::MotionFull),
        (Motion::Saver, Key::MotionSaver),
        (Motion::Off, Key::MotionOff),
    ];
    let items: Vec<(Hit, String)> = modes
        .iter()
        .map(|(v, k)| (Hit::Motion(*v), b.t(*k)))
        .collect();
    let probe = Rect {
        x: CONTENT_X,
        y: 0.0,
        w: CONTENT_W,
        h: 0.0,
    };
    let desc_w = row_text_w(probe, RIGHT - b.segmented_w(&items));
    let motion_desc = b.wrap(&b.t(Key::RobotMotionDesc), Font::Desc, desc_w);
    let motion_h = wrapped_row_h(motion_desc.len());

    let card = b.card(3.0 * ROW_H + motion_h);
    let heights = [ROW_H, motion_h, ROW_H, ROW_H];
    let mut rows = [card.y; 4];
    for i in 1..4 {
        rows[i] = rows[i - 1] + heights[i - 1];
    }
    let cy = |i: usize| rows[i] + heights[i] / 2.0;
    let on = s.robot;

    let x = b.toggle(Hit::Robot, RIGHT, cy(0), on);
    let desc = Some((b.t(Key::RobotInNotchDesc), Tone::Secondary));
    b.row_text(card, rows[0], b.t(Key::RobotInNotch), desc, false, x);

    let selected = index_of(Motion::ALL, &s.robot_motion);
    let x = b.segmented_if(Group::Motion, RIGHT, cy(1), items, selected, on);
    let label = b.t(Key::RobotMotion);
    b.row_text_lines(card, rows[1], label, motion_desc, Tone::Secondary, x);
    b.divider(card, rows[1]);

    let m = s.robot_sleep;
    let x = b.stepper(
        RIGHT,
        cy(2),
        sleep_value(b.lang, m),
        Hit::RobotSleep(-1),
        Hit::RobotSleep(1),
        (m > SLEEP_MIN, m < SLEEP_MAX),
        on && s.robot_motion != Motion::Off,
    );
    let desc = Some((b.t(Key::RobotSleepDesc), Tone::Secondary));
    b.row_text(card, rows[2], b.t(Key::RobotSleep), desc, false, x);
    b.divider(card, rows[2]);

    let x = b.toggle_if(Hit::Greeting, RIGHT, cy(3), s.greeting, on);
    let desc = Some((b.t(Key::MorningGreetingDesc), Tone::Secondary));
    b.row_text(card, rows[3], b.t(Key::MorningGreeting), desc, false, x);
    b.divider(card, rows[3]);
}

/// "10 mnt", "10 min": the sleep stepper's value.
pub fn sleep_value(lang: Lang, minutes: u32) -> String {
    format!("{minutes} {}", text(lang, Key::MinutesShort))
}

fn general(b: &mut Builder, s: &State) {
    let card = b.card(7.0 * ROW_H);
    let rows: [f32; 7] = std::array::from_fn(|i| card.y + i as f32 * ROW_H);
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

    // Just before the shell, as in keputusan-v3.1.html.
    let items = vec![
        (
            Hit::CliStart(CliStart::Explorer),
            b.t(Key::CliStartExplorer),
        ),
        (Hit::CliStart(CliStart::Last), b.t(Key::CliStartLast)),
    ];
    let selected = index_of(&CliStart::ALL, &s.cli_start);
    let x = b.segmented(Group::CliStart, RIGHT, cy(rows[5]), items, selected);
    let desc = Some((b.t(Key::CliStartFolderDesc), Tone::Secondary));
    b.row_text(card, rows[5], b.t(Key::CliStartFolder), desc, false, x);
    b.divider(card, rows[5]);

    let items = vec![
        (Hit::Shell(Shell::PowerShell), b.t(Key::ShellPowerShell)),
        (Hit::Shell(Shell::Cmd), b.t(Key::ShellCmd)),
    ];
    let selected = index_of(&Shell::ALL, &s.shell);
    let x = b.segmented(Group::Shell, RIGHT, cy(rows[6]), items, selected);
    let desc = Some((b.t(Key::ShellCliDesc), Tone::Secondary));
    b.row_text(card, rows[6], b.t(Key::ShellCli), desc, false, x);
    b.divider(card, rows[6]);

    warnings_card(b, s);
}

/// The System page's warnings (keputusan-v3.1.html p4) and the cleaner's
/// Mole protection, in a second card under Umum's seven rows; the section
/// scrolls to reach it, like Tampilan.
fn warnings_card(b: &mut Builder, s: &State) {
    let card = b.card(3.0 * ROW_H);
    let rows = [card.y, card.y + ROW_H, card.y + 2.0 * ROW_H];
    let cy = |y: f32| y + ROW_H / 2.0;

    let (min, max, _) = DISK_WARN_RANGE;
    let gb = s.disk_warn_gb;
    let x = b.stepper(
        RIGHT,
        cy(rows[0]),
        disk_warn_value(gb),
        Hit::DiskWarn(-1),
        Hit::DiskWarn(1),
        (gb > min, gb < max),
        true,
    );
    let desc = Some((b.t(Key::DiskAlertDesc), Tone::Secondary));
    b.row_text(card, rows[0], b.t(Key::DiskAlert), desc, false, x);

    let (min, max, _) = RAM_WARN_RANGE;
    let pct = s.ram_warn_percent;
    let x = b.stepper(
        RIGHT,
        cy(rows[1]),
        format!("{pct}%"),
        Hit::RamWarn(-1),
        Hit::RamWarn(1),
        (pct > min, pct < max),
        true,
    );
    let desc = Some((b.t(Key::RamAlertDesc), Tone::Secondary));
    b.row_text(card, rows[1], b.t(Key::RamAlert), desc, false, x);
    b.divider(card, rows[1]);

    let x = b.toggle(Hit::MoleProtect, RIGHT, cy(rows[2]), s.mole_protect);
    let desc = Some((b.t(Key::MoleProtectSettingDesc), Tone::Secondary));
    b.row_text(card, rows[2], b.t(Key::MoleProtectSetting), desc, false, x);
    b.divider(card, rows[2]);
}

/// "Jumlah item" one step (10) up or down, kept in 10–200; a hand-edited
/// value off the grid snaps onto it.
pub fn clip_items_step(n: u32, dir: i32) -> u32 {
    let stepped = if dir > 0 {
        n.saturating_add(ITEMS_STEP)
    } else {
        n.saturating_sub(ITEMS_STEP)
    };
    (stepped / ITEMS_STEP * ITEMS_STEP).clamp(MIN_ITEMS, MAX_ITEMS)
}

/// Settings → Clipboard after keputusan-v3.1.html §5 (`settingsWin`), with
/// the plan's hotkey choice (none, Ctrl+Alt+V, Win+Alt+V; never
/// Ctrl+Shift+V) and a clear button. With the history off the other rows
/// fade and take no clicks.
fn clipboard_page(b: &mut Builder, s: &State) {
    let c = s.clip;
    let card = b.card(7.0 * ROW_H);
    let rows: [f32; 7] = std::array::from_fn(|i| card.y + i as f32 * ROW_H);
    let cy = |y: f32| y + ROW_H / 2.0;
    let on = c.enabled;

    let x = b.toggle(Hit::ClipEnabled, RIGHT, cy(rows[0]), on);
    let desc = Some((b.t(Key::ClipHistoryDesc), Tone::Secondary));
    b.row_text(card, rows[0], b.t(Key::ClipHistory), desc, false, x);

    let n = c.max_items;
    let x = b.stepper(
        RIGHT,
        cy(rows[1]),
        n.to_string(),
        Hit::ClipItems(-1),
        Hit::ClipItems(1),
        (n > MIN_ITEMS, n < MAX_ITEMS),
        on,
    );
    let desc = Some((b.t(Key::ClipItemsDesc), Tone::Secondary));
    b.row_text(card, rows[1], b.t(Key::ClipItems), desc, false, x);
    b.divider(card, rows[1]);

    let x = b.toggle_if(Hit::ClipPersist, RIGHT, cy(rows[2]), c.persist, on);
    let desc = Some((b.t(Key::ClipPersistDesc), Tone::Secondary));
    b.row_text(card, rows[2], b.t(Key::ClipPersist), desc, false, x);
    b.divider(card, rows[2]);

    let x = b.toggle_if(
        Hit::ClipClearOnLock,
        RIGHT,
        cy(rows[3]),
        c.clear_on_lock,
        on,
    );
    let desc = Some((b.t(Key::ClipClearOnLockDesc), Tone::Secondary));
    b.row_text(card, rows[3], b.t(Key::ClipClearOnLock), desc, false, x);
    b.divider(card, rows[3]);

    let items = vec![
        (Hit::ClipHotkey(ClipHotkey::Off), b.t(Key::ClipHotkeyNone)),
        (
            Hit::ClipHotkey(ClipHotkey::CtrlAltV),
            "Ctrl+Alt+V".to_owned(),
        ),
        (Hit::ClipHotkey(ClipHotkey::WinAltV), "Win+Alt+V".to_owned()),
    ];
    let selected = index_of(ClipHotkey::ALL, &c.hotkey);
    let x = b.segmented(Group::ClipHotkey, RIGHT, cy(rows[4]), items, selected);
    let desc = if s.clip_hotkey_failed && c.hotkey != ClipHotkey::Off {
        (b.t(Key::ClipHotkeyTaken), Tone::Bad)
    } else {
        (b.t(Key::ClipHotkeyDesc), Tone::Secondary)
    };
    b.row_text(card, rows[4], b.t(Key::ClipHotkey), Some(desc), false, x);
    b.divider(card, rows[4]);

    let x = b.toggle_if(
        Hit::ClipFeedbackAll,
        RIGHT,
        cy(rows[5]),
        c.copied_feedback_all,
        on,
    );
    let desc = Some((b.t(Key::ClipFeedbackAllDesc), Tone::Secondary));
    b.row_text(card, rows[5], b.t(Key::ClipFeedbackAll), desc, false, x);
    b.divider(card, rows[5]);

    // Two clicks, as "Putuskan": the first arms it for a few seconds.
    let (label, sure) = (b.t(Key::ClipClearButton), b.t(Key::ConfirmShort));
    let w = b.button_w(&[&label, &sure], ButtonKind::Secondary);
    let item = if s.clip_confirm {
        (Hit::ClipClear, sure, ButtonKind::Confirm, true, w)
    } else {
        (Hit::ClipClear, label, ButtonKind::Secondary, true, w)
    };
    let x = b.buttons(RIGHT, cy(rows[6]), vec![item]);
    let desc = Some((b.t(Key::ClipClearDesc), Tone::Secondary));
    b.row_text(card, rows[6], b.t(Key::ClipClear), desc, false, x);
    b.divider(card, rows[6]);

    screenshot_card(b, s);
}

/// "Lama tampil" one step (2 s) up or down, kept in `SHOT_SECS_RANGE`; a
/// hand-edited value off the grid snaps onto it.
pub fn shot_secs_step(n: u32, dir: i32) -> u32 {
    let (min, max, step) = SHOT_SECS_RANGE;
    let stepped = if dir > 0 {
        n.saturating_add(step)
    } else {
        n.saturating_sub(step)
    };
    (stepped / step * step).clamp(min, max)
}

/// The screenshot preview (keputusan-v3.1.html p6, its `settingsWin`
/// "Screenshot") in a second card under the history's seven rows, named by
/// an eyebrow; the section scrolls to reach it. With the preview off the
/// other two rows fade and take no clicks. Simpan is the default.
fn screenshot_card(b: &mut Builder, s: &State) {
    let c = s.shot;
    b.eyebrow(Key::SecScreenshot, CONTENT_W);
    let card = b.card(3.0 * ROW_H);
    let rows = [card.y, card.y + ROW_H, card.y + 2.0 * ROW_H];
    let cy = |y: f32| y + ROW_H / 2.0;
    let on = c.preview;

    let x = b.toggle(Hit::ShotPreview, RIGHT, cy(rows[0]), on);
    b.row_text(card, rows[0], b.t(Key::ShotPreviewSetting), None, false, x);

    let items = vec![
        (Hit::ShotIgnore(ShotIgnore::Keep), b.t(Key::ShotKeep)),
        (Hit::ShotIgnore(ShotIgnore::Delete), b.t(Key::ShotDelete)),
    ];
    let selected = index_of(ShotIgnore::ALL, &c.on_ignore);
    let x = b.segmented_if(Group::ShotIgnore, RIGHT, cy(rows[1]), items, selected, on);
    let desc = Some((b.t(Key::ShotIgnoreDesc), Tone::Secondary));
    b.row_text(card, rows[1], b.t(Key::ShotIgnoreSetting), desc, false, x);
    b.divider(card, rows[1]);

    let (min, max, _) = SHOT_SECS_RANGE;
    let n = c.show_secs;
    let x = b.stepper(
        RIGHT,
        cy(rows[2]),
        crate::i18n::seconds_short(b.lang, n),
        Hit::ShotSecs(-1),
        Hit::ShotSecs(1),
        (n > min, n < max),
        on,
    );
    b.row_text(card, rows[2], b.t(Key::ShotShowFor), None, false, x);
    b.divider(card, rows[2]);
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
    // Above the card: its name, and the setup guide on the right.
    let guide = b.t(Key::SetupGuide);
    let w = b.button_w(&[&guide], ButtonKind::Link);
    let item = (Hit::GoogleGuide, guide, ButtonKind::Link, true, w);
    let x = b.buttons(
        CONTENT_X + CONTENT_W + LINK_PAD,
        b.y + EYEBROW_H / 2.0,
        vec![item],
    );
    b.eyebrow(Key::GoogleCalendar, x - GAP - CONTENT_X - 2.0);
    // With Vale's own client the card is one button; the user's own Client
    // ID and secret wait under "Lanjutan".
    // An own ID in the field keeps them open: it is the one a login uses.
    let fields = !g.built_in || g.advanced || g.has_id;
    let card = b.card(if fields {
        6.0 + 2.0 * FIELD_ROW_H + ROW_H
    } else {
        ROW_H
    });

    // Client ID and secret: label column, then the field.
    let top = card.y + 6.0;
    if fields {
        let labels = [b.t(Key::ClientId), b.t(Key::ClientSecret)];
        let label_w = labels
            .iter()
            .map(|l| b.w(l, Font::Label))
            .fold(0.0, f32::max);
        let field_x = card.x + INSET + label_w + 2.0 * GAP;
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
    }
    let buttons_y = if fields {
        top + 2.0 * FIELD_ROW_H
    } else {
        card.y
    };

    // Buttons, with the status chip (or a short message) on their left.
    let (save, connect) = (b.t(Key::Save), b.t(Key::Connect));
    let sign_in = b.t(Key::SignInGoogle);
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
    // A client to log in with (Vale's, or a typed Client ID), and not
    // connected already.
    let has_client = g.built_in || (g.has_id && (g.status != CalStatus::NotConfigured || g.dirty));
    let can_connect = has_client && g.status != CalStatus::Ok;
    let mut items = Vec::new();
    if fields {
        items.push((
            Hit::GoogleSave,
            save.clone(),
            kind(g.dirty),
            g.dirty,
            b.button_w(&[&save], ButtonKind::Secondary),
        ));
    }
    let connect = if fields { connect } else { sign_in };
    items.push((
        Hit::GoogleConnect,
        connect.clone(),
        kind(!g.dirty && can_connect && needs_login),
        can_connect,
        b.button_w(&[&connect], ButtonKind::Secondary),
    ));
    items.push((
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
    ));
    let row_cy = buttons_y + ROW_H / 2.0;
    let x = b.buttons(RIGHT, row_cy, items);
    let left_w = (x - 2.0 * GAP - card.x - INSET).max(0.0);
    match s.google_note {
        // A message after a click stands in for the status for a few seconds.
        Some((note, tone)) => {
            let rect = Rect {
                x: card.x + INSET,
                y: row_cy - 9.0,
                w: left_w,
                h: 18.0,
            };
            b.text(rect, note.to_owned(), Font::Desc, tone, Align::Left);
        }
        None => {
            let (key, tone) = status_key(g.status);
            b.chip(card.x + INSET, row_cy, b.t(key), tone, left_w);
        }
    }
    if g.built_in && !g.has_id {
        // Under the card, on the left: the way to the user's own client.
        let advanced = b.t(Key::GoogleAdvanced);
        let kind = ButtonKind::Disclosure(g.advanced);
        let w = b.button_w(&[&advanced], kind);
        let top = card.y + card.h + 4.0;
        let item = (Hit::GoogleAdvanced, advanced, kind, true, w);
        b.buttons(CONTENT_X + w, top + CONTROL_H / 2.0, vec![item]);
        b.y = top + CONTROL_H + 4.0;
    }

    // Claude Code hooks.
    b.y += EYEBROW_GAP;
    b.eyebrow(Key::ClaudeCode, CONTENT_W - 2.0);
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
    // The logo with the name, version and tagline beside it.
    let top = FIRST_CARD_Y + 2.0;
    b.l.about_logo = Some(Rect {
        x: CONTENT_X + 2.0,
        y: top,
        w: ABOUT_LOGO,
        h: ABOUT_LOGO,
    });
    let text_x = CONTENT_X + 2.0 + ABOUT_LOGO + 16.0;
    let line = |y, h| Rect {
        x: text_x,
        y,
        w: CONTENT_X + CONTENT_W - text_x,
        h,
    };
    b.text(
        line(top + 2.0, 26.0),
        "Vale".into(),
        Font::AboutName,
        Tone::Primary,
        Align::Left,
    );
    let version = version_label(b.lang, s.version);
    b.text(
        line(top + 28.0, 18.0),
        version,
        Font::AboutVersion,
        Tone::Secondary,
        Align::Left,
    );
    let tagline = b.t(Key::AboutTagline);
    b.text(
        line(top + 46.0, 18.0),
        tagline,
        Font::AboutVersion,
        Tone::Secondary,
        Align::Left,
    );
    b.y = top + ABOUT_LOGO + 18.0;

    let card = b.card(3.0 * ROW_H);
    // Web pages open from a bare "open" glyph; the data folder has a button.
    let rows = [
        (
            Hit::Repo,
            Key::SourceCode,
            "github.com/hartono0297/dynamic-island".to_owned(),
            Key::OpenGithub,
            ButtonKind::External,
        ),
        (
            Hit::Releases,
            Key::Updates,
            b.t(Key::UpdatesDesc),
            Key::SeeReleases,
            ButtonKind::External,
        ),
        (
            Hit::DataFolder,
            Key::DataFolder,
            s.data_dir.to_owned(),
            Key::OpenDataFolder,
            ButtonKind::Secondary,
        ),
    ];
    for (i, (hit, label, desc, button, kind)) in rows.into_iter().enumerate() {
        let y = card.y + i as f32 * ROW_H;
        let button = b.t(button);
        let w = b.button_w(&[&button], kind);
        let item = (hit, button, kind, true, w);
        // The glyph, not its larger click area, ends at the content edge.
        let right = if kind == ButtonKind::External {
            RIGHT + (ICON_BUTTON - EXTERNAL_GLYPH) / 2.0
        } else {
            RIGHT
        };
        let x = b.buttons(right, y + ROW_H / 2.0, vec![item]);
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
            for (status, dirty, has_id, confirm, built_in, advanced) in [
                (CalStatus::NotConfigured, false, false, false, false, false),
                (CalStatus::NotConfigured, true, true, false, false, false),
                (CalStatus::NeedsLogin, false, true, false, false, false),
                (CalStatus::Reconnect, true, true, false, false, false),
                (CalStatus::Ok, false, true, false, false, false),
                (CalStatus::Ok, false, true, true, false, false),
                (CalStatus::Offline, false, true, false, false, false),
                // A build with Vale's own Google client.
                (CalStatus::NeedsLogin, false, false, false, true, false),
                (CalStatus::Ok, false, false, true, true, false),
                (CalStatus::Reconnect, true, true, false, true, true),
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
                        robot: !locked,
                        robot_motion: if locked { Motion::Off } else { Motion::Saver },
                        robot_sleep: if locked { 60 } else { 1 },
                        greeting: locked,
                        scroll: 0.0,
                        lyrics: !locked,
                        shell: if locked {
                            Shell::Cmd
                        } else {
                            Shell::PowerShell
                        },
                        cli_start: if locked {
                            CliStart::Last
                        } else {
                            CliStart::Explorer
                        },
                        autostart: true,
                        battery_threshold: 50,
                        has_battery: battery,
                        disk_warn_gb: if locked { 100 } else { 5 },
                        ram_warn_percent: if locked { 98 } else { 70 },
                        mole_protect: !locked,
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
                            built_in,
                            advanced,
                        },
                        google_note: None,
                        clip_confirm: !locked,
                        hooks,
                        hook_note: None,
                        clip: ClipboardSettings {
                            enabled: !locked,
                            max_items: if locked { MAX_ITEMS } else { MIN_ITEMS },
                            hotkey: if locked {
                                ClipHotkey::WinAltV
                            } else {
                                ClipHotkey::CtrlAltV
                            },
                            ..ClipboardSettings::default()
                        },
                        clip_hotkey_failed: !battery,
                        shot: ScreenshotSettings {
                            preview: !locked,
                            on_ignore: if locked {
                                ShotIgnore::Delete
                            } else {
                                ShotIgnore::Keep
                            },
                            show_secs: if locked { 30 } else { 4 },
                        },
                        pages: if locked {
                            let mut p = PanelPages::default();
                            p.toggle(Page::Cli);
                            p.move_by(Page::Shortcuts, -8);
                            p
                        } else {
                            PanelPages::default()
                        },
                        version: "2.1.0",
                        data_dir: r"%APPDATA%\Vale",
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

    /// Every section and state, each scrolled to the top and to the bottom.
    fn all_layouts() -> Vec<(Section, Lang, Layout)> {
        let mut out = Vec::new();
        for s in states() {
            for section in Section::ALL {
                for scroll in [0.0, f32::MAX] {
                    let s = State {
                        scroll,
                        ..s.clone()
                    };
                    out.push((section, s.lang, layout(section, &s, &Estimate)));
                }
            }
        }
        out
    }

    /// The window, or for a scrolled section its whole content as if the
    /// window were tall enough (at this scroll).
    fn canvas(l: &Layout) -> Rect {
        Rect {
            y: -l.scroll,
            h: CLIENT_H + l.scroll_max,
            ..CLIENT
        }
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
        let content = Rect {
            x: CONTENT_X,
            y: 0.0,
            w: CONTENT_W + LINK_PAD,
            h: CLIENT_H,
        };
        for (section, lang, l) in all_layouts() {
            for c in &l.controls {
                let bounds = c.bounds();
                // The setup guide link sits on the line above its card.
                if let Control::Button(b) = c
                    && matches!(b.kind, ButtonKind::Link | ButtonKind::Disclosure(_))
                {
                    assert!(inside(bounds, content), "{section:?} {lang:?}: {c:?}");
                    assert!(l.cards.iter().all(|card| !overlaps(bounds, *card)));
                    continue;
                }
                assert!(
                    l.cards.iter().any(|card| inside(bounds, *card)),
                    "{section:?} {lang:?}: {c:?} outside every card"
                );
            }
            for chip in &l.chips {
                assert!(
                    l.cards.iter().any(|card| inside(chip.rect, *card)),
                    "{section:?} {lang:?}: {chip:?}"
                );
                assert!(inside(chip.dot, chip.rect) && inside(chip.label.rect, chip.rect));
            }
            for (f, r) in &l.fields {
                assert!(
                    l.cards.iter().any(|card| inside(*r, *card)),
                    "{section:?} {lang:?}: {f:?}"
                );
            }
            for card in &l.cards {
                assert!(inside(*card, canvas(&l)), "{section:?}: card {card:?}");
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
            boxes.extend(l.chips.iter().map(|c| (format!("{c:?}"), c.rect)));
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
            texts.extend(l.chips.iter().map(|c| &c.label));
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
                assert!(inside(t.rect, canvas(&l)), "{:?}", t.text);
            }
            for c in &l.controls {
                let labels: Vec<(&str, Rect, f32)> = match c {
                    Control::Segmented { items, .. } => items
                        .iter()
                        .map(|s| (s.label.as_str(), s.rect, 0.0))
                        .collect(),
                    Control::Button(b)
                        if matches!(b.kind, ButtonKind::Link | ButtonKind::Disclosure(_)) =>
                    {
                        vec![(b.label.as_str(), b.rect, LINK_GLYPH_W)]
                    }
                    // The label is not drawn.
                    Control::Button(b) if b.kind == ButtonKind::External => Vec::new(),
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

    /// Every section fits the window's height; Appearance (with Bata's
    /// rows), General (with the System warnings) and Clipboard (with the
    /// screenshot preview) scroll by exactly what they lack and then fit.
    #[test]
    fn everything_fits_in_the_window_height() {
        for (section, lang, l) in all_layouts() {
            let bottom = l
                .cards
                .iter()
                .map(Rect::bottom)
                .chain(l.texts.iter().map(|t| t.rect.bottom()))
                .fold(0.0, f32::max);
            let unscrolled = bottom + l.scroll;
            assert!(
                unscrolled - l.scroll_max <= CLIENT_H - PAD + 0.01,
                "{section:?} {lang:?}: {bottom}"
            );
            if matches!(
                section,
                Section::Appearance | Section::General | Section::Clipboard
            ) {
                assert!(l.scroll_max > 0.0 && l.viewport.is_some());
                // Scrolled to the end, the last card ends at the margin.
                if l.scroll == l.scroll_max {
                    assert!((bottom - (CLIENT_H - PAD)).abs() < 0.01, "{bottom}");
                }
            } else {
                assert_eq!((l.scroll_max, l.viewport), (0.0, None), "{section:?}");
            }
        }
    }

    #[test]
    fn appearance_scrolls_and_clicks_follow() {
        let base = State {
            robot: true,
            robot_motion: Motion::Full,
            robot_sleep: 10,
            greeting: true,
            ..states()[0].clone()
        };
        let robot_hits = |l: &Layout| -> Vec<Hit> {
            l.hits()
                .into_iter()
                .map(|(h, _)| h)
                .filter(|h| {
                    matches!(
                        h,
                        Hit::Robot | Hit::Motion(_) | Hit::RobotSleep(_) | Hit::Greeting
                    )
                })
                .collect()
        };
        let top = layout(Section::Appearance, &base, &Estimate);
        let view = top.viewport.expect("viewport");
        assert_eq!(top.scroll, 0.0);
        // At the top only the robot toggle peeks in; nothing below the
        // window takes a click.
        assert_eq!(robot_hits(&top), [Hit::Robot]);
        for (_, r) in top.hits() {
            assert!(r.bottom() <= CLIENT_H + 0.01, "{r:?}");
        }
        // Scrolled to the robot card: all four rows, where they are drawn.
        let end = State {
            scroll: robot_card_scroll(&base),
            ..base.clone()
        };
        let l = layout(Section::Appearance, &end, &Estimate);
        assert!(l.scroll > 0.0 && l.scroll <= l.scroll_max);
        assert_eq!(
            robot_hits(&l).len(),
            7,
            "2 toggles, 3 segments, minus, plus"
        );
        let toggle = l
            .controls
            .iter()
            .find_map(|c| match c {
                Control::Toggle {
                    hit: Hit::Greeting,
                    rect,
                    ..
                } => Some(*rect),
                _ => None,
            })
            .expect("greeting toggle");
        assert_eq!(l.hit(toggle.x + 4.0, toggle.y + 4.0), Some(Hit::Greeting));
        // The style row went up under the title: clipped, so no clicks.
        let style_hits: Vec<Rect> = l
            .hits()
            .into_iter()
            .filter(|(h, _)| matches!(h, Hit::Style(_)))
            .map(|(_, r)| r)
            .collect();
        assert!(style_hits.iter().all(|r| r.y >= view.y));
        // The wheel: a notch is a row, kept in range.
        assert_eq!(scroll_by(0.0, -120, 100.0), ROW_H);
        assert_eq!(scroll_by(0.0, 120, 100.0), 0.0);
        assert_eq!(scroll_by(90.0, -240, 100.0), 100.0);
        assert!(l.scrollbar().is_some() && top.scrollbar().is_some());
        let pomodoro = layout(Section::Pomodoro, &base, &Estimate);
        assert!(pomodoro.scrollbar().is_none());
    }

    #[test]
    fn general_scrolls_to_the_system_warnings() {
        let warn_hits = |l: &Layout| -> Vec<Hit> {
            l.hits()
                .into_iter()
                .map(|(h, _)| h)
                .filter(|h| matches!(h, Hit::DiskWarn(_) | Hit::RamWarn(_)))
                .collect()
        };
        for lang in [Lang::Id, Lang::En] {
            let base = State {
                lang,
                disk_warn_gb: 10,
                ram_warn_percent: 90,
                ..states()[0].clone()
            };
            let top = layout(Section::General, &base, &Estimate);
            assert!(top.viewport.is_some() && top.scrollbar().is_some());
            assert_eq!(top.cards.len(), 2, "the warnings have their own card");
            // At the top the warning card runs past the window: what is
            // below it takes no click.
            assert!(top.cards[1].bottom() > CLIENT_H);
            for (_, r) in top.hits() {
                assert!(r.bottom() <= CLIENT_H + 0.01, "{r:?}");
            }
            // Scrolled to the end: both steppers, the last card on the margin.
            let end = State {
                scroll: 1e6,
                ..base.clone()
            };
            let l = layout(Section::General, &end, &Estimate);
            assert_eq!(l.scroll, l.scroll_max);
            assert!((l.cards[1].bottom() - (CLIENT_H - PAD)).abs() < 0.01);
            assert_eq!(
                warn_hits(&l),
                [
                    Hit::DiskWarn(-1),
                    Hit::DiskWarn(1),
                    Hit::RamWarn(-1),
                    Hit::RamWarn(1)
                ]
            );
            let values: Vec<&str> = l
                .controls
                .iter()
                .filter_map(|c| match c {
                    Control::Stepper { minus, value, .. }
                        if matches!(minus.1, Some(Hit::DiskWarn(_) | Hit::RamWarn(_))) =>
                    {
                        Some(value.text.as_str())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(values, ["10 GB", "90%"]);
            // The Mole protection's toggle is the card's last row.
            let mole: Vec<Hit> = l
                .hits()
                .into_iter()
                .map(|(h, _)| h)
                .filter(|h| *h == Hit::MoleProtect)
                .collect();
            assert_eq!(mole, [Hit::MoleProtect]);
            for key in [
                Key::DiskAlert,
                Key::DiskAlertDesc,
                Key::RamAlert,
                Key::RamAlertDesc,
                Key::MoleProtectSetting,
                Key::MoleProtectSettingDesc,
            ] {
                assert!(
                    l.texts.iter().any(|t| t.text == text(lang, key)),
                    "{lang:?} {key:?}"
                );
            }
            // At the ends of their ranges a side goes.
            let ends = State {
                disk_warn_gb: 100,
                ram_warn_percent: 70,
                ..end
            };
            let l = layout(Section::General, &ends, &Estimate);
            assert_eq!(warn_hits(&l), [Hit::DiskWarn(-1), Hit::RamWarn(1)]);
        }
    }

    #[test]
    fn clipboard_scrolls_to_the_screenshot_card() {
        let shot_hits = |l: &Layout| -> Vec<Hit> {
            l.hits()
                .into_iter()
                .map(|(h, _)| h)
                .filter(|h| matches!(h, Hit::ShotPreview | Hit::ShotIgnore(_) | Hit::ShotSecs(_)))
                .collect()
        };
        for lang in [Lang::Id, Lang::En] {
            let base = State {
                lang,
                shot: ScreenshotSettings::default(),
                ..states()[0].clone()
            };
            let top = layout(Section::Clipboard, &base, &Estimate);
            assert!(top.viewport.is_some() && top.scrollbar().is_some());
            assert_eq!(top.cards.len(), 2, "the screenshot has its own card");
            for (_, r) in top.hits() {
                assert!(r.bottom() <= CLIENT_H + 0.01, "{r:?}");
            }
            let end = State {
                scroll: 1e6,
                ..base.clone()
            };
            let l = layout(Section::Clipboard, &end, &Estimate);
            assert_eq!(l.scroll, l.scroll_max);
            assert!((l.cards[1].bottom() - (CLIENT_H - PAD)).abs() < 0.01);
            assert_eq!(
                shot_hits(&l),
                [
                    Hit::ShotPreview,
                    Hit::ShotIgnore(ShotIgnore::Keep),
                    Hit::ShotIgnore(ShotIgnore::Delete),
                    Hit::ShotSecs(-1),
                    Hit::ShotSecs(1),
                ]
            );
            // The defaults: Simpan chosen, 8 s.
            let chosen = l.controls.iter().find_map(|c| match c {
                Control::Segmented {
                    group, selected, ..
                } if *group == Group::ShotIgnore => Some(*selected),
                _ => None,
            });
            assert_eq!(chosen, Some(0), "Simpan");
            let secs = l.controls.iter().find_map(|c| match c {
                Control::Stepper { minus, value, .. }
                    if matches!(minus.1, Some(Hit::ShotSecs(_))) =>
                {
                    Some(value.text.clone())
                }
                _ => None,
            });
            assert_eq!(secs, Some(crate::i18n::seconds_short(lang, 8)));
            for key in [
                Key::ShotPreviewSetting,
                Key::ShotIgnoreSetting,
                Key::ShotIgnoreDesc,
                Key::ShotShowFor,
            ] {
                assert!(
                    l.texts.iter().any(|t| t.text == text(lang, key)),
                    "{lang:?} {key:?}"
                );
            }
            assert!(
                l.texts
                    .iter()
                    .any(|t| t.text == text(lang, Key::SecScreenshot).to_uppercase())
            );
            // The preview off: only its toggle takes clicks.
            let off = State {
                shot: ScreenshotSettings {
                    preview: false,
                    ..ScreenshotSettings::default()
                },
                ..end
            };
            let l = layout(Section::Clipboard, &off, &Estimate);
            assert_eq!(shot_hits(&l), [Hit::ShotPreview]);
        }
    }

    #[test]
    fn shot_seconds_step_by_two_within_range() {
        assert_eq!(shot_secs_step(8, 1), 10);
        assert_eq!(shot_secs_step(8, -1), 6);
        assert_eq!(shot_secs_step(4, -1), 4);
        assert_eq!(shot_secs_step(30, 1), 30);
        // Off the grid: snapped onto it.
        assert_eq!(shot_secs_step(9, 1), 10);
        assert_eq!(shot_secs_step(9, -1), 6);
    }

    #[test]
    fn warning_steps_stay_on_their_grid() {
        assert_eq!(disk_warn_step(10, 1), 15);
        assert_eq!(disk_warn_step(10, -1), 5);
        assert_eq!(disk_warn_step(5, -1), 5);
        assert_eq!(disk_warn_step(100, 1), 100);
        // A hand-set value off the grid snaps onto it.
        assert_eq!(disk_warn_step(12, 1), 15);
        assert_eq!(disk_warn_step(12, -1), 10);
        assert_eq!(disk_warn_step(500, -1), 95);
        assert_eq!(ram_warn_step(90, 1), 91);
        assert_eq!(ram_warn_step(90, -1), 89);
        assert_eq!(ram_warn_step(98, 1), 98);
        assert_eq!(ram_warn_step(70, -1), 70);
        assert_eq!(ram_warn_step(40, 1), 71);
        assert_eq!(disk_warn_value(10), "10 GB");
    }

    #[test]
    fn robot_rows_follow_the_setting() {
        for lang in [Lang::Id, Lang::En] {
            let on = State {
                lang,
                robot: true,
                robot_motion: Motion::Saver,
                robot_sleep: 10,
                greeting: false,
                ..states()[0].clone()
            };
            let on = State {
                scroll: robot_card_scroll(&on),
                ..on
            };
            let l = layout(Section::Appearance, &on, &Estimate);
            let (_, items, selected) = l.segmented(Group::Motion).expect("Animasi");
            assert_eq!(items[selected].hit, Hit::Motion(Motion::Saver));
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            let want: Vec<&str> = [Key::MotionFull, Key::MotionSaver, Key::MotionOff]
                .iter()
                .map(|k| text(lang, *k))
                .collect();
            assert_eq!(labels, want);
            let value = l.controls.iter().find_map(|c| match c {
                Control::Stepper { minus, value, .. } if minus.1 == Some(Hit::RobotSleep(-1)) => {
                    Some(value.text.clone())
                }
                _ => None,
            });
            assert_eq!(value.as_deref(), Some(sleep_value(lang, 10).as_str()));
            // A wrapped description is its lines in order, joined by spaces.
            let shown = l
                .texts
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            for key in [
                Key::RobotInNotch,
                Key::RobotInNotchDesc,
                Key::RobotMotion,
                Key::RobotMotionDesc,
                Key::RobotSleep,
                Key::RobotSleepDesc,
                Key::MorningGreeting,
                Key::MorningGreetingDesc,
            ] {
                assert!(shown.contains(text(lang, key)), "{key:?}");
            }
            // Off: only the main toggle still takes clicks.
            let off = State {
                robot: false,
                ..on.clone()
            };
            let l = layout(Section::Appearance, &off, &Estimate);
            let hits: Vec<Hit> = l.hits().into_iter().map(|(h, _)| h).collect();
            assert!(hits.contains(&Hit::Robot));
            assert!(
                !hits
                    .iter()
                    .any(|h| matches!(h, Hit::Motion(_) | Hit::RobotSleep(_) | Hit::Greeting))
            );
            // Mati: no sleep timer, so no sleep setting.
            let still = State {
                robot_motion: Motion::Off,
                ..on.clone()
            };
            let l = layout(Section::Appearance, &still, &Estimate);
            assert!(
                !l.hits()
                    .iter()
                    .any(|(h, _)| matches!(h, Hit::RobotSleep(_)))
            );
        }
        assert_eq!(sleep_value(Lang::Id, 10), "10 mnt");
        assert_eq!(sleep_value(Lang::En, 10), "10 min");
    }

    /// The design's full "Hemat: tanpa kedip dan gerak, ekspresi tetap"
    /// wraps beside the segments; its row grows by the extra line and the
    /// rows under it follow, clear of the segmented control.
    #[test]
    fn motion_description_wraps_beside_the_segments() {
        assert_eq!(
            text(Lang::Id, Key::RobotMotionDesc),
            "Hemat: tanpa kedip dan gerak, ekspresi tetap"
        );
        for lang in [Lang::Id, Lang::En] {
            let s = State {
                lang,
                robot: true,
                scroll: 1e6,
                ..states()[0].clone()
            };
            let l = layout(Section::Appearance, &s, &Estimate);
            let label = l
                .texts
                .iter()
                .position(|t| t.text == text(lang, Key::RobotMotion))
                .expect("Animasi label");
            let sleep = l
                .texts
                .iter()
                .position(|t| t.text == text(lang, Key::RobotSleep))
                .expect("sleep label");
            let lines = &l.texts[label + 1..sleep];
            assert_eq!(
                lines
                    .iter()
                    .map(|t| t.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                text(lang, Key::RobotMotionDesc)
            );
            let (track, _, _) = l.segmented(Group::Motion).expect("segments");
            for t in lines {
                assert!(t.rect.right() + 2.0 * GAP <= track.x + 0.01, "{:?}", t.text);
            }
            // The sleep row starts below the last line, with the usual gap.
            let last = lines.last().expect("a line");
            let sleep_label = &l.texts[sleep];
            assert!(sleep_label.rect.y - last.rect.bottom() >= 15.0 - 0.01);
            assert_eq!(
                last.rect.y - lines[0].rect.y,
                (lines.len() - 1) as f32 * DESC_LINE_H
            );
        }
        let b = Builder {
            m: &Estimate,
            lang: Lang::Id,
            l: Layout::default(),
            y: 0.0,
        };
        assert_eq!(b.wrap("a b", Font::Desc, 1.0), ["a", "b"]);
        assert_eq!(b.wrap("a b", Font::Desc, 100.0), ["a b"]);
        assert_eq!(b.wrap("", Font::Desc, 100.0), [""]);
        assert_eq!(wrapped_row_h(1), ROW_H);
        assert_eq!(wrapped_row_h(2), ROW_H + DESC_LINE_H);
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
        let nav = &l.nav[4];
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
            built_in: false,
            advanced: false,
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
    fn built_in_client_offers_one_click_sign_in() {
        let base = states()[0].clone();
        let at = |google: GoogleState| {
            let s = State {
                google,
                ..base.clone()
            };
            layout(Section::Integrations, &s, &Estimate)
        };
        let button = |l: &Layout, hit: Hit| {
            l.controls.iter().find_map(|c| match c {
                Control::Button(b) if b.hit == hit => Some(b.clone()),
                _ => None,
            })
        };
        let g = GoogleState {
            status: CalStatus::NeedsLogin,
            dirty: false,
            has_id: false,
            confirm: false,
            built_in: true,
            advanced: false,
        };
        // One click: no fields, "Masuk dengan Google" as the main button.
        let simple = at(g);
        assert!(simple.fields.is_empty());
        let sign_in = button(&simple, Hit::GoogleConnect).expect("sign-in button");
        assert_eq!(
            (sign_in.label.as_str(), sign_in.kind, sign_in.enabled),
            ("Masuk dengan Google", ButtonKind::Primary, true)
        );
        assert!(button(&simple, Hit::GoogleSave).is_none());
        // Connected: nothing to sign in to, only to forget.
        let ok = at(GoogleState {
            status: CalStatus::Ok,
            ..g
        });
        assert!(!button(&ok, Hit::GoogleConnect).unwrap().enabled);
        assert!(button(&ok, Hit::GoogleDisconnect).unwrap().enabled);
        // "Lanjutan" opens the own-client fields, as without a built-in client.
        // It opens in place, so a chevron, not the "open" glyph of a link.
        let link = button(&simple, Hit::GoogleAdvanced).expect("advanced link");
        assert_eq!(link.kind, ButtonKind::Disclosure(false));
        let open = at(GoogleState {
            advanced: true,
            ..g
        });
        assert_eq!(open.fields.len(), 2);
        assert!(button(&open, Hit::GoogleSave).is_some());
        assert_eq!(
            button(&open, Hit::GoogleAdvanced).map(|b| b.kind),
            Some(ButtonKind::Disclosure(true)),
            "closes again"
        );
        // Without a built-in client: the fields, and no "Lanjutan".
        let plain = at(GoogleState {
            built_in: false,
            ..g
        });
        assert_eq!(plain.fields.len(), 2);
        assert!(button(&plain, Hit::GoogleAdvanced).is_none());
        // An own Client ID wins over Vale's: while one is filled in, its
        // fields and "Hubungkan" show, never "Masuk dengan Google".
        let own = at(GoogleState { has_id: true, ..g });
        assert_eq!(own.fields.len(), 2);
        assert_eq!(button(&own, Hit::GoogleConnect).unwrap().label, "Hubungkan");
    }

    #[test]
    fn clearing_the_clipboard_history_asks_once_more() {
        let base = states()[0].clone();
        let clear = |clip_confirm: bool| {
            let s = State {
                clip_confirm,
                ..base.clone()
            };
            let l = layout(Section::Clipboard, &s, &Estimate);
            l.controls
                .iter()
                .find_map(|c| match c {
                    Control::Button(b) if b.hit == Hit::ClipClear => {
                        Some((b.label.clone(), b.kind))
                    }
                    _ => None,
                })
                .expect("clear button")
        };
        assert_eq!(clear(false).1, ButtonKind::Secondary);
        // The first click arms it, as "Putuskan" does: "Yakin?" in red.
        assert_eq!(clear(true), ("Yakin?".to_string(), ButtonKind::Confirm));
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

    /// The scroll that brings Bata's card (no longer the last in Tampilan)
    /// to the top of the viewport.
    fn robot_card_scroll(s: &State) -> f32 {
        let top = State {
            scroll: 0.0,
            ..s.clone()
        };
        let l = layout(Section::Appearance, &top, &Estimate);
        let view = l.viewport.expect("viewport");
        let toggle = l
            .controls
            .iter()
            .find_map(|c| match c {
                Control::Toggle {
                    hit: Hit::Robot,
                    rect,
                    ..
                } => Some(*rect),
                _ => None,
            })
            .expect("robot toggle");
        let card_top = toggle.y + toggle.h / 2.0 - ROW_H / 2.0;
        card_top - view.y
    }

    /// A page toggle (page, on, enabled) and an order stepper (−, +).
    type PageToggle = (Page, bool, bool);
    type OrderStepper = (Option<Hit>, Option<Hit>);

    /// The "Halaman panel" toggles and order steppers, in card order.
    fn page_controls(l: &Layout) -> (Vec<PageToggle>, Vec<OrderStepper>) {
        let mut toggles = Vec::new();
        let mut steppers = Vec::new();
        for c in &l.controls {
            match c {
                Control::Toggle {
                    hit: Hit::PageOn(p),
                    on,
                    enabled,
                    ..
                } => toggles.push((*p, *on, *enabled)),
                Control::Stepper { minus, plus, .. }
                    if matches!(minus.1.or(plus.1), Some(Hit::PageMove(..))) =>
                {
                    steppers.push((minus.1, plus.1));
                }
                _ => {}
            }
        }
        (toggles, steppers)
    }

    #[test]
    fn panel_pages_card_lists_every_page_with_a_toggle_and_an_order_stepper() {
        for s in states().iter().take(8) {
            let l = layout(Section::Appearance, s, &Estimate);
            let (toggles, steppers) = page_controls(&l);
            let expected: Vec<(Page, bool, bool)> = s
                .pages
                .entries()
                .iter()
                .map(|(p, on)| (*p, *on, true))
                .collect();
            assert_eq!(toggles, expected);
            let order: Vec<Page> = s.pages.entries().iter().map(|e| e.0).collect();
            assert_eq!(steppers.len(), order.len());
            // "−" moves a page earlier, "+" later; none past the ends.
            assert_eq!(steppers[0], (None, Some(Hit::PageMove(order[0], 1))));
            assert_eq!(
                steppers[4],
                (
                    Some(Hit::PageMove(order[4], -1)),
                    Some(Hit::PageMove(order[4], 1))
                )
            );
            assert_eq!(steppers[8], (Some(Hit::PageMove(order[8], -1)), None));
        }
    }

    #[test]
    fn the_last_page_still_shown_cannot_be_switched_off() {
        let mut pages = PanelPages::default();
        for p in &Page::ALL[1..] {
            pages.toggle(*p);
        }
        let s = State {
            pages,
            ..states()[0].clone()
        };
        let l = layout(Section::Appearance, &s, &Estimate);
        let (toggles, _) = page_controls(&l);
        assert_eq!(toggles[0], (Page::Calendar, true, false), "locked on");
        assert!(toggles[1..].iter().all(|(_, on, enabled)| !on && *enabled));
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
                Control::Toggle { hit, rect, on, .. } if *hit == Hit::IdleClock => {
                    Some((*rect, *on))
                }
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
                    Control::Toggle { hit, rect, on, .. } if *hit == Hit::Lyrics => {
                        Some((*rect, *on))
                    }
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
    fn cli_start_folder_sits_just_above_the_shell() {
        for lang in [Lang::Id, Lang::En] {
            for start in CliStart::ALL {
                let s = State {
                    lang,
                    cli_start: start,
                    ..states()[0].clone()
                };
                let l = layout(Section::General, &s, &Estimate);
                let (track, items, selected) =
                    l.segmented(Group::CliStart).expect("Folder awal CLI");
                assert_eq!(items[selected].hit, Hit::CliStart(start));
                let hits: Vec<Hit> = items.iter().map(|i| i.hit).collect();
                assert_eq!(
                    hits,
                    [
                        Hit::CliStart(CliStart::Explorer),
                        Hit::CliStart(CliStart::Last)
                    ]
                );
                let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
                assert_eq!(
                    labels,
                    [
                        text(lang, Key::CliStartExplorer),
                        text(lang, Key::CliStartLast)
                    ]
                );
                let r = items[1].rect;
                assert_eq!(
                    l.hit(r.x + r.w / 2.0, r.y + r.h / 2.0),
                    Some(Hit::CliStart(CliStart::Last))
                );
                // One row above the shell, under everything else.
                let (shell, _, _) = l.segmented(Group::Shell).expect("Shell CLI");
                assert_eq!(shell.y - track.y, ROW_H);
                for key in [Key::CliStartFolder, Key::CliStartFolderDesc] {
                    assert!(
                        l.texts.iter().any(|t| t.text == text(lang, key)),
                        "{lang:?} {key:?}"
                    );
                }
                // The card still ends above the window's bottom margin.
                assert!(l.cards[0].bottom() <= CLIENT_H - PAD);
            }
        }
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
                // Below every other control of the first card (the
                // warnings have a card of their own under it).
                let first = l.cards[0];
                for c in &l.controls {
                    let in_first = c.bounds().bottom() <= first.bottom();
                    if in_first
                        && !matches!(
                            c,
                            Control::Segmented {
                                group: Group::Shell,
                                ..
                            }
                        )
                    {
                        assert!(c.bounds().bottom() <= track.y, "{c:?}");
                    }
                }
                assert!(track.bottom() <= first.bottom());
                for key in [Key::ShellCli, Key::ShellCliDesc] {
                    assert!(l.texts.iter().any(|t| t.text == text(lang, key)));
                }
            }
        }
        let l = layout(Section::Appearance, &states()[0], &Estimate);
        assert!(l.segmented(Group::Shell).is_none());
    }

    #[test]
    fn studio_radius_scale() {
        assert_eq!(
            (CARD_RADIUS, CONTROL_RADIUS, SEGMENT_RADIUS),
            (14.0, 10.0, 8.0)
        );
        // Fully rounded: wider than half of any chip, button or toggle.
        assert!(FULL_RADIUS >= CHIP_H.max(BUTTON_H).max(TOGGLE_H).max(STEP_BUTTON) / 2.0);
        // The segment and its track keep the same gap all round.
        assert_eq!(CONTROL_RADIUS - SEGMENT_RADIUS, SEG_PAD);
    }

    #[test]
    fn integrations_show_the_status_as_chips() {
        let base = states()[0].clone();
        for status in [
            CalStatus::NotConfigured,
            CalStatus::NeedsLogin,
            CalStatus::Reconnect,
            CalStatus::Ok,
            CalStatus::Offline,
        ] {
            let s = State {
                google: GoogleState {
                    status,
                    ..base.google
                },
                hooks: Some(HookState {
                    complete: true,
                    any: true,
                }),
                ..base.clone()
            };
            let l = layout(Section::Integrations, &s, &Estimate);
            let (key, tone) = status_key(status);
            let chips: Vec<(&str, Tone)> = l
                .chips
                .iter()
                .map(|c| (c.label.text.as_str(), c.label.tone))
                .collect();
            assert_eq!(
                chips,
                [
                    (text(s.lang, key), tone),
                    (text(s.lang, Key::HooksOn), Tone::Good)
                ]
            );
            // A message after a click stands in for the Google status.
            let noted = State {
                google_note: Some(("Tersimpan", Tone::Good)),
                ..s.clone()
            };
            let l = layout(Section::Integrations, &noted, &Estimate);
            assert_eq!(l.chips.len(), 1);
            assert!(l.texts.iter().any(|t| t.text == "Tersimpan"));
        }
        // Each card has its uppercase name above it.
        let l = layout(Section::Integrations, &base, &Estimate);
        for key in [Key::GoogleCalendar, Key::ClaudeCode] {
            let name = text(base.lang, key).to_uppercase();
            let label = l.texts.iter().find(|t| t.text == name).expect("eyebrow");
            assert_eq!(label.font, Font::Eyebrow);
            assert!(l.cards.iter().any(|c| c.y > label.rect.bottom()));
        }
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
