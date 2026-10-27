---
tags: [decision, system, timers, power]
date: 2026-10-28
---

# System page (Sistem) and its disk / RAM warnings

Run 4 of the v3.1 plan (keputusan-v3.1.html, decision 4a). Readings and
the warning logic live in `src/system.rs`; the page in
`panel_layout::system_layout` and `render_panel::paint_system`.

- **Readings run off the UI thread.** `Sampler` is a parked worker thread,
  started on the first request, that answers `SampleKind::Page` (CPU from
  two `GetSystemTimes` readings, RAM, fixed drives, top 3 processes from a
  ToolHelp snapshot, uptime) or `SampleKind::Warn` (RAM and drives only).
  `GetDiskFreeSpaceExW` can block on a spun-down disk, so it never runs on
  the UI thread. Only `DRIVE_FIXED` drives are read; removable and network
  drives are never woken.
- **CPU baseline.** Process and system times are kept between page samples.
  Without one, or with one older than 10 s, the worker takes a baseline and
  waits 300 ms, so a page that opens shows real numbers at once. Processes
  are summed per exe without case ("Chrome" is one row). A reused pid with
  another exe counts 0 %.
- **Page refresh.** `TIMER_SYSTEM_PAGE` is a 2 s one-shot, armed when a
  sample arrives and only while `system_sample_wanted` (the panel shows
  `Page::System`). `sync_system_page` (called from `refresh`) kills it when
  the page or panel goes. A sample that arrives late re-arms nothing.
- **Background warnings.** `TIMER_SYSTEM_WARN` is a one-shot, every 5 min,
  re-armed when its reading comes back. It is sooner only to confirm RAM's
  minute above the limit. A drain that holds only system readings does not
  refresh or redraw the island (`drain_refreshes`) unless a notice was
  raised.
- **Once per crossing** (`WarnMonitor`, like the battery). Disk: per drive,
  as soon as it drops under `disk_warn_gb`, re-armed once it is 1 GB back
  above (hysteresis). RAM: only after staying above `ram_warn_percent` for
  60 s, re-armed at or under the limit. Changing a limit does not re-raise
  a crossing that is already alerted. The notice is `NoticeKind::SystemLow`
  with Bata's Sad ("lesu") face, an ochre `Activity` badge when the robot is
  off, and "Buka". A click opens the panel on the System page.
- **Clicks.** A drive meter opens Explorer on its root
  (`shortcuts::Action::Drive`, ShellExecuteW). A process row opens Task
  Manager. Both collapse the panel first, as the shortcut tiles do.
- **"Bersihkan"** stays the dimmed ghost button with a "segera" tag and
  registers no region until the cleaner exists (`CLEANER_READY`, Runs 5–6).
  Nothing on this page deletes anything.
- **Config.** `disk_warn_gb` (5–100 by 5, default 10) and
  `ram_warn_percent` (70–98, default 90). Both are serde defaults, so an
  older config still loads. Like the battery threshold, the file owns them
  (`merge_owned`). Settings → Umum shows them in a second card, so General
  now scrolls like [[settings-appearance-scroll]].
- **Order.** `Page::System` sits before CLI for now. Run 9 sets the final
  order and the page settings.

Demo: `system`, `system-low` and `bata-sistem` use fixed figures
([[demo-data-time-of-day]]). Shots `system` and `system-warning` are in
[[screenshot-capture-script]].

Related: [[bata-idle-power-budget]], [[bata-robot-expressions]],
[[studio-tabular-figures-word-gaps]], [[media-click-guards]]

Listed in [[index]].
