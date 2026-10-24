# Dynamic Island untuk Windows

"Dynamic island" kecil di tengah atas layar Windows 11: kontrol musik, semua notifikasi toast Windows, notifikasi "Claude selesai/menunggu" dari Claude Code, dan peringatan baterai low. Ditulis dalam Rust (Win32 + Direct2D), tanpa browser engine dan tanpa runtime tambahan. Desain lengkapnya ada di `docs/superpowers/specs/2026-10-25-dynamic-island-design.md`.

## Setup

### 1. Toolchain (sekali saja)

- **Rust** lewat [rustup](https://rustup.rs), target default `x86_64-pc-windows-msvc`.
- **Visual Studio 2022 Build Tools** dengan workload **Desktop development with C++** (MSVC) dan **Windows SDK** (Windows 11 SDK).

Jalankan `cargo` dari **PowerShell atau cmd**, bukan Git Bash: `link.exe` milik Git ikut ada di PATH dan menutupi linker MSVC, jadi build gagal di tahap link.

### 2. Build dan jalankan

```powershell
cargo build --release
.\target\release\dynamic-island.exe
```

Exe-nya satu file (~430 KB) dan jalan langsung dari `target\release`. Kalau exe dipindah, jalankan lagi tray → "Pasang hook Claude Code" supaya hook menunjuk ke path baru (value autostart diperbarui sendiri setiap island start).

Saat pertama jalan:

- Kalau izin baca notifikasi belum ada, Windows menanyakannya. Pilih **Izinkan**. (Izin ini dicek setiap island start, jadi setelah exe diberi package identity, lihat "Mode event", izinnya ditanyakan lagi.)
- Island menampilkan petunjuk Do Not Disturb dan membuka halaman Settings → Notifications.
- `%APPDATA%\DynamicIsland\config.json` dibuat, dan island didaftarkan untuk jalan otomatis saat login (bisa dimatikan, lihat di bawah).

### 3. Matikan popup Windows (Do Not Disturb)

Supaya notifikasi tidak muncul dobel (popup Windows di kanan bawah + island), nyalakan **Do Not Disturb**: Settings → System → Notifications → Do not disturb. Island tetap menerima notifikasinya; popup Windows tidak muncul lagi. Island tidak mengubah setting ini sendiri (tidak ada API resmi untuk itu).

Konsekuensinya: notifikasi prioritas (telepon, alarm, dan aplikasi yang kamu izinkan tembus DND) tetap muncul juga dari Windows.

Petunjuk ini bisa dibuka lagi dari tray → **Cara matiin popup Windows**.

### 4. Hook Claude Code (opsional)

Tray → **Pasang hook Claude Code**. Ini menambahkan hook `Stop`, `Notification`, `UserPromptSubmit`, `PostToolUse`, dan `SessionEnd` ke `%USERPROFILE%\.claude\settings.json` yang memanggil `"...\dynamic-island.exe" --hook`.

- Hook lain yang sudah ada (misalnya dari Orca) tidak diubah; hook island ditambahkan di sebelahnya.
- Isi `settings.json` sebelum pertama kali diubah disimpan di `settings.json.bak-dynamic-island`.
- Aman dijalankan berulang kali. Kalau exe dipindah, hook lama diarahkan ke path baru, bukan diduplikasi.
- Biaya per event: exe-nya sendiri sekitar 15 ms (juga saat island sedang mati; hook selesai tanpa error). Claude Code secara default menjalankan hook lewat Git Bash, jadi totalnya sekitar 80–100 ms per event.

Notifikasi "Claude butuh respon" hanya muncul untuk notifikasi Claude yang memang menunggu kamu (minta izin, menunggu input, atau dialog pertanyaan); notifikasi lain dari Claude Code diabaikan. Notifikasi ini tetap tampil sampai ada event Claude berikutnya untuk sesi yang sama (termasuk sesi ditutup, `SessionEnd`), atau sampai diklik.

### 5. Jalan otomatis saat login

Tray → **Jalan saat Windows start** (default: nyala). Ini menulis/menghapus value `DynamicIsland` di `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

## Pemakaian

- **Tray icon** (klik kiri atau kanan): Gaya Pill / Notch, Jalan saat Windows start, Cara matiin popup Windows, Pasang hook Claude Code, Keluar.
- **Musik:** diamkan kursor sebentar (~0,35 detik) di atas island atau klik untuk memperbesar dan memunculkan tombol prev / play-pause / next.
- **Klik notifikasi:** membuka aplikasi sumbernya (bukan chat spesifiknya) lalu menutup notifikasi dari island. Notifikasinya tetap ada di Notification Center Windows.
- **Fullscreen** (game, video, presentasi): island ngumpet; notifikasi baru tetap muncul sebentar.

### config.json

`%APPDATA%\DynamicIsland\config.json`, dibaca saat island start (edit manual → keluar dari tray lalu jalankan lagi):

| Field | Default | Keterangan |
|---|---|---|
| `style` | `"Pill"` | `"Pill"` atau `"Notch"` |
| `battery_threshold` | `20` | Peringatan saat baterai ≤ nilai ini (%) dan tidak di-charge |
| `start_with_windows` | `true` | Sama dengan toggle di tray |
| `muted_apps` | `["Claude", "Orca"]` | Toast dari aplikasi yang namanya mengandung salah satu teks ini (huruf besar/kecil diabaikan) tidak ditampilkan. Default-nya menyaring Claude desktop dan Orca karena notifikasi Claude sudah datang dari hook |
| `onboarding_done` | `false` | Set `false` untuk mengulang langkah pertama jalan |

JSON yang rusak dianggap kosong dan semua field kembali ke default.

## Mode event untuk notifikasi (opsional)

Tanpa package identity, island membaca daftar notifikasi **setiap 1 detik** (polling). Biayanya sekitar 15–30 ms CPU per detik saat idle (±2–3% satu core). Fitur lain tidak terpengaruh.

Kalau mau idle mendekati 0, exe bisa diberi package identity lewat sparse package; island lalu otomatis berlangganan event `NotificationChanged` dan hanya membaca saat ada perubahan (plus satu pembacaan cadangan tiap 60 detik):

1. Jalankan (PowerShell biasa, tanpa admin):
   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1
   ```
   `-ExecutionPolicy Bypass` hanya berlaku untuk perintah ini; execution policy sistem tidak berubah (default Windows `Restricted` menolak menjalankan `.ps1`).
   Pertama kali, script membuat sertifikat self-signed lokal (`packaging\dev-cert\`, tidak di-commit) dan mencetak perintah `Import-Certificate ...`.
2. Jalankan perintah yang dicetak itu **sekali** di PowerShell **Administrator**:
   ```powershell
   Import-Certificate -FilePath "<repo>\packaging\dev-cert\dev.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople
   ```
   Ini membuat Windows **mempercayai sertifikat self-signed buatan lokal** untuk seluruh komputer (store `LocalMachine\TrustedPeople`). Siapa pun yang memegang `dev.pfx` dan password-nya bisa menandatangani package yang akan dipercaya mesin ini, jadi jaga folder `packaging\dev-cert\`. Untuk membatalkan: hapus sertifikat `CN=DynamicIslandDev` dari store itu, lalu `Get-AppxPackage DynamicIsland | Remove-AppxPackage`.
3. Jalankan lagi `powershell -ExecutionPolicy Bypass -File .\scripts\package.ps1`. Package `DynamicIsland` terdaftar dengan lokasi eksternal `target\release`.
4. Keluar dari island lalu jalankan lagi `target\release\dynamic-island.exe`.

Cek hasilnya dengan `--probe-notifications` (lihat Troubleshooting): baris `Change events: subscribed` berarti mode event aktif. Kalau berlangganan event gagal, island kembali ke polling 1 detik dengan sendirinya.

## Troubleshooting

- **Notifikasi tidak masuk ke island:** cek izin di Settings → Privacy & security → Notifications (akses notifikasi untuk aplikasi). Lalu jalankan probe dari terminal:
  ```powershell
  .\target\release\dynamic-island.exe --probe-notifications
  ```
  Probe meminta izin kalau belum ada, mencetak apakah mode event aktif (`Change events: subscribed` / `unavailable`), lalu notifikasi yang masuk selama 60 detik. Kirim toast uji dari terminal lain dengan `powershell -ExecutionPolicy Bypass -File .\scripts\send-test-toast.ps1` (tambahkan `-Long` di belakangnya untuk teks 300 karakter dengan emoji).
- **Probe khusus build debug** (`cargo build`, lalu `.\target\debug\dynamic-island.exe <arg>`):
  - `--probe-media`: cetak media yang sedang diputar setiap kali berubah.
  - `--probe-system`: cetak status baterai dan fullscreen setiap detik.
  - `--demo`: putar semua bentuk island (Pill lalu Notch) setiap 2 detik dengan data contoh. Jalan terus sampai prosesnya dimatikan.

  Hentikan dengan Ctrl+C, atau `Stop-Process -Name dynamic-island` (ini ikut menghentikan island utama kalau sedang jalan).
- **Island tidak muncul sama sekali:** wajar saat tidak ada musik dan notifikasi (island disembunyikan). Cek tray icon; hanya satu instance yang bisa jalan.
- **Hook Claude tidak memunculkan apa-apa:** pastikan island jalan dan `settings.json` berisi hook dengan path exe yang benar (pasang ulang dari tray).

## Batasan v1

- Hanya monitor utama.
- Tidak ada balas langsung atau tombol aksi untuk notifikasi (API resmi tidak menyediakannya).
- Aplikasi yang tidak memakai toast Windows (kemungkinan chat Steam) tidak tertangkap.
