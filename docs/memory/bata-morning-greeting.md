---
tags: [decision, robot, greeting, config]
date: 2026-10-27
---

# Morning greeting: once a day, before noon

> Since v3.8 the greeting follows the time of day (five periods, once per
> period per day) and the noon limit below is gone; see [[greeting-periods]].
> This note keeps the trigger and gotcha details, which still hold.

`robot::should_greet` was `(last, today, now_time, settings)`: the robot and
"Sapaan pagi" on, `last_greeting != today`, and before 12:00. It now takes the
last `GreetMark` and `now` and returns the period to greet in.

When it is tried: startup (after the first calendar snapshot arrives, the
cache the sync thread sends at once, so today's events can be counted),
a wake by the user (`PBT_APMRESUMESUSPEND`) and session unlock
(`WM_WTSSESSION_CHANGE` + `WTS_SESSION_UNLOCK`, via
`WTSRegisterSessionNotification`, feature `Win32_System_RemoteDesktop`).
Not over a fullscreen app: then nothing is recorded and the next chance
tries again (the reducer also drops a `Greeting` notice in fullscreen).

Gotcha: Windows locks on wake by default, and the resume message arrives
while the secure desktop shows. Greeting there would use up the day
unseen, and the unlock would then skip. So `greet_may_try(trigger, locked)`
(app.rs) skips startup and wake on a locked session; the session state
comes from `WTSQuerySessionInformationW(WTSSessionInfoEx).SessionFlags`,
falling back to the last lock/unlock message. `PBT_APMRESUMEAUTOMATIC`
alone never greets: it also comes on unattended wakes (maintenance,
Modern Standby). The unlock message always may greet.

It is `NoticeKind::Greeting { period }`: the cat's face by period (Wave 40 in
the morning), "Selamat pagi" over "X acara dan Y pengingat hari ini"
(`i18n::greeting_body`, EN pluralized; a free day says "Hari ini belum ada
acara"). Y counts today's undone reminders, overdue ones included. It expires
after 4 s through the normal notice deadline (`GREETING_DURATION`), no timer
loop.

Storage: `config.last_greeting` (v3.8: `"2026-10-31 evening"`; the old
`"2026-10-27"` reads as that date's Morning, an unreadable value as none) is
written when the greeting shows. It is one of the island's own fields in
`merge_owned`, so a save never loses it.

Related: [[bata-robot-expressions]], [[demo-data-time-of-day]], [[greeting-periods]]

Part of [[v3-studio-overview]]. Listed in [[index]].
