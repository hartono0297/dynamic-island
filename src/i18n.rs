//! The island's two UI languages. Every user-facing string comes from here:
//! fixed text through `Key` (`tr` for the current language, `text` for a
//! given one), text with numbers or names through the small helpers below.
//!
//! The current language is one global, set by the app from `config.language`
//! at startup and when it changes. Strings are looked up when a frame is drawn
//! or a notice is made, so a change shows on the next redraw; notices already
//! made keep their text. Hook and CLI output stays Indonesian.

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    /// Bahasa Indonesia, the default.
    #[default]
    Id,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Id => "id",
            Lang::En => "en",
        }
    }

    /// "id" / "en", any case, with surrounding blanks.
    pub fn from_code(code: &str) -> Option<Lang> {
        match code.trim().to_ascii_lowercase().as_str() {
            "id" => Some(Lang::Id),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

impl Serialize for Lang {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

/// An unknown code (a typo in config.json) is the default language rather
/// than an invalid config, which would be backed up and replaced.
impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let code = String::deserialize(d)?;
        Ok(Lang::from_code(&code).unwrap_or_default())
    }
}

#[cfg(not(test))]
mod current {
    use std::sync::atomic::{AtomicU8, Ordering};

    static LANG: AtomicU8 = AtomicU8::new(0);

    pub fn get() -> u8 {
        LANG.load(Ordering::Relaxed)
    }

    pub fn set(v: u8) {
        LANG.store(v, Ordering::Relaxed);
    }
}

/// Tests run on parallel threads: each gets its own language, so a test that
/// switches to English never changes the strings another test expects.
#[cfg(test)]
mod current {
    use std::cell::Cell;

    thread_local! {
        static LANG: Cell<u8> = const { Cell::new(0) };
    }

    pub fn get() -> u8 {
        LANG.with(Cell::get)
    }

    pub fn set(v: u8) {
        LANG.with(|l| l.set(v));
    }
}

pub fn set_lang(lang: Lang) {
    current::set(match lang {
        Lang::Id => 0,
        Lang::En => 1,
    });
}

pub fn lang() -> Lang {
    match current::get() {
        1 => Lang::En,
        _ => Lang::Id,
    }
}

/// Declares `Key`, `Key::ALL` and the table of both translations, so a key
/// without one of them does not compile.
macro_rules! keys {
    ($($name:ident => $id:literal, $en:literal;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Key {
            $($name,)*
        }

        impl Key {
            #[cfg_attr(not(test), allow(dead_code))]
            pub const ALL: &[Key] = &[$(Key::$name,)*];
        }

        fn pair(key: Key) -> (&'static str, &'static str) {
            match key {
                $(Key::$name => ($id, $en),)*
            }
        }
    };
}

keys! {
    // Notice sources (their first letter is the badge).
    SourceCalendar => "Kalender", "Calendar";
    SourceBattery => "Baterai", "Battery";
    SourceNotification => "Notifikasi", "Notification";
    SourceReminder => "Pengingat", "Reminder";

    // Island notices.
    ClaudeDone => "Claude selesai", "Claude finished";
    ClaudeWaiting => "Claude butuh respon", "Claude needs you";
    BatteryLow => "Baterai lemah", "Low battery";
    FocusDone => "Fokus selesai", "Focus done";
    BreakDone => "Istirahat selesai", "Break done";
    NotesSaveFailed => "Gagal simpan catatan", "Couldn't save notes";
    NotesUnreadable => "notes.json tidak terbaca", "Can't read notes.json";
    NotesUnreadableBody => "Disimpan sebagai notes.json.bak saat menyimpan berikutnya",
        "Will be saved as notes.json.bak on the next save";
    Missed => "Terlewat", "Missed";
    MarkDone => "Selesai", "Done";
    // The Claude notice's chip.
    Open => "Buka", "Open";
    RemindersSaveFailed => "Gagal simpan pengingat", "Couldn't save reminders";
    RemindersUnreadable => "reminders.json tidak terbaca", "Can't read reminders.json";
    RemindersUnreadableBody => "Disimpan sebagai reminders.json.bak saat menyimpan berikutnya",
        "Will be saved as reminders.json.bak on the next save";
    ConfigInvalid => "config.json tidak valid", "config.json is invalid";
    ConfigNotCopied => "Tidak bisa disalin — cek lagi isinya",
        "Couldn't copy it — check its contents";
    ConfigCheck => "Cek lagi isinya — lihat README", "Check its contents — see the README";
    ClientIdMissing => "Isi Google client ID dulu", "Add a Google client ID first";
    SeeReadme => "Lihat README", "See the README";
    DndTitle => "Matiin popup Windows", "Turn off Windows pop-ups";
    DndBody => "Nyalain Do Not Disturb: Settings → System → Notifications",
        "Turn on Do Not Disturb: Settings → System → Notifications";
    UserProfileUnset => "USERPROFILE tidak diset", "USERPROFILE is not set";

    // Calendar and Google.
    SaveEventFailed => "Gagal simpan acara", "Couldn't save event";
    GoogleScopes => "Izin Google belum lengkap", "Google permissions incomplete";
    GoogleScopesBody => "Centang kedua izin saat login", "Check both permissions when you sign in";
    GoogleRefused => "Google menolak", "Google refused";
    ClientRejected => "Client ID atau secret ditolak — cek Pengaturan → Integrasi",
        "Client ID or secret rejected — check Settings → Integrations";
    ApiDisabled => "Google Calendar API belum diaktifkan — lihat README",
        "Google Calendar API isn't enabled — see the README";
    NoTitle => "(Tanpa judul)", "(No title)";
    SaveFailed => "Gagal simpan", "Couldn't save";
    ReasonNotConfigured => "Google belum diatur", "Google isn't set up";
    ReasonNeedsLogin => "hubungkan Google dulu", "connect Google first";
    ReasonReconnect => "hubungkan ulang Google", "reconnect Google";
    ReasonScopes => "izin Google belum lengkap", "Google permissions incomplete";
    ReasonOffline => "tidak ada koneksi", "no connection";
    ReasonSyncStopped => "sinkron kalender berhenti", "calendar sync stopped";
    OauthConnected => "Berhasil terhubung. Kembali ke island ya.",
        "Connected. You can go back to the island.";
    OauthFailed => "Gagal terhubung — coba lagi dari island.",
        "Couldn't connect — try again from the island.";
    OauthScopes => "Izin Google belum lengkap — centang kedua izin, lalu coba lagi dari island.",
        "Google permissions incomplete — check both permissions, then try again from the island.";

    // Calendar page.
    Offline => "offline", "offline";
    ViewDay => "Hari", "Day";
    ViewWeek => "Minggu", "Week";
    ViewMonth => "Bulan", "Month";
    BackToTop => "Kembali ke atas", "Back to top";
    AddEventRow => "+ Tambah acara", "+ Add event";
    AddEvent => "Tambah acara", "Add event";
    ConnectGoogle => "Hubungkan Google", "Connect Google";
    ReconnectGoogle => "Hubungkan ulang Google", "Reconnect Google";
    AllDay => "Seharian", "All day";

    // Pomodoro page and compact view.
    Pomodoro => "Pomodoro", "Pomodoro";
    Focus => "Fokus", "Focus";
    Break => "Istirahat", "Break";
    LongBreak => "Istirahat panjang", "Long break";
    Sessions => "Jumlah sesi", "Sessions";
    MinutesShort => "mnt", "min";
    Start => "Mulai", "Start";
    Pause => "Jeda", "Pause";
    Resume => "Lanjut", "Resume";
    Reset => "Reset", "Reset";
    Skip => "Lewati", "Skip";
    Paused => "dijeda", "paused";
    Ready => "siap", "ready";
    ReadyClick => "Siap · klik", "Ready · click";
    NextUp => "Berikutnya:", "Next:";

    // Reminders page.
    Reminders => "Pengingat", "Reminders";
    RemindersEmpty => "Belum ada pengingat. Klik + Baru untuk menambah.",
        "No reminders yet. Click + New to add one.";
    DayToday => "Hari ini", "Today";
    DayTomorrow => "Besok", "Tomorrow";
    RepeatOnce => "Sekali", "Once";
    RepeatDaily => "Harian", "Daily";
    RepeatWeekly => "Mingguan", "Weekly";

    // Notes page.
    Notes => "Notes", "Notes";
    NewNote => "+ Baru", "+ New";
    New => "Baru", "New";
    NotesEmpty => "Belum ada catatan. Klik + Baru untuk menulis.",
        "No notes yet. Click + New to write one.";
    Today => "hari ini", "Today";
    Yesterday => "kemarin", "Yesterday";

    // Media page.
    NothingPlaying => "Tidak ada musik yang diputar", "Nothing playing";
    NothingPlayingHint => "Putar lagu di aplikasi mana pun", "Play music in any app";
    LyricsButton => "Lirik", "Lyrics";
    NowPlaying => "Sedang diputar", "Now playing";

    // CLI page and window.
    Cli => "CLI", "CLI";
    CliTypeCommand => "Ketik perintah", "Type a command";
    CliRunning => "berjalan", "running";
    CliEmpty => "Belum ada perintah", "No commands yet";
    CliEmptyHint => "Perintah jalan di folder ini; cd untuk pindah", "Commands run in this folder; cd to move";
    CliStop => "Hentikan", "Stop";
    CliStopped => "dihentikan", "stopped";
    CliBusy => "Masih ada perintah berjalan — Ctrl+C untuk menghentikan",
        "A command is still running — Ctrl+C to stop it";

    // Shortcut page.
    Shortcuts => "Pintasan", "Shortcuts";
    ShortcutScreenshot => "Screenshot", "Screenshot";
    ShortcutClipboard => "Clipboard", "Clipboard";
    ShortcutEmoji => "Emoji", "Emoji";
    ShortcutExplorer => "Explorer", "Explorer";
    ShortcutCalculator => "Kalkulator", "Calculator";
    ShortcutTaskManager => "Task Manager", "Task Manager";
    ShortcutSettings => "Pengaturan Windows", "Windows Settings";
    ShortcutLock => "Kunci layar", "Lock screen";

    // Editors.
    EditorNote => "Note", "Note";
    EditorEvent => "Acara", "Event";
    EditorAddEvent => "Tambah acara", "Add event";
    EditorEditEvent => "Edit acara", "Edit event";
    FieldTitle => "Judul", "Title";
    FieldDate => "Tanggal", "Date";
    FieldStart => "Mulai", "Start";
    FieldEnd => "Selesai", "End";
    FieldNotes => "Catatan", "Notes";
    Delete => "Hapus", "Delete";
    Save => "Simpan", "Save";
    Cancel => "Batal", "Cancel";
    Close => "Tutup", "Close";
    Saving => "Menyimpan…", "Saving…";
    TitleRequired => "Judul wajib diisi", "Title is required";
    EndBeforeStart => "Jam selesai harus setelah jam mulai", "End time must be after start time";
    EditorNewReminder => "Pengingat baru", "New reminder";
    EditorEditReminder => "Edit pengingat", "Edit reminder";
    FieldText => "Teks", "Text";
    FieldTime => "Jam", "Time";
    FieldRepeat => "Ulangi", "Repeat";
    TextRequired => "Teks wajib diisi", "Text is required";
    TimePassed => "Waktunya sudah lewat", "That time has passed";

    // Tray menu.
    MenuStylePill => "Gaya: Pill", "Style: Pill";
    MenuStyleNotch => "Gaya: Notch", "Style: Notch";
    MenuExit => "Keluar", "Quit";
    MenuSettings => "Pengaturan…", "Settings…";

    // Settings window.
    SettingsTitle => "Pengaturan Dynamic Island", "Dynamic Island Settings";
    SecAppearance => "Tampilan", "Appearance";
    SecGeneral => "Umum", "General";
    SecPomodoro => "Pomodoro", "Pomodoro";
    SecIntegrations => "Integrasi", "Integrations";
    SecAbout => "Tentang", "About";
    PreviewTrack => "Sedang diputar", "Now playing";
    IslandStyle => "Gaya island", "Island style";
    IslandStyleDesc => "Bentuk island di atas layar", "Its shape at the top of the screen";
    StylePill => "Pill", "Pill";
    StyleNotch => "Notch", "Notch";
    AccentColor => "Warna aksen", "Accent color";
    AccentTerracotta => "Terakota", "Terracotta";
    AccentSage => "Sage", "Sage";
    AccentOchre => "Oker", "Ochre";
    AccentClay => "Lempung", "Clay";
    AccentOlive => "Zaitun", "Olive";
    AccentPlum => "Plum", "Plum";
    IslandSurface => "Latar island", "Island fill";
    IslandSurfaceDesc => "Warna dasar island", "The island's base color";
    SurfaceEspresso => "Espresso", "Espresso";
    SurfaceGraphite => "Grafit", "Graphite";
    WindowMode => "Mode jendela", "Window mode";
    WindowModeDesc => "Pengaturan dan editor", "Settings and editors";
    ModeSystem => "Ikuti Windows", "Follow Windows";
    ModeDark => "Gelap", "Dark";
    ModeLight => "Terang", "Light";
    IdleClock => "Jam saat kosong", "Clock when idle";
    IdleClockDesc => "Tampilkan jam saat island kosong", "Show the time when the island is idle";
    Language => "Bahasa", "Language";
    LanguageDesc => "Island dan semua jendela", "The island and every window";
    LangIndonesian => "Indonesia", "Indonesia";
    LangEnglish => "English", "English";
    Autostart => "Jalan saat Windows start", "Start with Windows";
    AutostartDesc => "Buka island otomatis saat masuk", "Open the island when you sign in";
    BatteryAlert => "Peringatan baterai", "Battery alert";
    BatteryAlertDesc => "Kasih tahu saat baterai di bawah ini", "Warn when the charge drops below";
    NoBattery => "Tidak ada baterai", "No battery found";
    PopupsHelp => "Cara matikan popup Windows", "Turn off Windows pop-ups";
    PopupsHelpDesc => "Supaya notifikasi tidak muncul dua kali", "So notifications don't show twice";
    ShowHow => "Buka petunjuk", "Show me how";
    LyricsInNotch => "Lirik di notch", "Lyrics in notch";
    LyricsInNotchDesc => "Artis, judul & durasi dikirim ke lrclib.net", "Artist, title & length are sent to lrclib.net";
    ShellCli => "Shell CLI", "CLI shell";
    ShellCliDesc => "Untuk perintah di halaman CLI", "Runs the CLI page's commands";
    ShellPowerShell => "PowerShell", "PowerShell";
    ShellCmd => "cmd", "cmd";
    FocusDesc => "Lama satu sesi fokus", "Length of one focus session";
    BreakDesc => "Jeda di antara sesi", "Pause between sessions";
    LongBreakDesc => "Setelah sesi terakhir", "After the last session";
    SessionsDesc => "Sesi fokus sebelum istirahat panjang", "Focus sessions before a long break";
    PomodoroLocked => "Pomodoro sedang berjalan — reset dulu untuk mengubah ini.",
        "A pomodoro is running — reset it to change these.";
    GoogleCalendar => "Google Calendar", "Google Calendar";
    StatusNotConfigured => "Belum diatur", "Not set up";
    StatusNeedsLogin => "Belum terhubung", "Not connected";
    StatusConnected => "Terhubung", "Connected";
    StatusReconnect => "Perlu login ulang", "Sign in again";
    StatusOffline => "Offline", "Offline";
    ClientId => "Client ID", "Client ID";
    ClientSecret => "Client secret", "Client secret";
    SetupGuide => "Panduan setup", "Setup guide";
    Connect => "Hubungkan", "Connect";
    Disconnect => "Putuskan", "Disconnect";
    ConfirmShort => "Yakin?", "Sure?";
    Saved => "Tersimpan", "Saved";
    CheckBrowser => "Cek browser…", "Check your browser…";
    Disconnected => "Terputus", "Disconnected";
    ClaudeCode => "Claude Code", "Claude Code";
    HooksOn => "Hook terpasang", "Hooks installed";
    HooksOff => "Hook belum terpasang", "Hooks not installed";
    HooksUnknown => "Status tidak diketahui", "Status unknown";
    InstallHooks => "Pasang hook", "Install hooks";
    RemoveHooks => "Hapus hook", "Remove hooks";
    HooksAlready => "Sudah terpasang", "Already installed";
    HooksRemoved => "Hook dihapus", "Hooks removed";
    HooksNone => "Tidak ada hook", "No hooks found";
    Failed => "Gagal", "Failed";
    AboutTagline => "Island ala macOS untuk Windows 11", "A macOS-style island for Windows 11";
    SourceCode => "Kode sumber", "Source code";
    OpenGithub => "Buka GitHub", "Open GitHub";
    Updates => "Pembaruan", "Updates";
    UpdatesDesc => "Cek rilis terbaru di GitHub", "Check for the latest release";
    SeeReleases => "Lihat rilis", "See releases";
    DataFolder => "Folder data", "Data folder";
    OpenDataFolder => "Buka folder data", "Open data folder";
}

/// `key` in `lang`.
pub fn text(lang: Lang, key: Key) -> &'static str {
    let (id, en) = pair(key);
    match lang {
        Lang::Id => id,
        Lang::En => en,
    }
}

/// `key` in the current language.
pub fn tr(key: Key) -> &'static str {
    text(lang(), key)
}

/// The notice after a focus phase: the break that follows.
pub fn focus_done_body(lang: Lang, break_min: u32) -> String {
    match lang {
        Lang::Id => format!("Istirahat {break_min} menit · klik untuk mulai"),
        Lang::En => format!("{break_min}-minute break · click to start"),
    }
}

/// The notice after a break: focus session `k` of `total` comes next.
pub fn break_done_body(lang: Lang, k: u32, total: u32) -> String {
    match lang {
        Lang::Id => format!("Fokus sesi {k} dari {total} · klik untuk mulai"),
        Lang::En => format!("Focus session {k} of {total} · click to start"),
    }
}

/// "Sesi 2 dari 4" in the pomodoro header.
pub fn session_of(lang: Lang, k: u32, total: u32) -> String {
    match lang {
        Lang::Id => format!("Sesi {k} dari {total}"),
        Lang::En => format!("Session {k} of {total}"),
    }
}

/// "25 · 5 · 15 menit": focus, break and long break beside the pomodoro title.
pub fn pomo_durations(lang: Lang, focus: u32, short: u32, long: u32) -> String {
    let unit = pick(lang, "menit", "min");
    format!("{focus} · {short} · {long} {unit}")
}

/// "Istirahat 5 menit", "Break · 5 min": the phase after this one.
pub fn phase_minutes(lang: Lang, phase: &str, minutes: u32) -> String {
    match lang {
        Lang::Id => format!("{phase} {minutes} menit"),
        Lang::En => format!("{phase} · {minutes} min"),
    }
}

/// "+3 lagi" under a list that does not fit.
pub fn more_count(lang: Lang, n: usize) -> String {
    match lang {
        Lang::Id => format!("+{n} lagi"),
        Lang::En => format!("+{n} more"),
    }
}

pub fn battery_body(lang: Lang, percent: u8) -> String {
    match lang {
        Lang::Id => format!("Sisa {percent}%. Colokin charger."),
        Lang::En => format!("{percent}% left. Plug in the charger."),
    }
}

/// An invalid config.json was moved aside to `name`.
pub fn config_copied_to(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Id => format!("Disalin ke {name} — cek lagi isinya"),
        Lang::En => format!("Copied to {name} — check its contents"),
    }
}

/// "Versi 2.1.0" on the settings window's About page.
pub fn version_label(lang: Lang, version: &str) -> String {
    match lang {
        Lang::Id => format!("Versi {version}"),
        Lang::En => format!("Version {version}"),
    }
}

/// The CLI's `cd` to a folder that does not exist.
pub fn cd_not_found(lang: Lang, path: &str) -> String {
    match lang {
        Lang::Id => format!("cd: folder tidak ada: {path}"),
        Lang::En => format!("cd: no such folder: {path}"),
    }
}

/// The CLI could not start the shell.
pub fn cli_start_failed(lang: Lang, error: &str) -> String {
    match lang {
        Lang::Id => format!("Gagal menjalankan: {error}"),
        Lang::En => format!("Couldn't run it: {error}"),
    }
}

/// "Gagal simpan: {reason}".
pub fn save_failed_because(lang: Lang, reason: &str) -> String {
    format!("{}: {reason}", text(lang, Key::SaveFailed))
}

/// "Google menolak ({message})".
pub fn google_refused(lang: Lang, message: &str) -> String {
    format!("{} ({message})", text(lang, Key::GoogleRefused))
}

/// A 2xx answer from Google that is not what was asked for.
pub fn unreadable_reply(lang: Lang, status: u16) -> String {
    match lang {
        Lang::Id => format!("HTTP {status}: jawaban tidak terbaca"),
        Lang::En => format!("HTTP {status}: unreadable response"),
    }
}

/// Claude Code's settings file at `path` does not parse.
pub fn not_valid_json(lang: Lang, path: &str, error: &str) -> String {
    match lang {
        Lang::Id => format!("{path} bukan JSON yang valid: {error}"),
        Lang::En => format!("{path} is not valid JSON: {error}"),
    }
}

/// Claude Code's settings file at `path` has hooks in a shape the island
/// cannot add to.
pub fn unknown_format(lang: Lang, path: &str) -> String {
    match lang {
        Lang::Id => format!("format {path} tidak dikenali"),
        Lang::En => format!("unrecognized format in {path}"),
    }
}

/// The page the browser shows after the Google login.
pub fn oauth_page(lang: Lang, key: Key) -> String {
    format!(
        "<!doctype html><html lang=\"{}\"><meta charset=\"utf-8\"><title>Dynamic Island</title>\
         <p style=\"font:16px sans-serif\">{}</p>",
        lang.code(),
        text(lang, key)
    )
}

const MONTHS_ID: [&str; 12] = [
    "Januari",
    "Februari",
    "Maret",
    "April",
    "Mei",
    "Juni",
    "Juli",
    "Agustus",
    "September",
    "Oktober",
    "November",
    "Desember",
];
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT_ID: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des",
];
const MONTHS_SHORT_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// Monday first.
const DAYS_ID: [&str; 7] = [
    "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu", "Minggu",
];
const DAYS_EN: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const DAYS_SHORT_ID: [&str; 7] = ["Sen", "Sel", "Rab", "Kam", "Jum", "Sab", "Min"];
const DAYS_SHORT_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn pick<T>(lang: Lang, id: T, en: T) -> T {
    match lang {
        Lang::Id => id,
        Lang::En => en,
    }
}

fn weekday(d: NaiveDate) -> usize {
    d.weekday().num_days_from_monday() as usize
}

/// "Sen", "Mon".
pub fn day_short(lang: Lang, d: NaiveDate) -> &'static str {
    pick(lang, DAYS_SHORT_ID, DAYS_SHORT_EN)[weekday(d)]
}

/// "Okt", "Oct".
pub fn month_short(lang: Lang, d: NaiveDate) -> &'static str {
    pick(lang, MONTHS_SHORT_ID, MONTHS_SHORT_EN)[d.month0() as usize]
}

/// "25 Okt", "25 Oct".
pub fn short_date(lang: Lang, d: NaiveDate) -> String {
    let months = pick(lang, MONTHS_SHORT_ID, MONTHS_SHORT_EN);
    format!("{} {}", d.day(), months[d.month0() as usize])
}

/// "Oktober 2026", "October 2026".
pub fn month_title(lang: Lang, d: NaiveDate) -> String {
    let months = pick(lang, MONTHS_ID, MONTHS_EN);
    format!("{} {}", months[d.month0() as usize], d.year())
}

/// "Senin, 26 Oktober", "Monday, 26 October": the day view's title and the
/// month view's caption.
pub fn day_title_long(lang: Lang, d: NaiveDate) -> String {
    let days = pick(lang, DAYS_ID, DAYS_EN);
    let months = pick(lang, MONTHS_ID, MONTHS_EN);
    format!(
        "{}, {} {}",
        days[weekday(d)],
        d.day(),
        months[d.month0() as usize]
    )
}

/// The week from `monday`: "19 – 25 Okt 2026", "28 Sep – 4 Okt 2026",
/// "29 Des 2025 – 4 Jan 2026".
pub fn week_title(lang: Lang, monday: NaiveDate) -> String {
    let sunday = monday + chrono::Days::new(6);
    let end = format!("{} {}", short_date(lang, sunday), sunday.year());
    if monday.year() != sunday.year() {
        format!("{} {} – {end}", short_date(lang, monday), monday.year())
    } else if monday.month() != sunday.month() {
        format!("{} – {end}", short_date(lang, monday))
    } else {
        format!("{} – {end}", monday.day())
    }
}

/// "09.12" in Indonesian, "09:12" in English.
pub fn time_hm(lang: Lang, t: NaiveDateTime) -> String {
    t.format(pick(lang, "%H.%M", "%H:%M")).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn every_key_has_text_in_both_languages() {
        assert!(Key::ALL.len() > 50);
        for &key in Key::ALL {
            for lang in [Lang::Id, Lang::En] {
                let s = text(lang, key);
                assert!(!s.trim().is_empty(), "{key:?} {lang:?}");
                assert_eq!(s, s.trim(), "{key:?} {lang:?} has stray blanks");
            }
        }
    }

    #[test]
    fn indonesian_strings_are_unchanged() {
        assert_eq!(text(Lang::Id, Key::ClaudeWaiting), "Claude butuh respon");
        assert_eq!(text(Lang::Id, Key::AddEventRow), "+ Tambah acara");
        assert_eq!(text(Lang::En, Key::AddEventRow), "+ Add event");
        assert_eq!(text(Lang::En, Key::ReconnectGoogle), "Reconnect Google");
    }

    #[test]
    fn current_language_defaults_to_indonesian_and_switches() {
        assert_eq!(lang(), Lang::Id);
        assert_eq!(tr(Key::BatteryLow), "Baterai lemah");
        set_lang(Lang::En);
        assert_eq!(lang(), Lang::En);
        assert_eq!(tr(Key::BatteryLow), "Low battery");
        set_lang(Lang::Id);
        assert_eq!(tr(Key::BatteryLow), "Baterai lemah");
    }

    #[test]
    fn codes_round_trip_and_unknown_is_indonesian() {
        for lang in [Lang::Id, Lang::En] {
            let json = serde_json::to_string(&lang).unwrap();
            assert_eq!(json, format!("\"{}\"", lang.code()));
            assert_eq!(serde_json::from_str::<Lang>(&json).unwrap(), lang);
        }
        assert_eq!(Lang::from_code(" EN "), Some(Lang::En));
        assert_eq!(Lang::from_code("fr"), None);
        assert_eq!(serde_json::from_str::<Lang>("\"fr\"").unwrap(), Lang::Id);
        assert!(serde_json::from_str::<Lang>("3").is_err());
    }

    #[test]
    fn dates_in_indonesian() {
        let l = Lang::Id;
        assert_eq!(month_title(l, date(2026, 10, 25)), "Oktober 2026");
        assert_eq!(day_title_long(l, date(2026, 10, 26)), "Senin, 26 Oktober");
        assert_eq!(week_title(l, date(2026, 10, 19)), "19 – 25 Okt 2026");
        assert_eq!(week_title(l, date(2026, 9, 28)), "28 Sep – 4 Okt 2026");
        assert_eq!(
            week_title(l, date(2025, 12, 29)),
            "29 Des 2025 – 4 Jan 2026"
        );
        assert_eq!(day_short(l, date(2026, 10, 19)), "Sen");
        assert_eq!(day_title_long(l, date(2026, 10, 25)), "Minggu, 25 Oktober");
    }

    #[test]
    fn dates_in_english() {
        let l = Lang::En;
        assert_eq!(month_title(l, date(2026, 10, 25)), "October 2026");
        assert_eq!(month_title(l, date(2026, 5, 1)), "May 2026");
        assert_eq!(day_title_long(l, date(2026, 10, 26)), "Monday, 26 October");
        assert_eq!(week_title(l, date(2026, 10, 19)), "19 – 25 Oct 2026");
        assert_eq!(week_title(l, date(2026, 9, 28)), "28 Sep – 4 Oct 2026");
        assert_eq!(
            week_title(l, date(2025, 12, 29)),
            "29 Dec 2025 – 4 Jan 2026"
        );
        assert_eq!(day_short(l, date(2026, 10, 21)), "Wed");
        assert_eq!(day_title_long(l, date(2026, 10, 25)), "Sunday, 25 October");
    }

    #[test]
    fn time_separator_follows_the_language() {
        let t = date(2026, 10, 25).and_time(NaiveTime::from_hms_opt(9, 5, 0).unwrap());
        assert_eq!(time_hm(Lang::Id, t), "09.05");
        assert_eq!(time_hm(Lang::En, t), "09:05");
    }

    #[test]
    fn formatted_strings_in_both_languages() {
        assert_eq!(
            focus_done_body(Lang::Id, 5),
            "Istirahat 5 menit · klik untuk mulai"
        );
        assert_eq!(
            focus_done_body(Lang::En, 15),
            "15-minute break · click to start"
        );
        assert_eq!(
            break_done_body(Lang::Id, 2, 4),
            "Fokus sesi 2 dari 4 · klik untuk mulai"
        );
        assert_eq!(
            break_done_body(Lang::En, 2, 4),
            "Focus session 2 of 4 · click to start"
        );
        assert_eq!(session_of(Lang::Id, 1, 4), "Sesi 1 dari 4");
        assert_eq!(session_of(Lang::En, 1, 4), "Session 1 of 4");
        assert_eq!(pomo_durations(Lang::Id, 25, 5, 15), "25 · 5 · 15 menit");
        assert_eq!(pomo_durations(Lang::En, 50, 10, 20), "50 · 10 · 20 min");
        assert_eq!(phase_minutes(Lang::Id, "Istirahat", 5), "Istirahat 5 menit");
        assert_eq!(phase_minutes(Lang::En, "Focus", 25), "Focus · 25 min");
        assert_eq!(more_count(Lang::Id, 3), "+3 lagi");
        assert_eq!(more_count(Lang::En, 3), "+3 more");
        assert_eq!(battery_body(Lang::Id, 18), "Sisa 18%. Colokin charger.");
        assert_eq!(battery_body(Lang::En, 18), "18% left. Plug in the charger.");
        assert_eq!(
            save_failed_because(Lang::Id, text(Lang::Id, Key::ReasonOffline)),
            "Gagal simpan: tidak ada koneksi"
        );
        assert_eq!(
            save_failed_because(Lang::En, &google_refused(Lang::En, "Not Found")),
            "Couldn't save: Google refused (Not Found)"
        );
        assert_eq!(
            config_copied_to(Lang::En, "config.json.bak"),
            "Copied to config.json.bak — check its contents"
        );
        let page = oauth_page(Lang::En, Key::OauthConnected);
        assert!(page.starts_with("<!doctype html><html lang=\"en\">"));
        assert!(page.contains("Connected. You can go back to the island."));
    }
}
