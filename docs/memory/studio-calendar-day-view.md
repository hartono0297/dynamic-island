---
tags: [decision, calendar, layout]
date: 2026-10-27
---

# Calendar layout: title width, day paging, "now" line

- The calendar title width is measured by the renderer and passed into
  `calendar_layout(title_w, now, ...)`, so the prev/next arrows sit right after
  the title in any language.
- The day view is an hour timeline. `day_page` steps through hour windows and
  pairs of all-day events; "+N lagi" counts what later pages add, and the last
  page shows "Kembali ke atas".
- The "now" line uses `Local::now()` inside the renderer, so it is not testable
  from layout alone. Demo scenes must stay on today, see [[demo-data-time-of-day]].
- The media progress bar and the 5-bar equalizer, once open items here, are
  done: see [[studio-media-progress-timer]].
- The month grid is shared with the editors' date picker, see
  [[studio-editor-frame]].

Part of [[v3-studio-overview]]. Listed in [[index]].
