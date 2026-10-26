---
tags: [gotcha, theme, contrast]
date: 2026-10-26
---

# Light-mode secondary text is 0x786A5C, not the design's 0x7A6B5D

The design value 0x7A6B5D measures 4.46:1 on the light background 0xF4EEE6 and
fails the theme test requiring secondary vs bg >= 4.5. 0x786A5C gives 4.54:1.
A comment in `src/theme.rs` explains it. If the design palette is refreshed,
re-check contrast before pasting values.

Related: [[studio-palette-accent-aliases]]
