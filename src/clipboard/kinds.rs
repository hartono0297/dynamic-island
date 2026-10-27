//! What a copied text is (link, code, colour or plain text), and which
//! clipboard contents the history must never keep. Pure: tested without the
//! clipboard.

/// The kinds the Clipboard page tells apart. Text kinds come from
/// `detect_kind`; images and file lists from the clipboard format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipKind {
    Text,
    Link,
    Code,
    /// A hex colour: `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`.
    Color,
    /// A bitmap: thumbnail plus width × height (`store::ClipImage`).
    Image,
    /// Files copied in Explorer: their paths.
    Files,
}

impl ClipKind {
    pub const ALL: [ClipKind; 6] = [
        ClipKind::Text,
        ClipKind::Link,
        ClipKind::Code,
        ClipKind::Color,
        ClipKind::Image,
        ClipKind::Files,
    ];

    /// Stable tag for the history file.
    pub fn tag(self) -> u8 {
        match self {
            ClipKind::Text => 0,
            ClipKind::Link => 1,
            ClipKind::Code => 2,
            ClipKind::Color => 3,
            ClipKind::Image => 4,
            ClipKind::Files => 5,
        }
    }

    pub fn from_tag(tag: u8) -> Option<ClipKind> {
        ClipKind::ALL.into_iter().find(|k| k.tag() == tag)
    }

    /// Values drawn in DM Mono (the design's link and code cards).
    pub fn mono(self) -> bool {
        matches!(self, ClipKind::Link | ClipKind::Code)
    }
}

/// Shell and tool commands that start a one-line command. Words that also
/// start ordinary sentences ("go", "start", "copy", "make") are left out.
const COMMANDS: &[&str] = &[
    "cargo", "git", "npm", "npx", "pnpm", "yarn", "pip", "python", "py", "rustup", "rustc",
    "docker", "kubectl", "cd", "ls", "winget", "curl", "wget", "ssh", "scp", "sudo", "choco",
    "scoop", "gh", "dotnet", "cmake", "mkdir", "rm", "grep", "rg", "powershell", "pwsh",
    "taskkill", "ipconfig", "netsh", "rtk", "uv", "deno",
];

/// Marks that only code has, even on one line.
const CODE_MARKS: &[&str] = &[
    "::", "=>", "->", "&&", "||", "();", "){", "};", "</", "/>", "!=", "==", "+=", "fn ",
];

/// Text, link, code or colour, from the text alone. A link is one token:
/// `http(s)://…`, `www.…`, or a bare domain followed by a path
/// (`github.com/tw93/mole`). Prose (spaces, a period or comma between
/// words) never is one. Code is a known command with arguments, a line with
/// flags or code marks, or several lines with braces, semicolons or
/// indentation.
pub fn detect_kind(text: &str) -> ClipKind {
    let t = text.trim();
    if t.is_empty() {
        return ClipKind::Text;
    }
    if is_hex_color(t) {
        return ClipKind::Color;
    }
    if is_link(t) {
        return ClipKind::Link;
    }
    if is_code(text) {
        return ClipKind::Code;
    }
    ClipKind::Text
}

/// `#E2683C`, `#fff`, `#RRGGBBAA`.
pub fn is_hex_color(t: &str) -> bool {
    let Some(hex) = t.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// The colour of a hex value as 0xRRGGBB (alpha dropped).
pub fn hex_rgb(t: &str) -> Option<u32> {
    let t = t.trim();
    if !is_hex_color(t) {
        return None;
    }
    let hex = &t[1..];
    let digit = |i: usize| u32::from_str_radix(&hex[i..i + 1], 16).ok();
    match hex.len() {
        3 | 4 => {
            let (r, g, b) = (digit(0)?, digit(1)?, digit(2)?);
            Some((r * 17) << 16 | (g * 17) << 8 | b * 17)
        }
        _ => u32::from_str_radix(&hex[..6], 16).ok(),
    }
}

fn is_link(t: &str) -> bool {
    if t.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    for scheme in ["http://", "https://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            return is_host(rest.split(['/', '?', '#']).next().unwrap_or(""));
        }
    }
    if let Some(rest) = lower.strip_prefix("www.") {
        return is_host(&format!("www.{}", rest.split(['/', '?', '#']).next().unwrap_or("")));
    }
    // A bare domain counts only with a path after it: "notes.txt" or "v1.2"
    // alone are not links.
    match lower.split_once('/') {
        Some((host, path)) => is_host(host) && !path.is_empty(),
        None => false,
    }
}

/// `github.com`, `sub.example.co.id:8080`: dotted labels of letters, digits
/// and hyphens, the last one two or more letters.
fn is_host(host: &str) -> bool {
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |l: &&str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let tld = labels[labels.len() - 1];
    labels.iter().all(label_ok) && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_code(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() >= 2 {
        return lines.iter().any(|l| {
            let t = l.trim_end();
            t.ends_with(';')
                || t.ends_with('{')
                || t.ends_with('}')
                || t.trim_start().starts_with('}')
                || l.starts_with("  ")
                || l.starts_with('\t')
                || CODE_MARKS.iter().any(|m| t.contains(m))
        });
    }
    let line = text.trim();
    let mut words = line.split_whitespace();
    let first = words.next().unwrap_or("").to_ascii_lowercase();
    let more = words.next().is_some();
    if more && COMMANDS.contains(&first.as_str()) {
        return true;
    }
    if line.starts_with("$ ") || line.starts_with("> ") && more {
        return true;
    }
    let has_flag = line.split_whitespace().any(|w| {
        let flag = w.strip_prefix("--").or_else(|| w.strip_prefix('-'));
        flag.is_some_and(|f| f.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
    });
    if has_flag {
        return true;
    }
    if CODE_MARKS.iter().any(|m| line.contains(m)) || line.ends_with(';') || line.ends_with('{') {
        return true;
    }
    // `print(x)`, `foo.bar(1, 2)`: a call with nothing between name and `(`.
    if let Some(open) = line.find('(') {
        let before = line[..open].chars().last();
        let call = before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !line[..open].contains(' ')
            && line.ends_with(')');
        if call {
            return true;
        }
    }
    false
}

// ---- Privacy ----

/// How an excluded format marks content the history must skip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The format being on the clipboard at all.
    Present,
    /// A DWORD format whose value is 0 (or that cannot be read).
    ZeroDword,
}

/// Formats password managers and other apps set so clipboard monitors leave
/// their content alone (Windows' own history honours the same ones).
pub const EXCLUDED_FORMATS: [(&str, Rule); 4] = [
    ("ExcludeClipboardContentFromMonitorProcessing", Rule::Present),
    ("Clipboard Viewer Ignore", Rule::Present),
    ("CanIncludeInClipboardHistory", Rule::ZeroDword),
    ("CanUploadToCloudClipboard", Rule::ZeroDword),
];

/// The rule for a registered format's name, if it is an excluded one.
pub fn excluded_rule(name: &str) -> Option<Rule> {
    EXCLUDED_FORMATS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, r)| *r)
}

/// The formats on the clipboard (registered name, and the DWORD value of a
/// `ZeroDword` one when it could be read) say: store nothing. A present
/// `Present` format skips; a `ZeroDword` one skips when 0 or unreadable.
pub fn should_skip(formats: &[(String, Option<u32>)]) -> bool {
    formats.iter().any(|(name, dword)| match excluded_rule(name) {
        Some(Rule::Present) => true,
        Some(Rule::ZeroDword) => dword.is_none_or(|v| v == 0),
        None => false,
    })
}

/// The change came from the island itself: the sequence number it recorded
/// after its own write, or the island owning the clipboard.
pub fn is_own_change(sequence: u32, own_sequence: Option<u32>, owner_is_island: bool) -> bool {
    owner_is_island || own_sequence == Some(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_examples() {
        assert_eq!(detect_kind("github.com/tw93/mole/tree/windows"), ClipKind::Link);
        assert_eq!(detect_kind("cargo test --locked -- --nocapture"), ClipKind::Code);
        assert_eq!(detect_kind("#E2683C"), ClipKind::Color);
        assert_eq!(detect_kind("#fff"), ClipKind::Color);
        assert_eq!(detect_kind("#E2683C80"), ClipKind::Color);
        assert_eq!(
            detect_kind("Jl. Kemang Raya No. 12, Jakarta Selatan"),
            ClipKind::Text
        );
        assert_eq!(
            detect_kind("Denah lantai 2 sudah saya revisi, tolong dicek bagian tangga"),
            ClipKind::Text
        );
    }

    #[test]
    fn links() {
        for s in [
            "https://example.com",
            "http://localhost.dev:8080/x",
            "www.kompas.com",
            "  https://github.com/a/b?x=1  ",
            "docs.rs/windows/0.62",
        ] {
            assert_eq!(detect_kind(s), ClipKind::Link, "{s}");
        }
        for s in [
            "github.com",
            "notes.txt",
            "v1.2",
            "Halo. Apa kabar",
            "a, b.c/d",
            "e.g./x",
            "3.14/2",
        ] {
            assert_ne!(detect_kind(s), ClipKind::Link, "{s}");
        }
    }

    #[test]
    fn prose_with_a_period_or_comma_is_text() {
        for s in [
            "Rapat jam 3. Jangan lupa bawa laptop.",
            "Oke, nanti saya kirim",
            "See you at 5, ok?",
            "Mr. Smith",
            "Total - 12 item",
        ] {
            assert_eq!(detect_kind(s), ClipKind::Text, "{s}");
        }
    }

    #[test]
    fn code() {
        let multi = "fn main() {\n    println!(\"hi\");\n}";
        assert_eq!(detect_kind(multi), ClipKind::Code);
        let js = "const a = 1;\nconst b = 2;";
        assert_eq!(detect_kind(js), ClipKind::Code);
        let indented = "def f():\n    return 1";
        assert_eq!(detect_kind(indented), ClipKind::Code);
        assert_eq!(detect_kind("git status"), ClipKind::Code);
        assert_eq!(detect_kind("print(x)"), ClipKind::Code);
        assert_eq!(detect_kind("let x = a::b();"), ClipKind::Code);
        // Two lines of prose are text.
        assert_eq!(detect_kind("Halo semua\nSampai besok"), ClipKind::Text);
        assert_eq!(detect_kind("cargo"), ClipKind::Text);
    }

    #[test]
    fn colors_and_rgb() {
        assert!(!is_hex_color("#ggg"));
        assert!(!is_hex_color("#12345"));
        assert!(!is_hex_color("E2683C"));
        assert_eq!(hex_rgb("#E2683C"), Some(0xE2683C));
        assert_eq!(hex_rgb("#fff"), Some(0xFFFFFF));
        assert_eq!(hex_rgb("#11223380"), Some(0x112233));
        assert_eq!(hex_rgb("nope"), None);
    }

    #[test]
    fn tags_round_trip() {
        for k in ClipKind::ALL {
            assert_eq!(ClipKind::from_tag(k.tag()), Some(k));
        }
        assert_eq!(ClipKind::from_tag(99), None);
    }

    fn set(formats: &[(&str, Option<u32>)]) -> Vec<(String, Option<u32>)> {
        formats.iter().map(|(n, v)| (n.to_string(), *v)).collect()
    }

    #[test]
    fn excluded_formats_list_their_rule() {
        assert_eq!(EXCLUDED_FORMATS.len(), 4);
        assert_eq!(
            excluded_rule("ExcludeClipboardContentFromMonitorProcessing"),
            Some(Rule::Present)
        );
        assert_eq!(excluded_rule("Clipboard Viewer Ignore"), Some(Rule::Present));
        assert_eq!(
            excluded_rule("CanIncludeInClipboardHistory"),
            Some(Rule::ZeroDword)
        );
        assert_eq!(
            excluded_rule("canuploadtocloudclipboard"),
            Some(Rule::ZeroDword)
        );
        assert_eq!(excluded_rule("HTML Format"), None);
    }

    #[test]
    fn password_managers_are_skipped() {
        // KeePass 2: all four markers.
        let keepass = set(&[
            ("Clipboard Viewer Ignore", None),
            ("ExcludeClipboardContentFromMonitorProcessing", None),
            ("CanIncludeInClipboardHistory", Some(0)),
            ("CanUploadToCloudClipboard", Some(0)),
        ]);
        assert!(should_skip(&keepass));
        // KeePassXC: history and cloud off, plus the monitor exclusion.
        let keepassxc = set(&[
            ("CanIncludeInClipboardHistory", Some(0)),
            ("CanUploadToCloudClipboard", Some(0)),
            ("ExcludeClipboardContentFromMonitorProcessing", Some(1)),
        ]);
        assert!(should_skip(&keepassxc));
        // 1Password: the monitor exclusion only.
        assert!(should_skip(&set(&[(
            "ExcludeClipboardContentFromMonitorProcessing",
            None
        )])));
        // Bitwarden (Electron): history flag 0 only.
        assert!(should_skip(&set(&[("CanIncludeInClipboardHistory", Some(0))])));
        // A marker that cannot be read counts as 0.
        assert!(should_skip(&set(&[("CanUploadToCloudClipboard", None)])));
    }

    #[test]
    fn normal_copies_are_kept() {
        assert!(!should_skip(&[]));
        let browser = set(&[
            ("HTML Format", None),
            ("Chromium internal source URL", None),
            ("text/x-moz-url", None),
        ]);
        assert!(!should_skip(&browser));
        // Allowed explicitly: 1 keeps it.
        let allowed = set(&[
            ("CanIncludeInClipboardHistory", Some(1)),
            ("CanUploadToCloudClipboard", Some(1)),
        ]);
        assert!(!should_skip(&allowed));
    }

    #[test]
    fn own_changes_are_recognised() {
        assert!(is_own_change(7, Some(7), false));
        assert!(is_own_change(9, Some(7), true));
        assert!(!is_own_change(8, Some(7), false));
        assert!(!is_own_change(8, None, false));
    }
}
