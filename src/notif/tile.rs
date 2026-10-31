//! The letter tile a row shows when its app's logo could not be read: a
//! colour derived from the app's name (the same name, the same colour, every
//! run) and the name's first letter.

/// FNV-1a over the name's lowercase characters, so "WhatsApp" and "whatsapp"
/// share a tile.
fn hash(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}

/// 0xRRGGBB from hue (degrees), saturation and lightness (0..1).
fn hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// The tile's fill for `name`: one of 36 hues, muted and dark enough for
/// light text on it.
pub fn tile_color(name: &str) -> u32 {
    let hue = (hash(name) % 36) as f32 * 10.0;
    hsl(hue, 0.42, 0.40)
}

/// The tile's letter: the first letter or digit of `name`, uppercased; "?"
/// for a name with none.
pub fn tile_letter(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_is_stable_and_ignores_case_and_edge_blanks() {
        assert_eq!(tile_color("WhatsApp"), tile_color("WhatsApp"));
        assert_eq!(tile_color("WhatsApp"), tile_color("  whatsapp "));
        assert_eq!(tile_color("Teams"), tile_color("teams"));
    }

    #[test]
    fn different_apps_mostly_get_different_hues_and_all_stay_in_range() {
        let names = [
            "WhatsApp", "Teams", "Chrome", "Outlook", "Slack", "Discord", "Telegram", "Edge",
            "Spotify", "Mail", "Calendar", "Photos",
        ];
        let colours: Vec<u32> = names.iter().map(|n| tile_color(n)).collect();
        for c in &colours {
            assert!(*c <= 0xFF_FFFF);
            // Dark enough for light text: no channel above 0.7.
            for shift in [16, 8, 0] {
                assert!(((c >> shift) & 0xFF) <= 180, "{c:06x}");
            }
        }
        let mut unique = colours.clone();
        unique.sort_unstable();
        unique.dedup();
        assert!(unique.len() >= names.len() - 3, "{unique:x?}");
    }

    #[test]
    fn hsl_hits_the_primaries() {
        assert_eq!(hsl(0.0, 1.0, 0.5), 0xFF0000);
        assert_eq!(hsl(120.0, 1.0, 0.5), 0x00FF00);
        assert_eq!(hsl(240.0, 1.0, 0.5), 0x0000FF);
        assert_eq!(hsl(0.0, 0.0, 0.5), 0x808080);
    }

    #[test]
    fn the_letter_is_the_first_letter_or_digit_uppercased() {
        assert_eq!(tile_letter("WhatsApp"), "W");
        assert_eq!(tile_letter("teams"), "T");
        assert_eq!(tile_letter("  (beta) app"), "B");
        assert_eq!(tile_letter("7-Zip"), "7");
        assert_eq!(tile_letter("épice"), "É");
        assert_eq!(tile_letter("..."), "?");
        assert_eq!(tile_letter(""), "?");
    }
}
