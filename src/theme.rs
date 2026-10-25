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

/// Declares a settings enum stored in config.json by its variant name. An
/// unknown name (a typo while editing by hand) reads as the default instead of
/// making the whole config invalid.
macro_rules! named_enum {
    ($(#[$doc:meta])* $name:ident { $default:ident $(, $other:ident)* $(,)? }) => {
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

            /// Case-insensitive.
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|v| v.name().eq_ignore_ascii_case(name.trim()))
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
    Accent { Blue, Purple, Green, Orange, Pink, Red }
}

named_enum! {
    /// The island's fill.
    Surface { Black, Graphite }
}

named_enum! {
    /// Light or dark editors and settings window; `System` follows Windows.
    WindowTheme { System, Dark, Light }
}

impl Accent {
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Blue => 0x4F8CFF,
            Accent::Purple => 0xA78BFA,
            Accent::Green => 0x30D158,
            Accent::Orange => 0xFF9F0A,
            Accent::Pink => 0xFF6482,
            Accent::Red => 0xFF453A,
        }
    }

    /// Text and marks drawn on an accent fill: white where it keeps at least
    /// 3:1 contrast, near-black on the lighter accents.
    pub fn ink(self) -> u32 {
        match self {
            Accent::Blue | Accent::Red => 0xFFFFFF,
            Accent::Purple | Accent::Green | Accent::Orange | Accent::Pink => 0x000000,
        }
    }
}

impl Surface {
    pub fn rgb(self) -> u32 {
        match self {
            Surface::Black => 0x000000,
            Surface::Graphite => 0x1C1C1E,
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
}

pub fn island(accent: Accent, surface: Surface) -> Palette {
    // Black keeps the island's original greys; graphite lifts each a step.
    let (raised, raised_hi, track, column, placeholder, dot_off) = match surface {
        Surface::Black => (0x1C1C1E, 0x3A3A3C, 0x2A2A2C, 0x111113, 0x2C2C2E, 0x555555),
        Surface::Graphite => (0x2C2C2E, 0x48484A, 0x3A3A3C, 0x252527, 0x3A3A3C, 0x636366),
    };
    Palette {
        surface: surface.rgb(),
        raised,
        raised_hi,
        track,
        column,
        placeholder,
        dot_off,
        text: 0xF2F2F2,
        accent: accent.rgb(),
        on_accent: accent.ink(),
    }
}

pub fn island_palette(cfg: &Config) -> Palette {
    island(cfg.accent, cfg.surface)
}

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
    let (accent, on_accent) = (accent.rgb(), accent.ink());
    if light {
        WindowPalette {
            light,
            bg: 0xF3F3F5,
            card: 0xFFFFFF,
            border: 0x00000014,
            text: 0x1B1B1D,
            secondary: 0x6B6B70,
            accent,
            on_accent,
            field: 0xFFFFFF,
            button: 0xF6F6F8,
            button_hover: 0xEBEBEE,
            error: 0xC42B1C,
        }
    } else {
        WindowPalette {
            light,
            bg: 0x1B1B1D,
            card: 0x242427,
            border: 0xFFFFFF14,
            text: 0xF2F2F2,
            secondary: 0xA0A0A5,
            accent,
            on_accent,
            field: 0x2B2B2E,
            button: 0x2E2E32,
            button_hover: 0x38383C,
            error: 0xFF6B6B,
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
                (Accent::Blue, 0x4F8CFF),
                (Accent::Purple, 0xA78BFA),
                (Accent::Green, 0x30D158),
                (Accent::Orange, 0xFF9F0A),
                (Accent::Pink, 0xFF6482),
                (Accent::Red, 0xFF453A),
            ]
        );
        assert_eq!(Accent::default(), Accent::Blue);
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
            // White only where it reads; the lighter accents get dark ink.
            assert_eq!(
                a.ink() == 0xFFFFFF,
                contrast(a.rgb(), 0xFFFFFF) >= 3.0,
                "{a:?}"
            );
        }
    }

    #[test]
    fn black_surface_keeps_the_original_island_colours() {
        let p = island(Accent::Blue, Surface::Black);
        assert_eq!(
            p,
            Palette {
                surface: 0x000000,
                raised: 0x1C1C1E,
                raised_hi: 0x3A3A3C,
                track: 0x2A2A2C,
                column: 0x111113,
                placeholder: 0x2C2C2E,
                dot_off: 0x555555,
                text: 0xF2F2F2,
                accent: 0x4F8CFF,
                on_accent: 0xFFFFFF,
            }
        );
    }

    #[test]
    fn graphite_lightens_everything_raised() {
        let black = island(Accent::Orange, Surface::Black);
        let graphite = island(Accent::Orange, Surface::Graphite);
        assert_eq!(graphite.surface, 0x1C1C1E);
        assert_eq!((graphite.accent, graphite.on_accent), (0xFF9F0A, 0x000000));
        assert_eq!(graphite.text, black.text, "the island stays dark");
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
        for (b, g) in raised(&black).into_iter().zip(raised(&graphite)) {
            assert!(luminance(g) > luminance(b), "{g:06X} vs {b:06X}");
            // Still a visible step above the graphite surface.
            assert!(luminance(g) > luminance(graphite.surface), "{g:06X}");
        }
    }

    #[test]
    fn window_modes_match_the_pinned_hex_values() {
        let dark = window(Accent::Blue, WindowTheme::Dark, true);
        assert!(!dark.light);
        assert_eq!(
            (dark.bg, dark.card, dark.border, dark.text, dark.secondary),
            (0x1B1B1D, 0x242427, 0xFFFFFF14, 0xF2F2F2, 0xA0A0A5)
        );
        let light = window(Accent::Pink, WindowTheme::Light, false);
        assert!(light.light);
        assert_eq!(
            (
                light.bg,
                light.card,
                light.border,
                light.text,
                light.secondary
            ),
            (0xF3F3F5, 0xFFFFFF, 0x00000014, 0x1B1B1D, 0x6B6B70)
        );
        assert_eq!((light.accent, light.on_accent), (0xFF6482, 0x000000));
        for p in [dark, light] {
            assert!(contrast(p.text, p.bg) >= 7.0);
            assert!(contrast(p.text, p.field) >= 7.0);
            assert!(contrast(p.secondary, p.bg) >= 4.5);
            assert!(contrast(p.error, p.bg) >= 4.5);
        }
    }

    #[test]
    fn system_follows_the_windows_flag() {
        let sys = |light| window(Accent::Blue, WindowTheme::System, light);
        assert_eq!(sys(true), window(Accent::Blue, WindowTheme::Light, false));
        assert_eq!(sys(false), window(Accent::Blue, WindowTheme::Dark, true));
        let cfg = Config::default();
        assert_eq!(cfg.window_theme, WindowTheme::System);
        let t = Theme::of(&cfg);
        assert!(window(t.accent, t.window, true).light);
        assert!(!window(t.accent, t.window, false).light);
        assert_eq!(island_palette(&cfg), island(Accent::Blue, Surface::Black));
    }

    #[test]
    fn names_round_trip_and_unknown_names_are_rejected() {
        for a in Accent::ALL {
            assert_eq!(Accent::from_name(a.name()), Some(*a));
        }
        assert_eq!(Accent::from_name("orange"), Some(Accent::Orange));
        assert_eq!(Surface::from_name(" GRAPHITE "), Some(Surface::Graphite));
        assert_eq!(WindowTheme::from_name("light"), Some(WindowTheme::Light));
        assert_eq!(Accent::from_name("teal"), None);
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
