# Dynamic Island untuk Windows — Desain

Tanggal: 2026-10-25

## Intent

Sebuah "dynamic island" di tengah atas layar Windows 11, terinspirasi dari iPhone, yang jadi satu tempat buat kontrol musik dan semua notifikasi. Dipakai sendiri setiap hari oleh pemiliknya, sekaligus proyek eksperimen. Island menggantikan popup notifikasi bawaan Windows (pojok kanan bawah): kalau island sudah ada, notifikasi cukup muncul di island.

## Goals (v1)

1. **Kontrol media.** Tampilkan media yang sedang diputar dari aplikasi apa pun yang terdaftar di Windows media session (Spotify, YouTube di browser, dll.): judul, artis, cover art, status play/pause. Kontrol: play/pause, next, previous.
2. **Semua notifikasi Windows.** Notifikasi toast dari aplikasi lain (WhatsApp, Discord, dll.) muncul di island: ikon/nama aplikasi, judul, isi teks.
3. **Notifikasi "Claude selesai".** Saat Claude Code selesai satu turn (dan saat Claude menunggu izin/input), island menampilkan notifikasi berisi nama project. Sumbernya hook Claude Code, bukan notifikasi Windows.
4. **Baterai low.** Island membaca status baterai sendiri dan menampilkan peringatan saat baterai di bawah ambang (default 20%) dan tidak sedang di-charge.
5. **Dua gaya, bisa dipilih di settings:**
   - **A. Pill:** hitam pekat, bulat penuh, melayang ~8px di bawah tepi atas layar.
   - **B. Notch:** hitam, menempel ke tepi atas layar, sudut bawah membulat, "bahu" atas melengkung ke dalam seperti notch MacBook.
   Mockup referensi ada di `.superpowers/brainstorm/*/content/island-style.html`.
6. **Ringan.** Prioritas utama dari pemilik: "yang paling ringan, enteng, dan kecil".

## Non-goals (v1)

- Fitur "hal berguna lainnya" (kalender, timer, dll.) belum didefinisikan; dibahas setelah v1 jalan. Arsitektur sebaiknya membuat penambahan jenis konten baru gampang, tapi jangan dibangun spekulatif.
- Chat Steam: kemungkinan Steam tidak memakai toast Windows (dugaan, belum dicek). Kalau tidak tertangkap lewat listener, tidak dikejar di v1; cukup dicatat hasil pengecekannya.
- Multi-monitor: v1 hanya di monitor utama.
- macOS/Linux.
- Distribusi publik (installer, Store, auto-update).

## Anti-goals (gagal meskipun "jalan")

- Berat: RAM puluhan-ratusan MB, CPU terus-menerus aktif saat idle, atau terasa ngelag.
- Menghalangi kerja: island menangkap klik di area di luar bentuk island-nya sendiri, atau menutupi konten saat tidak ada yang perlu ditampilkan.
- Notifikasi dobel: popup Windows dan island sama-sama muncul untuk notifikasi yang sama.
- Membaca notifikasi lewat cara tidak resmi yang gampang rusak (misalnya membaca langsung file database notifikasi Windows).

## Keputusan yang sudah diambil

| Keputusan | Diputuskan oleh | Catatan |
|---|---|---|
| Island menggantikan popup Windows | Pemilik | Popup bawaan dimatikan supaya tidak dobel |
| Semua notifikasi Windows masuk ke island | Pemilik | |
| Gaya A dan B, bisa dipilih di settings; gaya "Windows 11 acrylic" tidak dipakai | Pemilik | |
| Bahasa: **Rust**, aplikasi native tanpa browser engine dan tanpa runtime tambahan | Claude (pemilik menyerahkan pilihan: "yang paling ringan") | Electron, Tauri/WebView2, dan .NET ditolak karena lebih berat |
| Notifikasi dibaca lewat API resmi `UserNotificationListener` (`Windows.UI.Notifications.Management`) | Claude | Butuh package identity + capability `userNotificationListener`, dan user menekan "Izinkan" sekali |
| Package identity lewat **sparse package** (MSIX dengan external location) yang ditandatangani sertifikat self-signed lokal | Claude | Setup sekali; exe tetap jalan dari folder build |
| Media lewat `GlobalSystemMediaTransportControlsSessionManager` (GSMTC) | Claude | |
| Notifikasi Claude lewat hook Claude Code (`Stop` dan `Notification`) yang mengirim pesan ke island lewat kanal lokal | Claude | Hook yang sudah ada di `~/.claude/settings.json` (dari aplikasi Orca) harus tetap ada; hook baru ditambahkan, bukan menimpa |
| Baterai dibaca langsung dari sistem (bukan dari notifikasi) | Claude | |
| Saat aplikasi fullscreen (game, video, presentasi): island ngumpet; notifikasi baru tetap muncul sebentar di atas, lalu island ngumpet lagi | Pemilik | "Kayak dynamic island di Apple" |
| Popup Windows dimatikan lewat **Do Not Disturb** yang dinyalakan pemilik sendiri sekali; island hanya menampilkan petunjuk | Claude | Tidak ada API resmi untuk mengubahnya dari aplikasi, dan trik registry/WNF melanggar anti-goal. Konsekuensi yang diterima: notifikasi prioritas (telepon, alarm) tetap bisa muncul dari Windows; aplikasi yang tidak lolos DND tetap muncul di Windows |
| Klik notifikasi di island = buka aplikasi sumbernya (bukan chat spesifik), lalu notifikasi ditutup dari island. Tidak ada balas langsung/tombol aksi | Claude (pertanyaan ke pemilik belum dijawab; batas teknis API resmi) | Listener resmi tidak bisa memicu aksi klik toast asli |
| Notifikasi tetap tersimpan di Notification Center Windows (island tidak menghapusnya) | Claude | Kalau island mati/crash, notifikasi tidak hilang |
| Notifikasi Claude hanya dari hook. Toast Windows dari aplikasi Claude desktop dan Orca disaring (tidak tampil di island) supaya tidak dobel | Claude | |
| Notifikasi "Claude menunggu izin/input" tetap tampil sampai event hook Claude berikutnya untuk sesi yang sama (atau diklik), tidak hilang otomatis | Claude | |

## Perilaku

- **Idle:** saat tidak ada media dan tidak ada notifikasi, island mengecil ke bentuk minimal atau disembunyikan (builder menentukan mana yang lebih enak, asalkan tidak mengganggu).
- **Media diputar:** bentuk kecil berisi cover art kecil, nama artis/judul singkat, indikator equalizer. Hover/klik memperbesar island dan menampilkan judul lengkap, artis, dan tombol prev/play-pause/next.
- **Notifikasi masuk:** island membesar dengan animasi, menampilkan notifikasi beberapa detik, lalu kembali ke keadaan sebelumnya (misalnya kembali ke tampilan musik). Notifikasi yang datang berdekatan diantrekan, tidak saling menimpa.
- **Animasi:** perubahan bentuk (kecil ↔ besar) harus mulus, bukan lompat. Ini bagian inti dari "rasa" dynamic island.
- **Klik-tembus:** area transparan di sekitar island tidak boleh menangkap klik.
- **Selalu di atas** jendela biasa.
- **Fullscreen:** saat aplikasi fullscreen aktif di monitor utama, island ngumpet (termasuk tampilan musik). Notifikasi baru tetap muncul sebentar, lalu island ngumpet lagi.
- **Saat pertama jalan:** minta izin baca notifikasi; kalau Do Not Disturb belum aktif, tampilkan petunjuk singkat cara menyalakannya.
- **Jalan otomatis** saat Windows login (bisa dimatikan di settings).
- **Settings minimal:** pilihan gaya (A/B), ambang baterai, start with Windows. Disimpan di file konfigurasi lokal.

## Constraints

- Windows 11 (laptop pemilik: Windows 11 Home 26200). Ada baterai.
- Target konsumsi: RAM di kisaran puluhan MB ke bawah saat idle, CPU mendekati 0% saat idle. Animasi tetap mulus.
- Sudah dicek 2026-10-25: Rust, MSVC Build Tools, dan Windows SDK **belum terpasang** (Visual Studio Installer ada, tapi tanpa produk). Perlu dipasang: rustup (target `x86_64-pc-windows-msvc`), MSVC Build Tools, dan Windows SDK (untuk MakeAppx/SignTool bagi sparse package). Node, Python 3.13, dan .NET ada.
- Island tidak boleh mengubah setting sistem Windows. Builder harus memverifikasi di awal bahwa notifikasi tetap sampai ke `UserNotificationListener` saat Do Not Disturb aktif; kalau ternyata tidak, berhenti dan laporkan sebelum lanjut, karena itu mengubah desain.

## Diserahkan ke builder

- Library UI/rendering di Rust (misalnya Win32 + Direct2D/DirectComposition lewat crate `windows`, atau framework lain), asalkan memenuhi target ringan, transparan per-piksel, klik-tembus, dan animasi mulus.
- Kanal komunikasi hook Claude → island (named pipe, localhost, dll.) dan format pesannya.
- Struktur kode, cara menyimpan settings, dan cara membuka settings (tray icon, klik kanan island, dll.).
- Durasi tampil notifikasi, kurva animasi, ukuran persis island.
- Detail tampilan mengikuti mockup referensi; tidak perlu pixel-perfect.
