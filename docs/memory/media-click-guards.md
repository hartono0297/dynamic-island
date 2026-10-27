---
tags: [decision, media, input]
date: 2026-10-27
---

# Media clicks are guarded and play/pause is explicit

Rules that keep the island from pausing music the user did not mean to pause:

- A click counts only when press and release land on the same target on the island. A press that ends somewhere else does nothing.
- After a hover-opened panel finishes its open animation, media buttons ignore clicks for 300 ms. A pointer that is still moving cannot hit play/pause by accident.
- Play/pause sends an explicit `TryPlay` or `TryPause` to the session the island shows, never a blind toggle on `GetCurrentSession()`. The session is matched by AppUserModelId. When several sessions share one AUMID (browser tabs, for example), the current session wins.

The progress timer for that same session is described in [[studio-media-progress-timer]]. When a pause has no island command behind it, see [[media-external-pause-causes]].

Listed in [[index]].
