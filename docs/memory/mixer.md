---
tags: [decision, mixer, audio, v3.2]
date: 2026-10-28
---

# Mixer page (v3.2 Run 9)

`Page::Mixer`, right after Musik (`Page::ALL` is ten pages; a saved
`panel_pages` without "Mixer" gets it back after Media like any new page).

- **`src/audio.rs`**: one worker thread (`audio-mixer`) owns COM
  (`CoInitializeEx` MTA). Requests: `Scan`, `Level(Target, f32)`,
  `Mute(Target, bool)`; the reply is `Event::Audio(Option<Vec<MixerRow>>)`
  (`None` = no output device). A batch waiting in the queue is cut by
  `coalesce` to the last level and mute per target and one scan, so a drag
  never queues up. Pure and tested: `clamp_level`, `percent`,
  `slider_level`, `step_level` (5 % grid), `exe_key`, `rows`, `coalesce`.
- **Rows**: master (default render endpoint, `IAudioEndpointVolume`) first,
  then up to `MAX_APPS` (4) programs, active first then by name, then the
  system sounds only while they play. Sessions come from
  `IAudioSessionManager2`; the name is the exe stem from
  `QueryFullProcessImageNameW`. Sessions are deduped by process id **and**
  merged by program: Chrome's several processes are one "Chrome" row and a
  level/mute is applied to every session with that exe (`Target::App(key)`).
  `IsSystemSoundsSession` is S_OK (0) for yes, S_FALSE (1) for no: compare
  `.0 == 0`, not `is_ok()`.
- **Interaction** (reducer stays pure): `Hit::MixerSlider(i)` / `MixerMute(i)`.
  The press on a slider is handled by `App::mixer_press` (it needs the pointer
  x; `renderer.mixer_level_at` maps it onto the track), which sends
  `Event::MixerLevel(i, level)`; the reducer turns it into
  `Effect::MixerLevel` only while the Mixer page is open. A drag follows on
  `WM_MOUSEMOVE` while the left button is down (`GetAsyncKeyState`); release
  or leaving the page ends it, and readings do not overwrite the rows during
  a drag. The wheel over a row (`mixer_row_under`) steps ±5 %. A click on a
  speaker is `Effect::MixerMute`. `App` shows the change at once and sends it
  to the thread.
- **Sampling**: `TIMER_MIXER` (26, 1 s) runs only while the page shows
  (`sync_mixer`); nothing in the background.
- Demo scene `mixer` uses made-up rows (`demo::demo_mixer`), never the
  machine's audio; screenshot name `mixer`.

**Not verified live:** real Core Audio output (master level, per-app
sessions, mute) on a PC with sound playing, how a drag feels, process names
for elevated or protected apps (those rows are left out), and a USB or
Bluetooth device switching while the page is open (the next scan picks the
new default endpoint).

Related: [[gestures]], [[shortcut-tiles]].
