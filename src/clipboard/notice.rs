//! The copy preview in the notch (`NoticeKind::ClipPreview`): which copies
//! get one (`clip_notice_wanted`, `NoticeGate`) and what its text lines say
//! (`clip_notice_lines`). Pure: tested with fake contexts and strings, never
//! the real clipboard (docs/memory/clipboard-notice.md, clipboard-privacy.md).

use std::time::{Duration, Instant};

use super::store::{Ingested, preview};

/// The longest a preview line gets before the renderer fits it to the box.
pub const PREVIEW_CHARS: usize = 120;

/// The same entry is not announced again within this time (a program that
/// writes the clipboard in a loop, a double Ctrl+C).
pub const CLIP_NOTICE_REPEAT: Duration = Duration::from_secs(4);

/// What decides whether a copy that just went into the history is announced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoticeCtx {
    /// "Notifikasi salinan".
    pub setting: bool,
    /// "Riwayat clipboard": the notice needs an entry to name.
    pub history: bool,
    /// What `ingest` did with the change.
    pub outcome: Ingested,
    /// The island wrote this itself (a card clicked, a search result).
    /// `win::read` answers `Read::Own` before anything is read, so a real
    /// call passes false; the rule is stated here so one test holds it.
    pub own: bool,
    /// The image went to the screenshot preview instead.
    pub shot: bool,
    /// The panel shows the Clipboard page (the new card appears there).
    pub clip_page_open: bool,
    /// A fullscreen app is up.
    pub fullscreen: bool,
}

/// Whether the copy that just went into the history gets a preview (the
/// repeat rule is `NoticeGate`'s). A skipped (password manager) or
/// kept-nothing change never does.
pub fn clip_notice_wanted(ctx: &NoticeCtx) -> bool {
    if !ctx.setting || !ctx.history || ctx.own || ctx.shot || ctx.clip_page_open || ctx.fullscreen {
        return false;
    }
    match ctx.outcome {
        Ingested::Skipped | Ingested::Nothing => false,
        Ingested::Added { .. } | Ingested::Bumped { .. } => true,
    }
}

/// Remembers the entry announced last and when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoticeGate {
    pub last: Option<(u64, Instant)>,
}

impl NoticeGate {
    /// Whether entry `id` may be announced at `now`: not when the same entry
    /// was announced less than `CLIP_NOTICE_REPEAT` ago. A yes is recorded.
    pub fn pass(&mut self, id: u64, now: Instant) -> bool {
        if let Some((last, at)) = self.last
            && last == id
            && now.saturating_duration_since(at) < CLIP_NOTICE_REPEAT
        {
            return false;
        }
        self.last = Some((id, now));
        true
    }
}

/// The notice's two lines for copied `text`: the first non-empty line and,
/// when there is one, the second. Each is trimmed, its whitespace runs
/// (tabs, a CR) become one space, and it is cut to `PREVIEW_CHARS`
/// characters with "…" (never inside a character). The renderer trims to
/// the box.
pub fn clip_notice_lines(text: &str) -> (String, Option<String>) {
    let mut lines = text
        .split(['\r', '\n'])
        .map(|line| preview(line, PREVIEW_CHARS))
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or_default();
    (first, lines.next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NoticeCtx {
        NoticeCtx {
            setting: true,
            history: true,
            outcome: Ingested::Added {
                id: 7,
                image: false,
            },
            own: false,
            shot: false,
            clip_page_open: false,
            fullscreen: false,
        }
    }

    #[test]
    fn an_external_copy_is_announced_by_its_entry() {
        assert!(clip_notice_wanted(&ctx()));
        let again = NoticeCtx {
            outcome: Ingested::Bumped { id: 3 },
            ..ctx()
        };
        assert!(clip_notice_wanted(&again), "same content, one id");
    }

    #[test]
    fn every_skip_rule_says_no() {
        let no = |c: NoticeCtx| assert!(!clip_notice_wanted(&c), "{c:?}");
        // Own copy.
        no(NoticeCtx { own: true, ..ctx() });
        // Private content and nothing kept.
        no(NoticeCtx {
            outcome: Ingested::Skipped,
            ..ctx()
        });
        no(NoticeCtx {
            outcome: Ingested::Nothing,
            ..ctx()
        });
        // The screenshot preview took the image.
        no(NoticeCtx {
            shot: true,
            ..ctx()
        });
        // The Clipboard page is on screen.
        no(NoticeCtx {
            clip_page_open: true,
            ..ctx()
        });
        // A fullscreen app.
        no(NoticeCtx {
            fullscreen: true,
            ..ctx()
        });
        // The setting off, or the history off (no entry to name).
        no(NoticeCtx {
            setting: false,
            ..ctx()
        });
        no(NoticeCtx {
            history: false,
            ..ctx()
        });
    }

    #[test]
    fn the_same_entry_waits_four_seconds_another_does_not() {
        let t0 = Instant::now();
        let mut gate = NoticeGate::default();
        assert!(gate.pass(1, t0));
        // The same entry again, inside the window: no.
        assert!(!gate.pass(1, t0 + Duration::from_millis(3999)));
        // Another entry is not held back (and becomes the one remembered).
        assert!(gate.pass(2, t0 + Duration::from_secs(1)));
        assert!(gate.pass(1, t0 + Duration::from_secs(2)));
        // After the window the same entry may be announced again.
        assert!(!gate.pass(1, t0 + Duration::from_secs(5)));
        assert!(gate.pass(1, t0 + Duration::from_secs(6)));
    }

    #[test]
    fn a_refused_repeat_does_not_extend_the_wait() {
        let t0 = Instant::now();
        let mut gate = NoticeGate::default();
        assert!(gate.pass(1, t0));
        assert!(!gate.pass(1, t0 + Duration::from_secs(3)));
        // Counted from the announcement, not from the refused try.
        assert!(gate.pass(1, t0 + CLIP_NOTICE_REPEAT));
    }

    fn lines(text: &str) -> (String, Option<String>) {
        clip_notice_lines(text)
    }

    #[test]
    fn two_lines_title_then_body() {
        assert_eq!(
            lines("Denah lantai 2\nTolong dicek tangga\nbaris ketiga"),
            ("Denah lantai 2".into(), Some("Tolong dicek tangga".into()))
        );
    }

    #[test]
    fn blank_leading_lines_and_blank_gaps_are_skipped() {
        assert_eq!(
            lines("\n\n   \n  halo  \n\n\t\n dunia \n"),
            ("halo".into(), Some("dunia".into()))
        );
    }

    #[test]
    fn crlf_and_a_bare_cr_split_lines() {
        assert_eq!(
            lines("satu\r\ndua\r\ntiga"),
            ("satu".into(), Some("dua".into()))
        );
        assert_eq!(lines("satu\rdua"), ("satu".into(), Some("dua".into())));
    }

    #[test]
    fn tabs_and_runs_of_spaces_become_one_space() {
        assert_eq!(
            lines("a\t\tb   c\u{a0}d"),
            ("a b c d".into(), None),
            "tab, spaces, no-break space"
        );
    }

    #[test]
    fn a_single_line_has_no_body() {
        assert_eq!(lines("cuma satu baris"), ("cuma satu baris".into(), None));
        assert_eq!(lines("satu\n   \n"), ("satu".into(), None));
    }

    #[test]
    fn a_very_long_line_is_cut_with_an_ellipsis() {
        let long = "x".repeat(500);
        let (title, body) = lines(&format!("{long}\n{long}"));
        assert_eq!(title.chars().count(), PREVIEW_CHARS + 1);
        assert!(title.ends_with('…'));
        assert_eq!(body.as_deref(), Some(title.as_str()));
        // Exactly at the limit nothing is cut.
        let fit = "y".repeat(PREVIEW_CHARS);
        assert_eq!(lines(&fit), (fit.clone(), None));
    }

    #[test]
    fn multibyte_and_emoji_are_never_split() {
        // Two-byte, three-byte and four-byte characters over the limit.
        for unit in ["é", "日", "😀", "👩\u{200d}💻"] {
            let text = unit.repeat(PREVIEW_CHARS + 30);
            let (title, _) = lines(&text);
            assert!(title.ends_with('…'), "{unit}");
            assert_eq!(title.chars().count(), PREVIEW_CHARS + 1, "{unit}");
            assert!(title.is_char_boundary(title.len()));
        }
        assert_eq!(
            lines("😀 halo\n日本語"),
            ("😀 halo".into(), Some("日本語".into()))
        );
    }

    #[test]
    fn nothing_but_blanks_is_empty() {
        assert_eq!(lines(""), (String::new(), None));
        assert_eq!(lines(" \r\n\t\n"), (String::new(), None));
    }
}
