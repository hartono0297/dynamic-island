---
tags: [decision, google, calendar, oauth]
date: 2026-10-28
---

# Google login: Vale's built-in client, or the user's own

Run 14 of the v3.1 plan. `src/google/client.rs` picks the OAuth client:

- **Own first**: a Client ID in config (Settings → Integrasi → "Lanjutan:
  pakai Client ID sendiri") wins, so people who set one up keep it.
- **Else built-in**: compiled in from `VALE_GOOGLE_CLIENT_ID` /
  `VALE_GOOGLE_CLIENT_SECRET` at build time (`option_env!`). The release
  workflow reads them from repository secrets of the same names. A Desktop-app
  client's secret is not secret for Google, so it may sit in the exe.
- **Else none**: `NotConfigured`, as before. Local builds have no built-in
  client unless those variables are set, so they look like v3.0.

Settings with a built-in client: one card row with "Masuk dengan Google" and
"Putuskan", and a disclosure link (`ButtonKind::Disclosure`, a chevron, not the
"open" glyph) under it that shows the old Client ID / secret fields. It
starts open when an own ID is saved. Without a built-in client the card is
unchanged and there is no link.

Tokens record the client ID that issued them (`Tokens::client`, empty in token
files from before). Tokens from the other client give `AuthError::ClientChanged`
→ status NeedsLogin and a "Masuk Google lagi" notice; empty means the user's
own client (the only kind before). PKCE, loopback and scopes are unchanged.

Owner steps (not the agent's): create the Google Cloud project and Desktop
client, publish the consent screen (path B, then verification C with the
`site/` pages), and add the two repository secrets.

To try the built-in look locally, build with placeholder values in those two
variables (into another `CARGO_TARGET_DIR`) and run `--settings integrations`
with `DI_GOOGLE=needslogin`.

- A rejected client (`invalid_client`) is a refusal with advice, not "not
  set up": the user's own → "cek Pengaturan → Integrasi", Vale's → "coba lagi
  nanti, atau pakai Client ID sendiri" (`sync::rejected`).
- While an own Client ID is filled in, its fields and "Hubungkan" show and
  the disclosure is hidden: the button never logs in with a client its label
  does not name. `Client`'s Debug redacts the secret.

- v3.2 Run 5 (messages only): without a built-in client and without an ID the
  status chip reads "Isi Client ID dulu" (`google_status_key`), the notice
  body says "Pengaturan → Integrasi, panduan ada di README", and the browser
  page after a login tells a cancelled consent (`OauthCancelled`) and a
  rejected client (`OauthRejected`) from a plain failure. The real fix, a
  built-in client, is still the owner's: create the Cloud project, set the two
  secrets, release.

Related: [[v3-1-handoff]], [[v3-studio-overview]]
