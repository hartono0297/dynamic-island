# Dynamic Island untuk Windows

"Dynamic island" kecil di tengah atas layar Windows 11. Notch hitamnya selalu ada; di-hover jadi panel dengan empat halaman: **Kalender** (Google Calendar, bisa tambah dan edit acara), **Pomodoro**, **Notes**, dan **Musik**. Selain itu island tetap menampilkan semua notifikasi toast Windows, notifikasi "Claude selesai/menunggu" dari Claude Code, dan peringatan baterai low. Ditulis dalam Rust (Win32 + Direct2D), tanpa browser engine dan tanpa runtime tambahan.

Desainnya ada di `docs/superpowers/specs/2026-10-25-dynamic-island-design.md` (v1) dan `docs/superpowers/specs/2026-10-25-dynamic-island-v2-panel-design.md` (v2).

## Yang baru di v2

- **Notch selalu terlihat.** Saat tidak ada apa-apa, island tampil sebagai notch/pill hitam kosong seukuran tampilan compact. Hanya ngumpet saat ada aplikasi fullscreen.
- **Panel saat hover**, ganti halaman dengan roda mouse: Kalender → Pomodoro → Notes → Musik.
- **Google Calendar**: tampilan Hari / Minggu / Bulan, tambah acara, edit acara. Perlu setup sekali, lihat [Setup Google Calendar](#setup-google-calendar).
- **Pomodoro** dengan lingkaran progres, di panel maupun di tampilan kecil.
- **Notes** lokal, ditulis di jendela kecil bertema gelap.
- Animasi membesar/mengecil dan pindah halaman lebih mulus; equalizer bergerak saat musik diputar.

## Install

Windows 11 64-bit. Buka **PowerShell** (tidak perlu admin), tempel satu baris ini, Enter:

```powershell
irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/install.ps1 | iex
```

Script-nya mengunduh rilis terbaru dari [Releases](https://github.com/hartono0297/dynamic-island/releases), mengecek checksum SHA-256, memasang `dynamic-island.exe` ke `%LOCALAPPDATA%\Programs\DynamicIsland`, membuat shortcut di Start Menu, lalu menjalankan island. Exe-nya satu file (~870 KB) tanpa runtime tambahan.

- **Update:** jalankan baris yang sama lagi. Island lama ditutup, exe diganti, island jalan lagi. Setting, notes, dan login Google tetap.
- **Tanpa script:** unduh `dynamic-island-<versi>.zip` dari Releases, ekstrak, lalu klik kanan `install.ps1` → *Run with PowerShell* (atau `powershell -ExecutionPolicy Bypass -File .\install.ps1`). Exe-nya juga bisa langsung dijalankan dari folder mana saja.
- **Uninstall:**

  ```powershell
  irm https://raw.githubusercontent.com/hartono0297/dynamic-island/main/uninstall.ps1 | iex
  ```

  Ini menghapus hook Claude Code milik island, autostart, shortcut, dan folder program. Data di `%APPDATA%\DynamicIsland` (setting, notes, login Google) tetap disimpan; untuk ikut menghapusnya, jalankan `$env:DI_PURGE = '1'` dulu sebelum baris uninstall.
- **Windows SmartScreen:** exe-nya belum ditandatangani (code signing berbayar). Kalau Windows memperingatkan saat exe dibuka dari hasil unduhan manual, pilih *More info → Run anyway*.

## Setelah install

Saat pertama jalan:

- Kalau izin baca notifikasi belum ada, Windows menanyakannya. Pilih **Izinkan**. (Izin ini dicek setiap island start, jadi setelah exe diberi package identity, lihat "Mode event", izinnya ditanyakan lagi.)
- Island menampilkan petunjuk Do Not Disturb dan membuka halaman Settings → Notifications.
- `%APPDATA%\DynamicIsland\config.json` dibuat, dan island didaftarkan untuk jalan otomatis saat login (bisa dimatikan, lihat di bawah).

### 1. Matikan popup Windows (Do Not Disturb)

Supaya notifikasi tidak muncul dobel (popup Windows di kanan bawah + island), nyalakan **Do Not Disturb**: Settings → System → Notifications → Do not disturb. Island tetap menerima notifikasinya; popup Windows tidak muncul lagi. Island tidak mengubah setting ini sendiri (tidak ada API resmi untuk itu).

Konsekuensinya: notifikasi prioritas (telepon, alarm, dan aplikasi yang kamu izinkan tembus DND) tetap muncul juga dari Windows.

Petunjuk ini bisa dibuka lagi dari tray → **Cara matiin popup Windows**.

### 2. Hook Claude Code (opsional)

Tray → **Pasang hook Claude Code**. Ini menambahkan hook `Stop`, `Notification`, `UserPromptSubmit`, `PostToolUse`, dan `SessionEnd` ke `%USERPROFILE%\.claude\settings.json` yang memanggil `"...\dynamic-island.exe" --hook`.

- Hook lain yang sudah ada (misalnya dari Orca) tidak diubah; hook island ditambahkan di sebelahnya.
- Isi `settings.json` sebelum pertama kali diubah disimpan di `settings.json.bak-dynamic-island`.
- Aman dijalankan berulang kali. Kalau exe dipindah, hook lama diarahkan ke path baru, bukan diduplikasi.
- Biaya per event: exe-nya sendiri sekitar 15 ms (juga saat island sedang mati; hook selesai tanpa error). Claude Code secara default menjalankan hook lewat Git Bash, jadi totalnya sekitar 80–100 ms per event.

Notifikasi "Claude butuh respon" hanya muncul untuk notifikasi Claude yang memang menunggu kamu (minta izin, menunggu input, atau dialog pertanyaan); notifikasi lain dari Claude Code diabaikan. Notifikasi ini tetap tampil sampai ada event Claude berikutnya untuk sesi yang sama (termasuk sesi ditutup, `SessionEnd`), atau sampai diklik.

### 3. Jalan otomatis saat login

Tray → **Jalan saat Windows start** (default: nyala). Ini menulis/menghapus value `DynamicIsland` di `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

### 4. Google Calendar (opsional)

Halaman Kalender butuh project Google Cloud milikmu sendiri. Langkahnya ada di [Setup Google Calendar](#setup-google-calendar). Tanpa itu, semua fitur lain tetap jalan dan halaman Kalender hanya menampilkan tombol "Hubungkan Google".

## Build dari source

1. **Rust** lewat [rustup](https://rustup.rs), target default `x86_64-pc-windows-msvc`.
2. **Visual Studio 2022 Build Tools** dengan workload **Desktop development with C++** (MSVC) dan **Windows SDK** (Windows 11 SDK).
3. Dari **PowerShell atau cmd** (bukan Git Bash: `link.exe` milik Git menutupi linker MSVC):

   ```powershell
   cargo build --release
   .\target\release\dynamic-island.exe
   ```

C runtime di-link statis (`.cargo/config.toml`), jadi exe hasil build jalan di PC tanpa Visual C++ Redistributable. Kalau island sedang jalan dari `target\release`, keluar dulu dari tray sebelum build: exe yang sedang dipakai tidak bisa ditimpa. Untuk memasang hasil build sendiri: `powershell -ExecutionPolicy Bypass -File .\install.ps1` dari folder yang berisi `dynamic-island.exe` dan `install.ps1` (salin exe-nya ke samping script dulu).

Rilis baru: push tag `vX.Y.Z`; GitHub Actions (`.github/workflows/release.yml`) membangun exe dan mengunggah `dynamic-island.exe`, checksum-nya, dan zip ke Releases.

## Pemakaian

- **Tray icon** (klik kiri atau kanan): Gaya: Pill, Gaya: Notch, Jalan saat Windows start, Cara matiin popup Windows, Pasang hook Claude Code, Keluar.
- **Dua gaya:** **Pill** melayang 8 px dari tepi atas dengan semua sudut bulat; **Notch** menempel ke tepi atas dengan bahu melengkung. Semua tampilan (notch kosong, panel, pomodoro kecil, notifikasi) mengikuti gaya yang dipilih.
- **Tampilan kecil**, urutan prioritas dari yang tertinggi: notifikasi sementara (app, Claude selesai, baterai, pomodoro selesai) → "Claude butuh respon" → pomodoro (jalan, dijeda, atau menunggu diklik) → musik (cover + equalizer yang bergerak selama lagu diputar) → notch kosong.
- **Klik notifikasi:** membuka aplikasi sumbernya (bukan chat spesifiknya) lalu menutup notifikasi dari island. Notifikasinya tetap ada di Notification Center Windows.
- **Fullscreen** (game, video, presentasi): semuanya ngumpet, termasuk notch kosong dan panel; notifikasi baru tetap muncul sebentar.
- Di luar bentuk island, klik tembus ke jendela di bawahnya.

### Panel

- **Buka:** diamkan kursor sebentar (~0,35 detik) di atas island. Kursor yang sekadar lewat tidak membukanya. Kursor keluar → panel mengecil lagi.
- **Pindah halaman:** putar roda mouse di atas panel. Scroll ke bawah = halaman berikutnya (Kalender → Pomodoro → Notes → Musik), scroll ke atas = sebelumnya; tidak berputar dari Musik balik ke Kalender. Titik di bawah panel menunjukkan halaman yang aktif. Island seharusnya tidak perlu diklik dulu supaya roda mouse sampai (belum dicek di semua mouse/touchpad; kalau roda tidak berefek, klik panel sekali).
- Panel dibuka lagi di halaman terakhir yang dilihat.
- **Notifikasi masuk saat panel terbuka:** panel mengecil jadi notifikasi itu (~5 detik, bisa diklik), lalu terbuka lagi kalau kursor masih di atasnya. Selama notifikasi sementara tampil, hover tidak membuka panel.
- **"Claude butuh respon"** tidak menahan panel: tampil sebagai strip kecil di atas panel. Klik strip itu untuk menutupnya.
- Selama jendela note atau acara terbuka, panel tetap terbuka.

### Kalender

- Tombol **Hari / Minggu / Bulan** di kanan atas; panah di sebelah judul pindah ke hari/minggu/bulan sebelumnya atau berikutnya.
- **Bulan:** grid tanggal, hari ini disorot, tanggal yang punya acara diberi titik. Klik tanggal → daftar acaranya di kanan.
- **Minggu:** 7 kolom dengan acara per hari. Kolom yang kepenuhan menampilkan "+N"; klik untuk membuka hari itu di tampilan Hari.
- **Hari:** daftar acara urut jam.
- **"+N lagi"**: daftar tidak muat. Dari tampilan Bulan, klik untuk membuka hari itu di tampilan Hari; di tampilan Hari, klik untuk menampilkan acara berikutnya. "Kembali ke atas" mengulang dari awal.
- **"+ Tambah acara"** atau klik acara → jendela kecil di bawah panel: Judul, Tanggal, Seharian, Mulai, Selesai, Catatan. **Simpan** (atau Ctrl+Enter) mengirim ke Google Calendar; **Batal** atau Esc menutup tanpa menyimpan. Kalau gagal simpan, pesannya tampil di jendela itu dan isiannya tidak hilang.
- Acara baru masuk ke kalender utama (primary). Yang ditampilkan: acara dari semua kalender yang dicentang di Google Calendar.
- **Hanya-baca di island:** acara berulang, acara beberapa hari, dan acara dari kalender yang tidak bisa kamu ubah. Jendelanya hanya menampilkan isi dan tombol **Tutup**. Ubah acara seperti itu di Google Calendar web.
- Menghapus acara tidak bisa dari island.
- **Pembaruan:** data diambil saat halaman Kalender dibuka (kalau sudah lebih dari 1 menit), setiap pindah periode, dan otomatis tiap 10 menit. Zona waktu mengikuti Windows.
- **Offline:** data terakhir tetap ditampilkan dengan tanda "offline" di judul. Data itu disimpan di `calendar-cache.json`, jadi tetap ada setelah island dijalankan ulang.

### Pomodoro

- Lingkaran progres besar dengan sisa waktu, label fase (Fokus / Istirahat / Istirahat panjang), "Sesi n dari N", dan tombol **Mulai/Jeda/Lanjut**, **Reset**, **Lewati**.
- Durasi diatur dengan tombol **−** / **+**: Fokus 5–120 menit (langkah 5), Istirahat 1–30 menit, Istirahat panjang 5–60 menit (langkah 5), Jumlah sesi 1–12. Default 25 / 5 / 15 menit, 4 sesi. Tersimpan di `config.json`. Tombolnya redup (tidak bisa diubah) selama fase berjalan atau dijeda.
- Saat timer jalan dan panel tertutup: lingkaran kecil + sisa waktu (mm:ss) + "Fokus 2/4" atau "Istirahat". Merah = fokus, hijau = istirahat.
- **Fase selesai:** bunyi notifikasi Windows sekali, notifikasi ~5 detik ("Fokus selesai · Istirahat 5 menit · klik untuk mulai"), lalu island kecil menetap di "Siap · klik". **Fase berikutnya baru mulai setelah diklik**, di island kecil atau tombol Mulai di panel.
- Setelah Jumlah sesi fokus, istirahatnya istirahat panjang.
- Timer tidak disimpan: keluar dari island = pomodoro mulai dari awal.

### Notes

- Daftar note (baris pertama + waktu terakhir diubah), terbaru di atas. Kalau tidak muat, klik "+N lagi" untuk halaman berikutnya ("Kembali ke atas" mengulang). Roda mouse tetap untuk pindah halaman panel, bukan menggulir daftar.
- **"+ Baru"** atau klik note → jendela kecil bertema gelap dengan kotak teks biasa (copy-paste, undo, emoji, IME jalan).
- **Tersimpan otomatis** saat jendela ditutup: Ctrl+Enter, Esc, atau klik di luar jendela.
- **Hapus** (note yang sudah ada): menghapus note itu. Mengosongkan teksnya juga menghapus note.
- Disimpan lokal di `%APPDATA%\DynamicIsland\notes.json`, tidak disinkron ke mana pun. Kalau file itu rusak dan tidak bisa dibaca, island menampilkan notice "notes.json tidak terbaca" saat start (daftar notes kosong, tapi filenya belum disentuh) dan memindahkannya ke `notes.json.bak` (atau `.bak2`, `.bak3`…) sebelum menyimpan berikutnya, jadi isinya tidak tertimpa.

### Musik

Halaman ke-4: cover, judul, artis, dan tombol prev / play-pause / next untuk media yang sedang diputar di aplikasi mana pun (Spotify, browser, dll.). Saat panel tertutup, musik tampil di island kecil dengan equalizer yang bergerak selama lagu diputar (berhenti saat dijeda).

## Setup Google Calendar

Island memakai Google Calendar API resmi lewat OAuth "Desktop app", dengan project Google Cloud milikmu sendiri. Cukup sekali.

1. Buka [Google Cloud Console](https://console.cloud.google.com/) dan buat project baru (pemilih project di bar atas → **New project**). Namanya bebas, misalnya "Dynamic Island".
2. **APIs & Services → Library** → cari **Google Calendar API** → **Enable**.
3. Atur consent screen: buka **Google Auth Platform** (atau **APIs & Services → OAuth consent screen**, yang membuka halaman yang sama) → **Get started**.
   - App name: bebas. User support email: email kamu.
   - Audience: **External**.
   - Contact information: email kamu. Setujui kebijakan data pengguna, lalu **Create**.
4. **Audience → Test users → Add users** → masukkan alamat Gmail yang kalendernya mau ditampilkan → **Save**. Publishing status tetap **Testing** (lihat catatan 7 hari di bawah).
5. **Clients → Create client** → Application type **Desktop app** → nama bebas → **Create**. Salin **Client ID** dan **Client secret** saat itu juga: Google bisa menampilkan client secret **hanya sekali**, di dialog setelah Create. Kalau terlewat, buka client itu dan tambahkan secret baru (**Add secret**), lalu pakai yang baru. (Di tampilan konsol lama: **Credentials → Create credentials → OAuth client ID**.) Tidak perlu mengisi redirect URI: island memakai redirect loopback `http://127.0.0.1:<port>` yang otomatis diizinkan untuk Desktop app.
6. Buka `%APPDATA%\DynamicIsland\config.json` dan isi dua field ini, lalu simpan filenya:
   ```json
   "google_client_id": "1234567890-abc...apps.googleusercontent.com",
   "google_client_secret": "GOCSPX-..."
   ```
   Kalau field-nya belum ada (config dari v1), tambahkan kedua baris itu di dalam `{ }` dan **jangan lupa koma** di akhir baris sebelumnya; tanpa koma JSON-nya tidak valid (lihat [config.json dan file data](#configjson-dan-file-data)).
   Island tidak perlu ditutup atau dijalankan ulang: `config.json` dibaca lagi setiap kali kamu klik Hubungkan Google.
7. Hover island → halaman **Kalender** → **Hubungkan Google**. Browser default terbuka:
   - Pilih akun yang tadi didaftarkan sebagai test user.
   - Google menampilkan layar "Google hasn't verified this app" karena app-nya milikmu sendiri dan belum diverifikasi. Pilih **Continue**.
   - Izinkan dua akses yang diminta: **lihat dan ubah acara** di kalendermu (scope `https://www.googleapis.com/auth/calendar.events`) dan **lihat daftar kalender** yang kamu ikuti (scope `https://www.googleapis.com/auth/calendar.calendarlist.readonly`, supaya island tahu kalender mana yang dicentang). Google bisa menampilkan keduanya sebagai **kotak centang yang belum dicentang**: centang **keduanya**. Kalau salah satu tidak dicentang, island tidak menyimpan login itu dan menampilkan "Izin Google belum lengkap".
   - Setelah island selesai menukar kode login dengan token, browser menampilkan "Berhasil terhubung. Kembali ke island ya." dan acaramu muncul di panel. Kalau gagal, browser menampilkan "Gagal terhubung — coba lagi dari island".
   - Island menunggu login paling lama 5 menit. Selama itu klik Hubungkan Google berikutnya diabaikan. Kalau login tidak selesai dalam 5 menit (tab browser ditutup, dibatalkan, atau Google menampilkan halaman error), tombol Hubungkan Google tetap ada: klik lagi saja.

Kalau client ID masih kosong, klik Hubungkan Google memunculkan "Isi Google client ID dulu · lihat README".

### Login ulang tiap 7 hari (status "Testing")

Selama publishing status project masih **Testing**, Google memberi refresh token yang **kedaluwarsa setelah 7 hari** untuk scope kalender. Setelah itu halaman Kalender menampilkan **Hubungkan ulang Google**; klik dan login lagi seperti langkah 7.

Supaya tidak perlu login ulang tiap minggu: Google Auth Platform → **Audience** → **Publish app** → status jadi **In production**. Untuk pemakaian pribadi kamu tidak perlu mengajukan verifikasi; konsekuensinya layar "Google hasn't verified this app" tetap muncul setiap login (pilih **Advanced** → lanjutkan ke app-mu). Token yang didapat selama status masih Testing tetap kedaluwarsa di hari ke-7; setelah itu island menampilkan **Hubungkan ulang Google**, dan login sekali lagi memberi token tanpa batas 7 hari. Mau langsung pindah tanpa menunggu: keluar dari island, hapus `%APPDATA%\DynamicIsland\google-token.bin`, jalankan island lagi, lalu klik **Hubungkan Google**.

### Di mana kredensialnya disimpan

- **Client ID dan client secret** ada di `%APPDATA%\DynamicIsland\config.json` sebagai **teks biasa**. Untuk Desktop app, client secret menurut Google memang tidak benar-benar rahasia (semua orang yang punya exe-nya bisa membacanya), tapi tetap jangan bagikan file ini.
- **Token login** disimpan terenkripsi dengan DPAPI Windows di `%APPDATA%\DynamicIsland\google-token.bin`; hanya akun Windows-mu di komputer ini yang bisa membukanya. Token tidak pernah dicetak ke log atau console.
- **Memutus koneksi:** keluar dari island, hapus `google-token.bin`, lalu cabut akses app-nya di [myaccount.google.com/permissions](https://myaccount.google.com/permissions). Kalau Google menolak token (dicabut atau kedaluwarsa), island menghapus file itu sendiri dan menampilkan Hubungkan ulang Google.

## config.json dan file data

Semua data island ada di `%APPDATA%\DynamicIsland\`:

| File | Isi |
|---|---|
| `config.json` | Pengaturan (tabel di bawah) |
| `notes.json` | Notes |
| `calendar-cache.json` | Salinan terakhir data kalender, untuk mode offline |
| `google-token.bin` | Token Google, terenkripsi DPAPI |

`config.json` dibaca saat island start. Field-nya ada dua jenis:

- **Boleh diedit manual walau island sedang jalan:** `google_client_id`, `google_client_secret`, `muted_apps`, `battery_threshold`. Island tidak pernah menimpa isianmu. Field Google dibaca lagi setiap klik Hubungkan Google; `muted_apps` dan `battery_threshold` paling lambat berlaku setelah island dijalankan ulang.
- **Milik island:** `style`, `start_with_windows`, `pomodoro`, `onboarding_done`. Ubah dari island (tray atau halaman Pomodoro), atau edit saat island tidak jalan (tray → Keluar → edit → jalankan lagi). Island menulis field ini sendiri setiap kali kamu mengubahnya dari tray atau panel, jadi editan manual saat island jalan bisa tertimpa.

Kalau `config.json` bukan JSON yang valid (misalnya koma ketinggalan), saat menyimpan berikutnya island **memindahkan** file itu ke `config.json.bak` (atau `.bak2`, `.bak3`… kalau sudah ada), menulis `config.json` baru, dan menampilkan notice "config.json tidak valid". Klik Hubungkan Google dengan file yang tidak valid juga memunculkan notice itu. Salin lagi isianmu dari file `.bak` ke file baru.

| Field | Default | Keterangan |
|---|---|---|
| `style` | `"Pill"` | `"Pill"` atau `"Notch"` |
| `battery_threshold` | `20` | Peringatan saat baterai ≤ nilai ini (%) dan tidak di-charge |
| `start_with_windows` | `true` | Sama dengan toggle di tray |
| `muted_apps` | `["Claude", "Orca"]` | Toast dari aplikasi yang namanya mengandung salah satu teks ini (huruf besar/kecil diabaikan) tidak ditampilkan. Default-nya menyaring Claude desktop dan Orca karena notifikasi Claude sudah datang dari hook |
| `onboarding_done` | `false` | Set `false` untuk mengulang langkah pertama jalan |
| `pomodoro` | `{"focus_min": 25, "short_break_min": 5, "long_break_min": 15, "sessions": 4}` | Diatur dari halaman Pomodoro. Nilai di luar rentang tombol −/+ dikembalikan ke batasnya |
| `google_client_id` | `""` | Client ID OAuth Desktop app (lihat Setup Google Calendar) |
| `google_client_secret` | `""` | Client secret pasangannya |

Kalau file sudah tidak valid saat island start, island memakai default untuk semua field. Kalau file baru rusak saat island sedang jalan, island tetap memakai pengaturan yang sedang dipakainya. Field yang tidak ada (misalnya config dari v1) memakai default-nya.

## Mode event untuk notifikasi (opsional)

Tanpa package identity, island membaca daftar notifikasi **setiap 1 detik** (polling). Biayanya sekitar 15–30 ms CPU per detik saat idle (±2–3% satu core). Fitur lain tidak terpengaruh.

Kalau mau idle mendekati 0, exe bisa diberi package identity lewat sparse package (khusus [build dari source](#build-dari-source): package-nya menunjuk ke `target\release`, bukan ke hasil install script); island lalu otomatis berlangganan event `NotificationChanged` dan hanya membaca saat ada perubahan (plus satu pembacaan cadangan tiap 60 detik):

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

## Pemakaian resource

Diukur di build release v2 (polling notifikasi 1 detik, tanpa package identity):

| Kondisi | Hasil |
|---|---|
| Idle (notch kosong), 30 detik | 0,55–0,78 detik CPU (±2–3% satu core, hampir semuanya polling notifikasi); thread UI 0 ms |
| RAM saat start | ~34 MB working set / ~9 MB private |
| RAM setelah panel, jendela note/acara, dan notifikasi dipakai | ~49–52 MB working set / ~20–23 MB private |
| Pomodoro jalan, tampilan kecil, 30 detik | 0,81 detik CPU; thread UI 47 ms (satu gambar ulang per detik) |

Saat idle tidak ada gambar ulang sama sekali. Equalizer musik menggambar ulang ~30 kali per detik hanya selama lagu diputar dan island dalam tampilan kecil musik; CPU-nya saat itu belum diukur.

## Troubleshooting

- **Notifikasi tidak masuk ke island:** cek izin di Settings → Privacy & security → Notifications (akses notifikasi untuk aplikasi). Lalu jalankan probe dari terminal:
  ```powershell
  & "$env:LOCALAPPDATA\Programs\DynamicIsland\dynamic-island.exe" --probe-notifications
  ```
  (Kalau build sendiri: `.\target\release\dynamic-island.exe --probe-notifications`.)
  Probe meminta izin kalau belum ada, mencetak apakah mode event aktif (`Change events: subscribed` / `unavailable`), lalu notifikasi yang masuk selama 60 detik. Kirim toast uji dari terminal lain dengan `powershell -ExecutionPolicy Bypass -File .\scripts\send-test-toast.ps1` (tambahkan `-Long` di belakangnya untuk teks 300 karakter dengan emoji).
- **Install gagal "Rilis ... tidak punya dynamic-island.exe" atau checksum tidak cocok:** coba lagi beberapa menit kemudian (rilis mungkin baru diunggah), atau unduh zip dari Releases secara manual.
- **Probe khusus build debug** (`cargo build`, lalu `.\target\debug\dynamic-island.exe <arg>`):
  - `--probe-media`: cetak media yang sedang diputar setiap kali berubah.
  - `--probe-system`: cetak status baterai dan fullscreen setiap detik.
  - `--demo`: putar semua bentuk island, termasuk panel, pomodoro, dan notes dengan data contoh (Pill lalu Notch) setiap 2 detik. Jalan terus sampai prosesnya dimatikan.
  - `--demo-editor <jenis>`: buka satu jendela editor dengan data contoh. Jenisnya `note`, `note-new`, `event`, `event-new`, atau `event-ro`.

  Hentikan dengan Ctrl+C, atau `Stop-Process -Name dynamic-island` (ini ikut menghentikan island utama kalau sedang jalan).
- **Island tidak terlihat sama sekali:** notch kosong selalu tampil kecuali saat ada aplikasi fullscreen. Kalau tidak ada, cek tray icon; hanya satu instance yang bisa jalan.
- **Hook Claude tidak memunculkan apa-apa:** pastikan island jalan dan `settings.json` berisi hook dengan path exe yang benar (pasang ulang dari tray).
- **"Isi Google client ID dulu":** `google_client_id` di `config.json` masih kosong, atau filenya belum disimpan. Isi, simpan, lalu klik Hubungkan Google lagi.
- **Kalender tetap "Hubungkan Google" setelah login, atau Google menampilkan halaman error (misalnya "Access blocked" / Error 403: access_denied):** penyebab yang paling sering adalah **akun belum ditambahkan sebagai test user** (Google Auth Platform → Audience → Test users). Cek juga client ID dan secret tersalin utuh, tipenya **Desktop app**, dan Google Calendar API sudah di-enable di project yang sama. Setelah diperbaiki, klik Hubungkan Google lagi (kalau login sebelumnya masih ditunggu, klik lagi setelah 5 menit).
- **"Izin Google belum lengkap":** di layar izin Google, salah satu dari dua kotak centang (lihat langkah 7 di Setup Google Calendar) tidak dicentang, jadi island tidak menyimpan login itu. Klik Hubungkan Google lagi dan centang **keduanya**. Kalau kotak centangnya tidak muncul lagi, cabut dulu akses app-nya di [myaccount.google.com/permissions](https://myaccount.google.com/permissions).
- **"Google menolak":** Google menjawab tapi menolak permintaannya; isi notice-nya adalah pesan dari Google. Kalender tetap menampilkan data terakhir dan tombol Hubungkan Google. Dari pembaruan otomatis, notice yang sama hanya muncul sekali selama island jalan (muncul lagi setelah Hubungkan Google berhasil); dari klik Hubungkan Google, notice muncul setiap kali. Kalau isinya "Client ID atau secret ditolak — cek config.json", salin ulang keduanya dari halaman client di Google Cloud Console.
- **"Google Calendar API belum diaktifkan — lihat README":** Google Calendar API belum di-enable di project yang punya client ID-mu. Ulangi langkah 2 Setup Google Calendar di project yang **sama**, tunggu beberapa menit, lalu klik Hubungkan Google.
- **"config.json tidak valid":** ada salah ketik di `config.json`, biasanya koma yang ketinggalan atau berlebih. Perbaiki salah ketiknya; kalau island sudah membuat `config.json.bak` dan menulis file baru, salin field-mu dari `.bak` ke `config.json`.
- **Kalender bertanda "offline":** island gagal mengambil data (tidak ada internet, Wi-Fi hotel minta login, Google sedang bermasalah, atau batas permintaan Google sedang tercapai). Penolakan dari Google tidak tampil sebagai offline, lihat "Google menolak". Satu kalender lain (bukan kalender utama) yang gagal dimuat dilewati saja; acara kalender lainnya tetap tampil. Data terakhir tetap tampil; island mencoba lagi saat halaman Kalender dibuka atau tiap 10 menit.
- **Animasi terasa patah-patah:** keluar dari island lewat tray, lalu jalankan `$env:DI_FRAME_LOG = "1"; & "$env:LOCALAPPDATA\Programs\DynamicIsland\dynamic-island.exe"` di PowerShell. Jarak antar frame setiap animasi (ms) ditulis ke `%TEMP%\di-frames.log`.

## Batasan

- Hanya monitor utama.
- Tidak ada balas langsung atau tombol aksi untuk notifikasi (API resmi tidak menyediakannya).
- Aplikasi yang tidak memakai toast Windows (kemungkinan chat Steam) tidak tertangkap.
- Kalender hanya Google Calendar (bukan Outlook/Microsoft 365 atau iCal). Dari island tidak bisa menghapus acara, mengundang peserta, mengatur pengingat, atau membuat/mengubah acara berulang dan acara beberapa hari.
- Notes hanya lokal, tidak disinkron.
- Pomodoro tidak punya statistik/riwayat dan tidak bertahan setelah island ditutup.
