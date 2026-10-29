---
tags: [memory, index]
---

# Project memory

One note per decision, gotcha or convention. Link each new note here.

## Overview

- [[v3-studio-overview]]: the v3 Studio redesign, one line per decision

## Decisions

- [[studio-palette-accent-aliases]]
- [[studio-font-sources]]
- [[studio-icons]]
- [[studio-calendar-day-view]]
- [[studio-media-progress-timer]]
- [[studio-editor-frame]]
- [[studio-reminder-editor]]
- [[bata-robot-expressions]]
- [[bata-idle-power-budget]]
- [[bata-morning-greeting]]
- [[lyric-notch-width]]
- [[settings-appearance-scroll]]
- [[media-click-guards]]
- [[media-diagnosis-log]]
- [[cli-explorer-folder]]
- [[system-page]]
- [[cleaner]]
- [[mole-mode]]
- [[clipboard-history]]
- [[screenshot-preview]]
- [[screenshot-snap]]: own region capture that snaps to windows and boxes, the editor after it, UIA budget, own-copy skip
- [[panel-pages]]
- [[vale-rename]]
- [[google-built-in-client]]
- [[github-pages]]: site/ goes to GitHub Pages through the Actions source (Pages cannot serve site/ directly)
- [[v3-1-handoff]]: where v3.1 stands, how to continue on another PC
- [[open-mode]]: Mode buka (Hover, Hover & tetap, Klik) and notices with a button
- [[panel-linger-and-motion]]: the panel stays open 4 s after an action; every page and sub-view change moves (slide, fade, reveal), reduced motion, budget
- [[scroll-volume]]: the wheel over the music notch / cover turns the master volume 2 % a notch
- [[lyrics-sync]]: position rate, resync tolerance, lookahead, offset buttons, lrclib title clean + search fallback
- [[shortcut-tiles]]: Pintasan tiles chosen in Settings (page jumps, Vale Settings, own tiles added in a form), drag to reorder
- [[gestures]]: middle click, swipe and sideways wheel on the music views
- [[v3-2-handoff]]: where v3.2 stands, what is unverified, owner steps
- [[v3-3-handoff]]: v3.3 (icon refresh, stay-behind rise, shelf drop, file search), owner checks, minors left
- [[readme-structure]]: README with gallery, install and uninstall, credits in THIRD-PARTY-NOTICES.md; long guide in docs/guide.md; APNG tours
- [[stay-behind]]: experimental setting, island below ordinary windows, rises at the top edge with a short fade and slide (v3.3)
- [[shelf]]: file shelf page, OLE drop target, drag out (paths only, in memory); a drop latches the panel (v3.3)
- [[search]]: quick search on a hotkey: apps, calculator, Windows settings, web
- [[search-files]]: files and folders by name in six user folders, own walker with caps, icons/sizes/counts from a background worker
- [[mixer]]: Mixer page, per-app volume through a Core Audio worker thread

## Gotchas

- [[studio-light-secondary-text-contrast]]
- [[studio-font-loading]]
- [[demo-data-time-of-day]]
- [[studio-tabular-figures-word-gaps]]
- [[cli-panel-keyboard-focus]]
- [[media-external-pause-causes]]
- [[cli-explorer-com]]
- [[clipboard-privacy]]
- [[install-script-follows-latest-release]]
- [[shot-editor-paint-order]]: Direct2D paints in call order; the editor's footer background must come before the buttons in it (the Discard bar)
- [[taskbar-icon-from-exe]]: the Win11 taskbar shows the exe's cached icon, not WM_SETICON; the installer and the app tell the shell

## Conventions

- [[studio-bata-logo-sources]]
- [[screenshot-capture-script]]
- [[logo]]: the cat head is the logo (icon, tray, Settings, README); Windows icon caches
