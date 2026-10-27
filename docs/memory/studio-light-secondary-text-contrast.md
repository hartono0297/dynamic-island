---
tags: [gotcha, theme, contrast]
date: 2026-10-27
---

# Light-mode secondary text is 0x786A5C, not the design's 0x7A6B5D

The design value 0x7A6B5D measures 4.46:1 on the light background 0xF4EEE6 and
fails the theme test requiring secondary vs bg >= 4.5. 0x786A5C gives 4.54:1.
A comment in `src/theme.rs` explains it.

Rule: the theme test is the authority, not the mock-up. If the design palette
is refreshed, run the tests before pasting values, and nudge a failing colour
the smallest step that passes rather than lowering the threshold. Secondary
text carries real content (the reminder summary is drawn in it), so it is
not decoration that may go faint.

Related: [[studio-palette-accent-aliases]], [[studio-reminder-editor]]

Part of [[v3-studio-overview]].
