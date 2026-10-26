//! Theme settings (accent colour, island surface, window mode) and the
//! colours they give the island renderer, the editors and the settings
//! window. The island itself is always dark; only its surface changes.
//! Windows (editors, settings) follow the window mode, where "System" reads
//! the Windows app mode (`AppsUseLightTheme`).
//!
//! Colours are `0xRRGGBB` unless a field says otherwise.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::config::Config;
use crate::pomodoro::Phase;

/// Declares a settings enum stored in config.json by its variant name. An
/// unknown name (a typo while editing by hand) reads as the default instead of
/// making the whole config invalid. Optional `aliases` map names an older
/// version wrote onto current variants; they are read, never written.
macro_rules! named_enum {
    (
        $(#[$doc:meta])* $name:ident { $default:ident $(, $other:ident)* $(,)? }
        $(aliases { $($alias:literal => $target:ident),* $(,)? })?
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum $name {
            #[default]
            $default,
            $($other,)*
        }

        impl $name {
            pub const ALL: &[$name] = &[$name::$default $(, $name::$other)*];

            /// Position in `ALL`.
            fn index(self) -> u32 {
                Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as u32
            }

            /// `ALL[i]`, or the default past its end.
            fn from_index(i: u32) -> Self {
                Self::ALL.get(i as usize).copied().unwrap_or_default()
            }

            pub fn name(self) -> &'static str {
                match self {
                    $name::$default => stringify!($default),
                    $($name::$other => stringify!($other),)*
                }
            }

            /// Older names and the variants they now read as.
            const ALIASES: &[(&str, $name)] = &[$($(($alias, $name::$target),)*)?];

            /// Case-insensitive; a current name first, then an alias.
            pub fn from_name(name: &str) -> Option<Self> {
                let name = name.trim();
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.name().eq_ignore_ascii_case(name))
                    .or_else(|| {
                        Self::ALIASES
                            .iter()
                            .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
                            .map(|(_, v)| *v)
                    })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.name())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let name = String::deserialize(d)?;
                Ok(Self::from_name(&name).unwrap_or_default())
            }
        }
    };
}

named_enum! {
    /// Accent colour of the island and the windows.
    Accent { Terracotta, Sage, Ochre, Clay, Olive, Plum }
    aliases {
        "Blue" => Terracotta,
        "Green" => Sage,
        "Orange" => Ochre,
        "Pink" => Clay,
        "Purple" => Plum,
        "Red" => Terracotta,
    }
}

named_enum! {
    /// The island's fill.
    Surface { Espresso, Graphite }
    aliases { "Black" => Espresso }
}

named_enum! {
    /// Light or dark editors and settings window; `System` follows Windows.
    WindowTheme { System, Dark, Light }
}

impl Accent {
    /// The accent on the island and in dark windows.
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Terracotta => 0xE2683C,
            Accent::Sage => 0x9DB59A,
            Accent::Ochre => 0xD9A441,
            Accent::Clay => 0xC9826B,
            Accent::Olive => 0xA3A55A,
            Accent::Plum => 0xB07A9A,
        }
    }

    /// Text and marks drawn on `rgb()`: espresso ink, which reads on every
    /// one of these mid-light earth tones.
    pub fn ink(self) -> u32 {
        0x17120F
    }

    /// The accent in light windows: a darker shade that holds 3:1 against
    /// the light background and cards.
    pub fn light_rgb(self) -> u32 {
        match self {
            Accent::Terracotta => 0xC9542B,
            Accent::Sage => 0x6C7D6A,
            Accent::Ochre => 0x96712D,
            Accent::Clay => 0xA36957,
            Accent::Olive => 0x7A7C44,
            Accent::Plum => 0x996A86,
        }
    }

    /// Text and marks drawn on `light_rgb()`.
    pub fn light_ink(self) -> u32 {
        0xFFFBF5
    }
}

impl Surface {
    pub fn rgb(self) -> u32 {
        match self {
            Surface::Espresso => 0x17120F,
            Surface::Graphite => 0x221B17,
        }
    }
}

/// The three theme settings together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    pub accent: Accent,
    pub surface: Surface,
    pub window: WindowTheme,
}

impl Theme {
    pub fn of(cfg: &Config) -> Self {
        Theme {
            accent: cfg.accent,
            surface: cfg.surface,
            window: cfg.window_theme,
        }
    }
}

/// Island colours. Panel rows, tracks and cards sit a step above the surface,
/// so they lighten with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The island's fill.
    pub surface: u32,
    /// Note rows, the view selector track, the waiting strip.
    pub raised: u32,
    /// The active segment of the view selector.
    pub raised_hi: u32,
    /// Ring tracks, stepper and secondary buttons, dividers.
    pub track: u32,
    /// Week view day columns.
    pub column: u32,
    /// Album art placeholder tile.
    pub placeholder: u32,
    /// Inactive page dots.
    pub dot_off: u32,
    pub text: u32,
    /// Today, the selected day, event dot fallback, equalizer, primary
    /// buttons, the active page dot.
    pub accent: u32,
    /// Text and marks on an accent fill.
    pub on_accent: u32,
    /// Captions, times, hour labels, inactive segments.
    pub secondary: u32,
    /// Status colours, fixed whatever the accent: sage for breaks, event
    /// dots and all-day events; ochre for battery and "Terlewat"; plum for
    /// a third event colour.
    pub sage: u32,
    pub ochre: u32,
    pub plum: u32,
}

impl Palette {
    /// The pomodoro's focus phase: the accent.
    pub fn focus(&self) -> u32 {
        self.accent
    }

    /// The pomodoro's breaks: sage.
    pub fn rest(&self) -> u32 {
        self.sage
    }

    /// A pomodoro phase's colour: `focus` or `rest`.
    pub fn phase(&self, phase: Phase) -> u32 {
        match phase {
            Phase::Focus => self.focus(),
            Phase::ShortBreak | Phase::LongBreak => self.rest(),
        }
    }
}

/// Corner radii of the Studio scale, in logical px. A chip is fully rounded
/// (half its height) and has no constant.
pub mod radius {
    /// The open panel.
    pub const PANEL: f32 = 30.0;
    /// The island's notices, both styles.
    pub const NOTICE: f32 = 26.0;
    /// The media page's album cover.
    pub const COVER: f32 = 22.0;
    /// Shortcut tiles.
    pub const TILE: f32 = 18.0;
    /// Cards: the notes page's.
    pub const CARD: f32 = 14.0;
    /// A notice's badge.
    pub const BADGE: f32 = 13.0;
    /// Week view day columns.
    pub const COLUMN: f32 = 12.0;
    /// Controls: the view selector's track, buttons.
    pub const CONTROL: f32 = 10.0;
    /// Day view event blocks.
    pub const BLOCK: f32 = 9.0;
    /// A segment inside a control, the all-day band.
    pub const SEGMENT: f32 = 8.0;
    /// Week view event chips.
    pub const EVENT_CHIP: f32 = 7.0;
    /// The compact album art.
    pub const ART: f32 = 6.0;
}

pub fn island(accent: Accent, surface: Surface) -> Palette {
    // Warm browns over espresso; graphite lifts each a step.
    let (raised, raised_hi, track, column, placeholder, dot_off) = match surface {
        Surface::Espresso => (0x241D18, 0x3A2F27, 0x332A23, 0x1D1713, 0x332A23, 0x4A3E35),
        Surface::Graphite => (0x2E2620, 0x463A31, 0x3D332B, 0x272019, 0x3D332B, 0x574A40),
    };
    Palette {
        surface: surface.rgb(),
        raised,
        raised_hi,
        track,
        column,
        placeholder,
        dot_off,
        text: 0xF4EDE4,
        accent: accent.rgb(),
        on_accent: accent.ink(),
        secondary: 0xA89A8B,
        sage: Accent::Sage.rgb(),
        ochre: Accent::Ochre.rgb(),
        plum: Accent::Plum.rgb(),
    }
}

pub fn island_palette(cfg: &Config) -> Palette {
    island(cfg.accent, cfg.surface)
}

/// The CLI's terminal (the panel card and the window's output), the same in
/// both window modes: background and text.
pub const TERMINAL_BG: u32 = 0x0F0C0A;
pub const TERMINAL_TEXT: u32 = 0xF4EDE4;

/// Colours of ordinary windows (editors, settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPalette {
    pub light: bool,
    pub bg: u32,
    pub card: u32,
    /// `0xRRGGBBAA`: a hairline at low alpha over `bg` or `card`.
    pub border: u32,
    pub text: u32,
    pub secondary: u32,
    pub accent: u32,
    pub on_accent: u32,
    /// Text field fill (editors).
    pub field: u32,
    /// Secondary button fill, and its hover.
    pub button: u32,
    pub button_hover: u32,
    /// Error messages.
    pub error: u32,
}

pub fn window(accent: Accent, mode: WindowTheme, system_light: bool) -> WindowPalette {
    let light = match mode {
        WindowTheme::System => system_light,
        WindowTheme::Dark => false,
        WindowTheme::Light => true,
    };
    if light {
        WindowPalette {
            light,
            bg: 0xF4EEE6,
            card: 0xFFFBF5,
            border: 0x221A141A,
            text: 0x221A14,
            // The design's #7A6B5D falls just short of 4.5:1 on bg.
            secondary: 0x786A5C,
            accent: accent.light_rgb(),
            on_accent: accent.light_ink(),
            field: 0xFFFFFF,
            button: 0xEFE6DA,
            button_hover: 0xEADFD1,
            error: 0xC42B1C,
        }
    } else {
        WindowPalette {
            light,
            bg: 0x1E1814,
            card: 0x272019,
            border: 0xF4EDE414,
            text: 0xF4EDE4,
            secondary: 0xA89A8B,
            accent: accent.rgb(),
            on_accent: accent.ink(),
            field: 0x2C241D,
            button: 0x33291F,
            button_hover: 0x3D3227,
            error: 0xEF7A6E,
        }
    }
}

/// The theme in effect, for drawing code that has no `Config` (the editors):
/// set by the app, read on the UI thread.
static CURRENT: AtomicU32 = AtomicU32::new(0);
static SYSTEM_LIGHT: AtomicBool = AtomicBool::new(false);

fn pack(t: Theme) -> u32 {
    t.accent.index() | t.surface.index() << 8 | t.window.index() << 16
}

fn unpack(v: u32) -> Theme {
    Theme {
        accent: Accent::from_index(v & 0xFF),
        surface: Surface::from_index((v >> 8) & 0xFF),
        window: WindowTheme::from_index((v >> 16) & 0xFF),
    }
}

/// Makes `t` the theme in effect. True when it changed.
pub fn set_current(t: Theme) -> bool {
    CURRENT.swap(pack(t), Ordering::Relaxed) != pack(t)
}

pub fn current() -> Theme {
    unpack(CURRENT.load(Ordering::Relaxed))
}

/// Records whether Windows apps are in light mode. True when it changed.
pub fn set_system_light(light: bool) -> bool {
    SYSTEM_LIGHT.swap(light, Ordering::Relaxed) != light
}

/// The window palette in effect.
pub fn current_window() -> WindowPalette {
    let t = current();
    window(t.accent, t.window, SYSTEM_LIGHT.load(Ordering::Relaxed))
}

/// `AppsUseLightTheme` under HKCU Personalize; light when the value is
/// missing, as Windows itself assumes.
pub fn read_system_light() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_err() || value != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_match_the_pinned_hex_values() {
        let hex: Vec<(Accent, u32)> = Accent::ALL.iter().map(|a| (*a, a.rgb())).collect();
        assert_eq!(
            hex,
            [
                (Accent::Terracotta, 0xE2683C),
                (Accent::Sage, 0x9DB59A),
                (Accent::Ochre, 0xD9A441),
                (Accent::Clay, 0xC9826B),
                (Accent::Olive, 0xA3A55A),
                (Accent::Plum, 0xB07A9A),
            ]
        );
        assert_eq!(Accent::default(), Accent::Terracotta);
        assert_eq!(Surface::default(), Surface::Espresso);
        assert_eq!(Accent::Terracotta.light_rgb(), 0xC9542B);
    }

    fn luminance(rgb: u32) -> f64 {
        let lin = |shift: u32| {
            let c = ((rgb >> shift) & 0xFF) as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(16) + 0.7152 * lin(8) + 0.0722 * lin(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn ink_on_every_accent_is_readable() {
        for a in Accent::ALL {
            let c = contrast(a.rgb(), a.ink());
            assert!(c >= 3.0, "{a:?}: {c:.2}");
            // The espresso ink reads as body text on every accent.
            assert!(c >= 4.5, "{a:?}: {c:.2}");
        }
    }

    #[test]
    fn light_mode_accents_read_on_the_light_window() {
        let p = window(Accent::Terracotta, WindowTheme::Light, false);
        for a in Accent::ALL {
            let (fill, ink) = (a.light_rgb(), a.light_ink());
            for surface in [p.bg, p.card] {
                let c = contrast(fill, surface);
                assert!(c >= 3.0, "{a:?} on {surface:06X}: {c:.2}");
            }
            let c = contrast(fill, ink);
            assert!(c >= 3.0, "{a:?} ink: {c:.2}");
        }
    }

    #[test]
    fn espresso_surface_matches_the_pinned_island_colours() {
        let p = island(Accent::Terracotta, Surface::Espresso);
        assert_eq!(
            p,
            Palette {
                surface: 0x17120F,
                raised: 0x241D18,
                raised_hi: 0x3A2F27,
                track: 0x332A23,
                column: 0x1D1713,
                placeholder: 0x332A23,
                dot_off: 0x4A3E35,
                text: 0xF4EDE4,
                accent: 0xE2683C,
                on_accent: 0x17120F,
                secondary: 0xA89A8B,
                sage: 0x9DB59A,
                ochre: 0xD9A441,
                plum: 0xB07A9A,
            }
        );
        assert_eq!((p.focus(), p.rest()), (0xE2683C, 0x9DB59A));
    }

    #[test]
    fn island_text_and_status_colours_read_on_every_surface() {
        for &surface in Surface::ALL {
            let p = island(Accent::Terracotta, surface);
            assert!(contrast(p.text, p.surface) >= 7.0, "{surface:?}");
            assert!(contrast(p.secondary, p.surface) >= 4.5, "{surface:?}");
            assert!(contrast(p.secondary, p.raised) >= 4.5, "{surface:?}");
            for status in [p.sage, p.ochre, p.plum] {
                assert!(contrast(status, p.surface) >= 4.5, "{status:06X}");
            }
        }
        // Status colours stay put whatever the accent.
        let plum = island(Accent::Plum, Surface::Espresso);
        assert_eq!((plum.sage, plum.ochre), (0x9DB59A, 0xD9A441));
        assert_eq!((plum.focus(), plum.rest()), (0xB07A9A, 0x9DB59A));
    }

    #[test]
    fn graphite_lightens_everything_raised() {
        let espresso = island(Accent::Ochre, Surface::Espresso);
        let graphite = island(Accent::Ochre, Surface::Graphite);
        assert_eq!(graphite.surface, 0x221B17);
        assert_eq!((graphite.accent, graphite.on_accent), (0xD9A441, 0x17120F));
        assert_eq!(graphite.text, espresso.text, "the island stays dark");
        let raised = |p: &Palette| {
            [
                p.raised,
                p.raised_hi,
                p.track,
                p.column,
                p.placeholder,
                p.dot_off,
            ]
        };
        for (e, g) in raised(&espresso).into_iter().zip(raised(&graphite)) {
            assert!(luminance(g) > luminance(e), "{g:06X} vs {e:06X}");
            // Still a visible step above the graphite surface.
            assert!(luminance(g) > luminance(graphite.surface), "{g:06X}");
        }
    }

    #[test]
    fn window_modes_match_the_pinned_hex_values() {
        let dark = window(Accent::Terracotta, WindowTheme::Dark, true);
        assert!(!dark.light);
        assert_eq!(
            (dark.bg, dark.card, dark.border, dark.text, dark.secondary),
            (0x1E1814, 0x272019, 0xF4EDE414, 0xF4EDE4, 0xA89A8B)
        );
        assert_eq!(
            (dark.field, dark.button, dark.button_hover),
            (0x2C241D, 0x33291F, 0x3D3227)
        );
        assert_eq!((dark.accent, dark.on_accent), (0xE2683C, 0x17120F));
        let light = window(Accent::Terracotta, WindowTheme::Light, false);
        assert!(light.light);
        assert_eq!(
            (
                light.bg,
                light.card,
                light.border,
                light.text,
                light.secondary
            ),
            (0xF4EEE6, 0xFFFBF5, 0x221A141A, 0x221A14, 0x786A5C)
        );
        assert_eq!(
            (light.field, light.button, light.button_hover),
            (0xFFFFFF, 0xEFE6DA, 0xEADFD1)
        );
        assert_eq!((light.accent, light.on_accent), (0xC9542B, 0xFFFBF5));
        for p in [dark, light] {
            assert!(contrast(p.text, p.bg) >= 7.0);
            assert!(contrast(p.text, p.field) >= 7.0);
            assert!(contrast(p.secondary, p.bg) >= 4.5);
            assert!(contrast(p.error, p.bg) >= 4.5);
        }
    }

    #[test]
    fn system_follows_the_windows_flag() {
        let sys = |light| window(Accent::Sage, WindowTheme::System, light);
        assert_eq!(sys(true), window(Accent::Sage, WindowTheme::Light, false));
        assert_eq!(sys(false), window(Accent::Sage, WindowTheme::Dark, true));
        let cfg = Config::default();
        assert_eq!(cfg.window_theme, WindowTheme::System);
        let t = Theme::of(&cfg);
        assert!(window(t.accent, t.window, true).light);
        assert!(!window(t.accent, t.window, false).light);
        assert_eq!(
            island_palette(&cfg),
            island(Accent::Terracotta, Surface::Espresso)
        );
    }

    #[test]
    fn names_round_trip_and_unknown_names_are_rejected() {
        for a in Accent::ALL {
            assert_eq!(Accent::from_name(a.name()), Some(*a));
        }
        assert_eq!(Accent::from_name("ochre"), Some(Accent::Ochre));
        assert_eq!(Surface::from_name(" GRAPHITE "), Some(Surface::Graphite));
        assert_eq!(WindowTheme::from_name("light"), Some(WindowTheme::Light));
        assert_eq!(Accent::from_name("teal"), None);
        assert_eq!(Surface::from_name("grey"), None);
        assert_eq!(WindowTheme::from_name("black"), None);
    }

    #[test]
    fn old_names_read_as_their_studio_colours() {
        for (old, new) in [
            ("Blue", Accent::Terracotta),
            ("Green", Accent::Sage),
            ("Orange", Accent::Ochre),
            ("Pink", Accent::Clay),
            ("Purple", Accent::Plum),
            ("Red", Accent::Terracotta),
        ] {
            assert_eq!(Accent::from_name(old), Some(new), "{old}");
            let shouted = format!("  {}  ", old.to_uppercase());
            assert_eq!(Accent::from_name(&shouted), Some(new), "{old}");
        }
        assert_eq!(Surface::from_name("black"), Some(Surface::Espresso));
        // Aliases are read, never written.
        for a in Accent::ALL {
            assert!(Accent::ALIASES.iter().all(|(old, _)| *old != a.name()));
        }
        assert_eq!(
            serde_json::to_string(&Accent::from_name("Blue").unwrap()).unwrap(),
            "\"Terracotta\""
        );
        assert_eq!(
            serde_json::from_str::<Surface>("\"Black\"").unwrap(),
            Surface::Espresso
        );
    }

    #[test]
    fn theme_packs_losslessly() {
        for &accent in Accent::ALL {
            for &surface in Surface::ALL {
                for &window in WindowTheme::ALL {
                    let t = Theme {
                        accent,
                        surface,
                        window,
                    };
                    assert_eq!(unpack(pack(t)), t);
                }
            }
        }
    }
}
