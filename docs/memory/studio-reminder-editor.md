---
tags: [decision, editor, reminder, i18n, convention, data]
date: 2026-10-27
---

# Reminder editor (Run 6a, 6b): summary, chips, days, quick times

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
- **Next occurrence: one rule.** `reminders::next_repeat` decides when a
  repeating reminder rings next (Daily: every day; Weekly: the weekdays in
  `days`, or the start's weekday when empty). The store calls it through
  `advance_in`/`first_due_at` with instants in a time zone (`at_local`, so
  DST gaps and folds behave as before); `i18n::next_occurrence` (the
  summary's "berikutnya …") calls it with naive local dates. Change the
  rule there only.
- **`Reminder::days`** (Run 6b): `Vec<Weekday>`, `#[serde(default,
  skip_serializing_if = "Vec::is_empty")]`, chrono's form `"Mon"`.
  `put` keeps it sorted Monday first, empty for Once/Daily, and empty when
  it is just the due date's weekday, so old files load and save unchanged
  (tested byte for byte). The editor fills an empty weekly list with the
  date's weekday (`app::reminder_editor_kind`); a single day follows the
  date when it changes, picked days stay. The last day chip cannot be
  turned off; switching to Mingguan with none on turns on the date's.
- **Quick-time pills** (`editor_layout::QuickTime::target`): +30 min and
  +1 h round up to the whole minute (never "29 menit lagi"); Nanti malam is
  20:00 today, tomorrow from 20:00 on; Besok pagi is 08:00 tomorrow. They
  keep the repeat and clear a TimePassed error. The pill whose target is the
  date and time set is drawn on.
- **Placeholder summary**: a new reminder untouched since it opened
  (`Mode::Reminder::fresh`) shows "Pilih tanggal dan jam, atau pakai
  pintasan." instead of its summary, as in design state 1.
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
- **Run 6b is done**: rows in design order are title, date + time, quick
  pills (wrap if too wide), repeat segments, day chips (only for Mingguan;
  the window grows by 42 px through `refresh`), summary. Tab follows that
  order.
- Demo: `--demo-editor reminder|reminder-new|reminder-weekly|reminder-past`
  (debug build only), with `DI_POPOVER=calendar|time`. `reminder-weekly` is
  Sen/Rab/Jum 09.00 from this week's Monday (design state 3).

Related: [[studio-editor-frame]], [[studio-tabular-figures-word-gaps]],
[[studio-light-secondary-text-contrast]]

Listed in [[index]].
