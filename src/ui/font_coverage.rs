//! Guards the Latin subset built by `scripts/make-fonts.ps1`: the embedded
//! fonts must keep every glyph the UI draws from them. Pure test code; it reads
//! the cmap of the TTF bytes and scans the sources for non-ASCII characters.
//! See the note `docs/memory/exe-size.md` before trimming a font further.

use std::collections::BTreeSet;
use std::path::Path;

const FONTS: [(&str, &[u8]); 5] = [
    ("DMSans-400", include_bytes!("../../assets/fonts/DMSans-400.ttf")),
    ("DMSans-500", include_bytes!("../../assets/fonts/DMSans-500.ttf")),
    ("DMSans-600", include_bytes!("../../assets/fonts/DMSans-600.ttf")),
    ("DMMono-400", include_bytes!("../../assets/fonts/DMMono-400.ttf")),
    ("DMMono-500", include_bytes!("../../assets/fonts/DMMono-500.ttf")),
];

/// Characters the UI draws that are NOT in the subset. They have always come
/// from DirectWrite's system fallback (Segoe UI Symbol / Segoe UI Emoji), so
/// they are listed here to make that a decision, not an accident. Do not add
/// to it to silence a failure after trimming a font.
const SYSTEM_FALLBACK: &[char] = &[
    '\u{2197}', // ↗ model.rs, render_panel.rs
    '\u{21B3}', // ↳ CLI notices (i18n.rs)
    '\u{2264}', // ≤ render.rs
    '\u{25B8}', // ▸ Mole log
    '\u{266A}', // ♪ geometry.rs
    '\u{25CF}', // ● demo only
    '\u{2713}', // ✓ demo only; DM Sans has no check mark
    '\u{2500}', '\u{2502}', '\u{2503}', '\u{257F}', // box drawing, Mole log
];

/// Non-ASCII code points in the sources that are never drawn: BOM and
/// replacement-character checks, zero-width joiner, ESC/CSI controls.
const NOT_DRAWN: &[char] = &['\u{FEFF}', '\u{FFFD}', '\u{200D}', '\u{9B}'];

/// Mirrors `UNICODES` in `scripts/make-fonts.ps1`.
const SUBSET_RANGES: &[(u32, u32)] = &[
    (0x0020, 0x007E),
    (0x00A0, 0x00FF),
    (0x0100, 0x017F),
    (0x2010, 0x2027),
    (0x2030, 0x203A),
    (0x20AC, 0x20AC),
    (0x2122, 0x2122),
    (0x2190, 0x2193),
    (0x2212, 0x2212),
    (0x2713, 0x2713),
];

fn be16(d: &[u8], at: usize) -> usize {
    u16::from_be_bytes([d[at], d[at + 1]]) as usize
}

fn be32(d: &[u8], at: usize) -> usize {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as usize
}

/// Every code point mapped by the font's Unicode cmap (formats 4 and 12).
fn cmap(font: &[u8]) -> BTreeSet<u32> {
    let tables = be16(font, 4);
    let mut cmap_at = None;
    for i in 0..tables {
        let rec = 12 + 16 * i;
        if &font[rec..rec + 4] == b"cmap" {
            cmap_at = Some(be32(font, rec + 8));
        }
    }
    let base = cmap_at.expect("cmap table");
    let mut set = BTreeSet::new();
    for i in 0..be16(font, base + 2) {
        let rec = base + 4 + 8 * i;
        let (platform, encoding) = (be16(font, rec), be16(font, rec + 2));
        if !(platform == 3 && (encoding == 1 || encoding == 10)) {
            continue;
        }
        let sub = base + be32(font, rec + 4);
        match be16(font, sub) {
            4 => {
                let segs = be16(font, sub + 6) / 2;
                let ends = sub + 14;
                let starts = ends + 2 * segs + 2;
                let deltas = starts + 2 * segs;
                let offsets = deltas + 2 * segs;
                for s in 0..segs {
                    let (end, start) = (be16(font, ends + 2 * s), be16(font, starts + 2 * s));
                    if start == 0xFFFF {
                        continue;
                    }
                    for cp in start..=end {
                        let ro = be16(font, offsets + 2 * s);
                        let gid = if ro == 0 {
                            (cp + be16(font, deltas + 2 * s)) & 0xFFFF
                        } else {
                            let at = offsets + 2 * s + ro + 2 * (cp - start);
                            match be16(font, at) {
                                0 => 0,
                                g => (g + be16(font, deltas + 2 * s)) & 0xFFFF,
                            }
                        };
                        if gid != 0 {
                            set.insert(cp as u32);
                        }
                    }
                }
            }
            12 => {
                for g in 0..be32(font, sub + 12) {
                    let at = sub + 16 + 12 * g;
                    let (start, end, gid) = (be32(font, at), be32(font, at + 4), be32(font, at + 8));
                    for cp in start..=end {
                        if gid + (cp - start) != 0 {
                            set.insert(cp as u32);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    set
}

fn rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Non-ASCII characters written in a source file outside comments and test
/// modules: raw in a literal, or as a `\u{..}` escape.
fn drawn_chars(source: &str) -> BTreeSet<char> {
    let mut set = BTreeSet::new();
    let live = source.split("#[cfg(test)]").next().unwrap_or(source);
    for line in live.lines() {
        let code = match line.trim_start() {
            t if t.starts_with("//") => continue,
            _ => line,
        };
        // A trailing `//` comment; a URL inside a literal loses only its tail.
        let code = code.split(" // ").next().unwrap_or(code);
        set.extend(code.chars().filter(|c| !c.is_ascii()));
        let mut rest = code;
        while let Some(i) = rest.find("\\u{") {
            rest = &rest[i + 3..];
            if let Some(end) = rest.find('}') {
                if let Some(c) = u32::from_str_radix(&rest[..end], 16).ok().and_then(char::from_u32) {
                    if !c.is_ascii() {
                        set.insert(c);
                    }
                }
            }
        }
    }
    set
}

/// Mapped code points per font inside `SUBSET_RANGES`, as the generated files
/// have them today. The source families do not cover every code point of the
/// ranges (DM Mono has no arrows, DM Sans no check mark), so this pins what the
/// subset keeps: dropping a range or a glyph lowers the count and fails here.
const SUBSET_COUNTS: [(&str, usize); 5] = [
    ("DMSans-400", 325),
    ("DMSans-500", 325),
    ("DMSans-600", 325),
    ("DMMono-400", 316),
    ("DMMono-500", 316),
];

fn in_subset(map: &BTreeSet<u32>) -> usize {
    SUBSET_RANGES.iter().map(|&(lo, hi)| map.range(lo..=hi).count()).sum()
}

#[test]
fn every_font_keeps_its_latin_subset() {
    for ((name, bytes), (count_name, count)) in FONTS.iter().zip(SUBSET_COUNTS) {
        assert_eq!(*name, count_name);
        let map = cmap(bytes);
        // Plain text: ASCII and Latin-1 (user titles, file names, Indonesian).
        for cp in (0x20..=0x7E).chain(0xA0..=0xFF) {
            if cp != 0xAD {
                assert!(map.contains(&cp), "{name} lost U+{cp:04X}");
            }
        }
        assert_eq!(in_subset(&map), count, "{name}: glyph count in the subset changed");
    }
}

#[test]
fn the_fonts_have_the_symbols_the_ui_draws() {
    // Drawn from the sans: separators, signs, arrows, quotes, units.
    let sans = "\u{B7}\u{D7}\u{F7}\u{B0}\u{B1}\u{A7}\u{2013}\u{2014}\u{2026}\u{203A}\u{2192}\
                \u{2191}\u{2193}\u{2212}\u{2018}\u{2019}\u{201C}\u{201D}\u{2022}\u{20AC}\u{2122}%";
    // Drawn from the mono (terminal log, clipboard text, chips).
    let mono = "\u{B7}\u{D7}\u{2026}\u{203A}\u{2013}\u{2014}\u{2212}%";
    for (name, bytes) in FONTS {
        let map = cmap(bytes);
        let want = if name.starts_with("DMSans") { sans } else { mono };
        for c in want.chars() {
            assert!(map.contains(&(c as u32)), "{name} lacks U+{:04X} {c}", c as u32);
        }
    }
}

#[test]
fn every_character_the_sources_draw_is_covered() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    assert!(files.len() > 50, "found {} source files", files.len());
    // Only the sans is required here. The mono draws terminal and clipboard
    // text (its symbols are checked above); a literal the mono lacks falls
    // back to a system font like any user character does.
    let sans: Vec<BTreeSet<u32>> =
        FONTS.iter().filter(|(n, _)| n.starts_with("DMSans")).map(|(_, b)| cmap(b)).collect();
    let mut missing = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for c in drawn_chars(&text) {
            if NOT_DRAWN.contains(&c) || SYSTEM_FALLBACK.contains(&c) {
                continue;
            }
            if sans.iter().any(|map| !map.contains(&(c as u32))) {
                missing.push(format!("{c} U+{:04X} in {}", c as u32, file.display()));
            }
        }
    }
    assert!(missing.is_empty(), "uncovered glyphs:\n{}", missing.join("\n"));
}
