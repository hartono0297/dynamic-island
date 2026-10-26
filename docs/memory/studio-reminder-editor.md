---
tags: [decision, editor, reminder, i18n, convention]
date: 2026-10-27
---

# Reminder editor (Run 6a): summary, chips, dimmed past

The reminder editor follows `docs/design/pengingat-cli.html` pages 1-2 on the
shared [[studio-editor-frame]]. What to know before changing it:

- **Text lives in `i18n.rs`.** `reminder_date_chip` gives the date chip ("Hari
  ini, 26 Okt", or "Mulai besok" for a repeating one). `reminder_summary`
  returns a `ReminderSummary { lead, bold, tail, passed }`. The editor draws
  `bold` in `p.text` at 11 px weight 500 (`Text::SmallStrong`) and the rest in
  `p.secondary`, or all of it in `p.error` when a one-off has `passed`.
- **16.30 vs 16:30.** Times come from `i18n::time_label`: ID writes "16.30",
  EN "16:30". The design mock-ups write 16:30 in both. Tests assert the app's
  convention, not the mock-up's.
- **Next occurrence** for Daily/Weekly is `i18n::next_occurrence`. It is a
  naive copy of `reminders::first_due_at`/`advance_in` (date + k·step, first
  one after now), so `reminders.rs` stays untouched. If the store's rule
  changes, change this one too.
- **Dimmed past days.** `month_grid::DayState::past` (default false) draws a
  day like another month's day but keeps it clickable. Only the reminder's
  popover sets it, via `Kind::Date { dim_past: true }`. The pick's fill and
  today's ring still win. The panel and the event picker never set it.
- **Times of day.** `Kind::Time { periods: true }` (reminder only) puts
  pagi/siang/sore/malam beside 08:00/12:00/17:00/20:00 (`period_label`). The
  event lists keep `periods: false` and their durations.
- **Simpan** is off (drawn at 0.4) until the text has a non-blank character
  (`Mode::can_save`). `save_next` refuses an empty reminder for the button,
  Enter and Ctrl+Enter alike. `validate_reminder` is unchanged: a past one-off
  gets `TimePassed`, which shows in the summary's slot (`Layout::error`
  replaces `Layout::summary`, so there is no extra row).
- **Hapus** is the design's `.btn.dng` for reminders only: no fill until it is
  hovered, error colour, 4 px left padding (`editor_layout::DANGER_PAD`). The
  note keeps its pill.
- Run 6b (not built yet): the quick-time pills, the weekday chips and
  multi-day weekly. They need a data change in `reminders.json`.
- Demo: `--demo-editor reminder|reminder-new|reminder-weekly|reminder-past`
  (debug build only), with `DI_POPOVER=calendar|time`.

Related: [[studio-editor-frame]], [[studio-tabular-figures-word-gaps]],
[[studio-light-secondary-text-contrast]]

Listed in [[index]].
