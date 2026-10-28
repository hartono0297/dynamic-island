---
tags: [decision, gestures, media, v3.2]
date: 2026-10-28
---

# Gestures on the island (v3.2 Run 8)

`src/gestures.rs` (pure: `Swipe`, `HWheel`, `Dir`); `App` feeds them from plain
window messages, no global hook (the plan's "hook + evaluator" was not
needed): `WM_LBUTTONDOWN/UP` for a swipe, `WM_MOUSEHWHEEL` for the sideways
wheel, `WM_MBUTTONUP` for the middle button.

- Count only while `App::gestures_active`: `config.gestures` (default on,
  hand-edited, no Settings row) **and** a media session **and** the view is
  `MediaCompact`, `MediaLyric` or `Panel(Media)`. Elsewhere nothing happens.
- Swipe: ≥ 56 logical px sideways (scaled by the window's dpi), at least
  twice as far across as up/down, within 700 ms. Left = next, right = prev. A
  swipe clears the pending click so it never opens the panel in Click mode.
- Sideways wheel: deltas add up to one notch (120); a flick fires once and
  the rest of the same scroll (touchpad momentum) is ignored for 500 ms.
  Right = next.
- Middle button over the drawn island = play/pause.
- All three go through `Event::Click(Hit::Prev/Next/PlayPause)`, so the
  existing media guard and the media log see them like button clicks.

The plan's "clipboard peek" already exists from v3.1: Settings → Clipboard →
"Disalin untuk semua salinan" (`copied_feedback_all`) shows the "Disalin"
notice with a one-line preview (`app_clip::copied_notice`); nothing added.

**Not verified live:** a real touchpad's horizontal scroll (does it reach the
island window?), and whether a swipe feels right on a touch screen.
