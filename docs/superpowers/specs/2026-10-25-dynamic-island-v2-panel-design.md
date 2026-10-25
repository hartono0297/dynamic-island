# Dynamic Island v2: Panel, Kalender Google, Pomodoro, Notes — Desain

Tanggal: 2026-10-25. Melanjutkan desain v1: `docs/superpowers/specs/2026-10-25-dynamic-island-design.md` (semua aturan v1 tetap berlaku kecuali yang diubah di sini). Kode v1 ada di `main` (Rust, `windows` 0.62, jendela layered + Direct2D, reducer `IslandState`).

## Intent

Island v1 baru muncul kalau ada musik atau notifikasi. Pemilik ingin island jadi "pusat kecil" yang selalu ada: notch hitam kosong yang selalu terlihat, dan kalau di-hover membesar jadi panel berisi beberapa halaman (kalender Google, pomodoro, notes, musik) yang diganti dengan scroll mouse. Pemilik juga merasa animasi membesar/mengecil di v1 kurang mulus.

## Goals

1. **Notch selalu terlihat.** Saat tidak ada apa-apa, island tampil sebagai notch/pill hitam kosong (ukuran compact). Island hanya hilang saat aplikasi fullscreen (game, nonton fullscreen, presentasi); aturan v1 "notifikasi baru tetap muncul sebentar saat fullscreen" tetap berlaku.
2. **Panel saat hover.** Hover di island → membesar jadi panel (sekitar 560 × 290 logical px, sesuai mockup). Scroll roda mouse saat panel terbuka → pindah halaman. Titik indikator halaman di bawah panel. Mouse keluar → kembali kecil. Panel membuka halaman terakhir yang dilihat.
3. **Halaman, urutan tetap:** (1) Kalender, (2) Pomodoro, (3) Notes, (4) Musik.
4. **Kalender Google — baca, tambah, edit.**
   - Login Google sekali (OAuth lewat browser), token disimpan di komputer.
   - Tampilan **Hari / Minggu / Bulan** (tombol segmen di kanan atas). Bulan: grid tanggal, tanggal yang punya acara diberi titik, klik tanggal → daftar acara tanggal itu di sisi kanan. Minggu: 7 kolom/baris dengan acara per hari. Hari: daftar acara berurutan jam.
   - Navigasi ke periode sebelum/berikutnya (mis. panah di judul).
   - **Tambah** acara ("+ Tambah acara") dan **edit** acara (klik acara) lewat jendela kecil di bawah notch: judul, tanggal, jam mulai, jam selesai (atau "seharian"), catatan. Simpan → dikirim ke Google Calendar. Hapus acara tidak diminta (lihat non-goals).
   - Data selalu terbaru: ambil dari Google API (bukan link iCal).
5. **Pomodoro.**
   - Durasi fokus, istirahat, istirahat panjang, dan jumlah sesi per siklus diatur langsung di halaman Pomodoro dengan tombol − / +; tersimpan untuk lain kali. Default 25 / 5 / 15 menit, 4 sesi.
   - Halaman Pomodoro: **lingkaran progres besar** dengan sisa waktu di tengah, label fase (Fokus / Istirahat / Istirahat panjang), "Sesi n dari N", tombol Mulai/Jeda, Reset, Lewati.
   - Saat timer jalan dan island kecil: **lingkaran progres kecil** + sisa waktu (mm:ss) + label singkat. Merah = fokus, hijau = istirahat. (Pemilik minta lingkaran juga di tampilan kecil.)
   - Fase selesai: island menampilkan notice sementara ("Fokus selesai — istirahat 5 menit" / "Istirahat selesai"), bunyi notifikasi Windows sekali, dan **fase berikutnya baru mulai setelah diklik** (tidak jalan sendiri).
6. **Notes custom.**
   - Halaman Notes: daftar notes (teks + waktu terakhir diubah), terbaru di atas, bisa digulir kalau lebih dari muat (gulir di dalam halaman ini tidak boleh bentrok dengan scroll pindah halaman — builder menentukan mekanismenya, lihat bawah).
   - "+ Baru" atau klik note → jendela kecil bertema gelap di bawah notch dengan kotak teks multi-baris biasa (copy-paste, undo, emoji, IME jalan). Tutup jendela → tersimpan otomatis. Ada cara menghapus note.
   - Disimpan lokal di `%APPDATA%\DynamicIsland\` (tidak disinkron ke mana pun).
7. **Musik** jadi halaman ke-4, isi sama seperti v1 tapi di panel besar.
8. **Animasi lebih mulus.** Membesar/mengecil dan pindah halaman harus terasa halus tanpa patah-patah.

## Non-goals (v2)

- Outlook/Microsoft 365, iCal, atau kalender lain selain Google.
- Menghapus acara Google, mengundang peserta, pengingat, acara berulang yang dibuat/diubah dari island (acara berulang yang sudah ada tetap ditampilkan).
- Sinkron notes ke cloud.
- Statistik pomodoro, riwayat, atau pomodoro yang bertahan setelah island ditutup.
- Multi-monitor (tetap monitor utama).

## Anti-goals

- Island jadi berat: v1 idle ~29 MB RAM; v2 boleh naik tapi tetap di "puluhan MB". Tidak boleh ada animasi/redraw terus-menerus saat idle; pomodoro yang jalan boleh redraw paling sering 1× per detik di tampilan kecil.
- Notch kosong yang selalu terlihat menghalangi kerja: di luar bentuk island tetap klik-tembus; notch kosong tidak boleh lebih besar dari ukuran compact v1.
- Kebocoran kredensial Google: token tidak disimpan sebagai teks biasa dan tidak pernah dicetak ke log/console.
- Panel terbuka tidak sengaja saat mouse sekadar lewat (pertahankan dwell hover ~350 ms dari v1).

## Keputusan yang sudah diambil

| Keputusan | Diputuskan oleh | Catatan |
|---|---|---|
| Notch hitam kosong selalu terlihat, hilang hanya saat fullscreen | Pemilik | |
| Hover → panel; scroll → pindah halaman | Pemilik | Scroll untuk pindah halaman, bukan menggulir isi |
| Halaman: Kalender, Pomodoro, Notes, Musik | Pemilik (isi), Claude (urutan) | |
| Kalender = Google Calendar lewat API resmi, baca + tambah + edit | Pemilik | Scope OAuth: `https://www.googleapis.com/auth/calendar.events` (lihat + ubah acara). Tambahan `calendar.readonly` atau `calendar.calendarlist.readonly` jika dibutuhkan untuk daftar kalender — builder memilih scope minimum |
| Tampilan kalender Hari / Minggu / Bulan | Pemilik | |
| Pomodoro dengan lingkaran progres, di panel dan di tampilan kecil | Pemilik | |
| Durasi fokus/istirahat/istirahat panjang/jumlah sesi diatur dari island dengan − / + | Pemilik | |
| Detail perilaku pomodoro (default 25/5/15/4, fase berikutnya mulai setelah klik, bunyi sekali) | Claude (pemilik menyerahkan) | |
| Notes: tampil di island, ditulis di jendela kecil terpisah | Claude (pemilik menyerahkan) | Alasan: jendela layered island tidak bisa memuat kontrol teks native; membuat editor teks sendiri berat dan rawan bug |
| Tambah/edit acara juga lewat jendela kecil yang sama gayanya | Claude | |
| Mode notifikasi tetap polling 1 s tanpa sertifikat | Pemilik | Pemilik memilih jalan tanpa sertifikat untuk saat ini |
| Dua gaya v1 (Pill dan Notch) tetap ada dan tetap dipilih di menu tray; semua kondisi baru (notch kosong, panel, pomodoro kecil) mengikuti gaya yang dipilih: Pill = melayang 8 px dengan semua sudut bulat, Notch = menempel ke tepi atas dengan bahu melengkung | Pemilik | "Tergantung setting user" |
| Setelah fase pomodoro selesai: notice sementara 5 s, lalu tampilan kecil menetap di "Istirahat siap · klik untuk mulai" (atau "Fokus siap…") sampai pemilik klik island kecil atau tombol Mulai di panel. Pomodoro yang dijeda tetap dianggap "pomodoro jalan" di urutan prioritas (waktu berhenti) | Claude | Jawaban atas pertanyaan reviewer |
| Hover saat ada notice: notice sementara (5 s) menahan panel supaya bisa diklik; "Claude menunggu" (sticky) tidak menahan — hover tetap membuka panel dan notice itu tampil sebagai strip kecil di atas panel | Claude | |
| Selama jendela notes/edit acara terbuka, panel tetap terbuka (dihitung seperti hover); jendela menempel di bawah panel; setelah ditutup, fokus kembali ke aplikasi sebelumnya dan panel mengikuti aturan hover biasa | Claude | |

## Perilaku detail

- **Prioritas tampilan kecil (dari tertinggi):** notice sementara (notif app, Claude, baterai, pomodoro selesai) → Claude menunggu (sticky, aturan v1) → pomodoro jalan → musik → notch kosong.
- **Panel terbuka + notif masuk:** notif tampil sementara di atas panel (builder menentukan caranya, mis. banner di dalam panel atau panel kembali kecil lalu notice), lalu panel kembali seperti semula jika mouse masih di atasnya.
- **Fullscreen:** semua ngumpet (termasuk notch kosong dan panel); notice baru tetap muncul sebentar.
- **Kalender offline / belum login:** halaman Kalender menampilkan tombol "Hubungkan Google" (belum login) atau data terakhir yang tersimpan dengan tanda "offline" (gagal ambil data). Gagal simpan acara → pesan jelas di jendela edit, input tidak hilang.
- **Pembaruan kalender:** ambil ulang saat panel kalender dibuka (jika data lebih tua dari ~1 menit) dan berkala saat island jalan (builder menentukan interval yang hemat, mis. 5–15 menit). Zona waktu = zona waktu Windows.
- **Kalender yang ditampilkan:** acara dari kalender-kalender yang dicentang ("selected") di Google Calendar milik pemilik; acara baru masuk ke kalender utama (primary).

## Setup Google (sekali, oleh pemilik)

- Pemilik membuat project di Google Cloud Console, mengaktifkan Google Calendar API, mengatur OAuth consent screen (tipe External, pemilik sebagai test user / atau dipublikasikan), dan membuat OAuth client tipe **Desktop app**. Client ID (dan client secret, yang untuk desktop app tidak benar-benar rahasia) dimasukkan ke island — builder menentukan caranya (mis. `config.json` atau dialog sekali). README memandu langkah-langkahnya.
- **Batasan Google yang sudah dicek di dokumentasi:** project dengan publishing status **"Testing"** mendapat refresh token yang **kedaluwarsa 7 hari** untuk scope kalender. Konsekuensi: pemilik harus login ulang tiap minggu, kecuali project dipindah ke status "In production" (untuk pemakaian pribadi; Google menampilkan layar "aplikasi belum diverifikasi"). Island harus mendeteksi token kedaluwarsa dan menampilkan "Hubungkan ulang Google", bukan gagal diam-diam.
- Alur login: OAuth 2.0 untuk aplikasi desktop dengan redirect **loopback** (`http://127.0.0.1:<port>`), PKCE, dibuka di browser default.

## Constraints

- Tetap Rust + `windows` crate, tanpa browser engine/Electron/.NET. Dependensi baru seminimal mungkin; HTTPS bisa lewat WinRT `Windows.Web.Http.HttpClient` atau crate kecil — builder memilih dengan alasan ringan.
- Token Google disimpan terenkripsi per-user (mis. DPAPI `CryptProtectData`) di `%APPDATA%\DynamicIsland\`.
- Jendela island sekarang 440 × 150 logical px; harus diperbesar agar panel (± bahu notch dan bayangan) muat. Area di luar bentuk tetap alpha 0 / klik-tembus.
- Jendela kecil (notes, edit acara) adalah jendela Win32 biasa yang bisa difokus, tema gelap, muncul tepat di bawah notch; menutupnya mengembalikan fokus ke aplikasi sebelumnya.
- Scroll roda mouse harus sampai ke island walau island tidak aktif/terfokus (Windows 11 default "scroll inactive windows" — builder memastikan).
- Semua teks UI berbahasa Indonesia.

## Diserahkan ke builder

- Ukuran persis panel dan jendela, tata letak tiap halaman (ikuti mockup `.superpowers/brainstorm/1823-1792909126/content/panel-pages.html` untuk isi, dan `pill-vs-notch.html` untuk bentuk kedua gaya).
- Penyebab dan perbaikan animasi kurang mulus (dugaan: timer 16 ms yang tidak presisi + gambar ulang penuh di CPU tiap frame). Kriteria: terukur — catat waktu antar frame saat animasi dan tunjukkan tidak ada frame yang meleset jauh (mis. tidak ada jeda > 25 ms pada 60 Hz).
- Animasi pindah halaman (geser/fade).
- Cara menggulir daftar notes yang panjang tanpa bentrok dengan scroll pindah halaman (mis. scroll pindah halaman hanya saat kursor di area judul/titik, atau Shift+scroll, atau daftar notes dibatasi dan "Lihat semua" membuka jendela).
- Format file notes, struktur modul baru, interval sinkron kalender, cara memasukkan client ID Google.
