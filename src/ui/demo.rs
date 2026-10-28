//! `--demo` (debug builds only): cycles through every view, 2 s each, Pill
//! then Notch, with fake calendar, notes, reminders and pomodoro data (some
//! of it long, to check trimming), so the window and renderer can be checked
//! by eye and by screen capture against `docs/design/studio-ui.pdf`. Runs
//! until the process is killed.
//!
//! Environment knobs for screenshots:
//! `--demo-cli` runs only the CLI page's scenes, for comparing with
//! `docs/design/pengingat-cli.html` (p3, p4, p5) and the folder ones with
//! `docs/design/keputusan-v3.1.html` (p2). Their folder menu times count
//! from a fixed "now", so they never depend on the clock.
//!
//! - `DI_DEMO_STEP=n` holds step `n` (see `SCENES`; Notch adds `SCENES.len()`).
//!   A scene's name works too: `DI_DEMO_STEP=cal-day`, or `notch:cal-day`.
//! - `DI_DEMO_T=0.5` freezes the morph into that step at this progress.
//! - `DI_DEMO_SLIDE=0.4` draws the panel mid page-slide.
//! - `DI_LANG=en` draws the UI in English (also for `--demo-editor`).
//! - `DI_ACCENT=orange` and `DI_SURFACE=graphite` pick the island theme.
//! - `DI_CLOCK=0` draws the idle island without its clock.
//! - `DI_LYRIC_AT=ms` puts the lyric scenes at that position in the fake
//!   LRC (`DEMO_LRC`); `DI_DEMO_FADE=0.5` freezes the crossfade from the
//!   previous line at that progress.
//! - `DI_LYRIC_SCROLL=ms` freezes a line too long for the notch that long
//!   after it showed (held for the first `LYRIC_HOLD_MS`, then scrolling);
//!   without it the line scrolls live from the step's start.
//!
//! The lyric scenes size the notch as the island does: the line measured
//! by the renderer, capped by this screen's width (`lyric_max_w`).
//!
//! The cleaner's scenes (`cleaner`, `cleaner-confirm`, `cleaner-done`)
//! use fixed sizes and never start the scanner or the cleaner. Its Mole
//! scenes (`cleaner-mole`, `cleaner-mole-scanning`, `cleaner-mole-confirm`,
//! `cleaner-mole-log`, `cleaner-mole-result`, `cleaner-mole-unreadable`,
//! `cleaner-mole-missing`)
//! and the notices `mole-done` and `mole-failed` use the design's fixed
//! data; they never look for Mole, let alone run it.
//!
//! `--demo-robot` runs only Bata's scenes (`bata-*`): the sheet of faces and
//! sizes (studio-robot.html p2) and the notch states (p3). Their clocks are
//! fixed ("14:32", "23:48"), so nothing depends on the time of day.

use std::time::{Duration, Instant};

use chrono::{Days, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, MA_NOACTIVATE, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
    TranslateMessage, WM_DESTROY, WM_MOUSEACTIVATE, WM_QUIT,
};

use super::geometry::{
    ANIM_MS, ShapeKind, ShapeRect, ease_out_back, lerp_shape, lyric_box_w, lyric_max_w,
    lyric_scroll_offset, lyric_shape, lyric_width, shape_with_robot,
};
use super::render::{
    CleanerView, CliView, ClipView, FolderMenuView, Frame, MoleView, Progress, Renderer,
    RobotFrame, SearchView, ShotView, SystemView,
};
use super::robot::{Expr, Pose};
use super::window::IslandWindow;
use crate::calendar::{CalEvent, CalStatus, CalendarSnapshot, EventTime};
use crate::clean::{self, AdminOutcome, CatSize, CleanCat, CleanMode, CleanReport, ScanResult};
use crate::cli::{self, Output, Shell};
use crate::cli_folders::{FolderList, ListKind};
use crate::cli_panel::{Echoes, RunState};
use crate::clipboard::image::Thumb;
use crate::clipboard::store::ClipImage;
use crate::clipboard::{Clip, ClipData, ClipKind};
use crate::config::Style;
use crate::i18n::{Key, tr};
use crate::lyrics::{self, FADE, LyricsFound, LyricsState, Query};
use crate::model::{CalView, MediaInfo, Notice, NoticeKind, Page, View};
use crate::mole::{CleanRun, GroupSize, MoleOpts, MoleReport, PreviewSummary};
use crate::notes::Note;
use crate::pomodoro::{PomoSettings, Pomodoro};
use crate::reminders::{Reminder, Repeat};
use crate::state::MolePhase;
use crate::system::{Alert, Drive, GIB, ProcUsage, SampleKind, SystemSample, alert_notice};
use crate::theme::{self, Accent, Palette, Surface};
use crate::tiles::{CustomKind, CustomTile, ShortcutTiles};

const PHASE: Duration = Duration::from_millis(2000);
/// The design's track position, 1:42 of 3:58 (about 43 %).
const DEMO_PROGRESS: Progress = Progress {
    position_ms: 102_000,
    end_ms: 238_000,
};

#[derive(Clone, Copy, PartialEq)]
enum Pomo {
    Idle,
    Running,
    Paused,
    Ready,
    /// A short break, running.
    Break,
}

#[derive(Clone, Copy)]
struct Scene {
    name: &'static str,
    kind: ShapeKind,
    view: View0,
    pomo: Pomo,
    cal_view: CalView,
    status: CalStatus,
    day_page: usize,
    notes: usize,
    /// Sample reminders on the reminder page: 5 or 12.
    reminders: usize,
    strip: bool,
    /// Position in `DEMO_LRC` for the lyric view.
    lyric_at: u64,
    /// "Lirik di notch" is on (the Musik page's button).
    lyrics_on: bool,
    /// The volume plate the wheel brings up, at this level.
    volume: Option<f32>,
    /// The Pintasan page with custom tiles among the built-in ones.
    custom_tiles: bool,
    /// The shelf with files being dragged over the island (its drop zone lit).
    shelf_drag: bool,
    /// The quick search is open with the demo's made-up apps.
    search: bool,
    /// What the CLI page's card shows.
    cli: CliDemo,
    /// The track with very long names instead of the design's.
    long_media: bool,
    /// Bata's face; `None` draws the island without the robot.
    robot: Option<Expr>,
    /// A fixed idle clock instead of the time now.
    clock: Option<&'static str>,
    /// Bata's sheet of faces instead of the island.
    sheet: bool,
    /// The System page with drive C: under the 10 GB warning (ochre).
    system_low: bool,
    /// The cleaner over the System page, and how its button stands.
    clean: Option<CleanDemo>,
    /// What the Clipboard page shows.
    clip: ClipDemo,
}

/// The cleaner's states (deepclean-opsi-d.html p10 and d1).
#[derive(Clone, Copy, PartialEq)]
enum CleanDemo {
    /// The top three ticked, the button showing the total.
    Ticks,
    /// The button armed: "Yakin? Hapus 2,3 GB".
    Armed,
    /// Mole mode, its preview done: 1,8 / 0,4 / 0,1 / 0,1 GB, ± 2,4 GB.
    MolePreview,
    /// Mole mode while the dry-run runs.
    MoleScanning,
    /// Mole mode armed: "Yakin? Hapus permanen".
    MoleArmed,
    /// Mole cleaning: the design's log lines.
    MoleLog,
    /// Mole done: the measured result over its log, "Pratinjau lagi".
    MoleResult,
    /// Mole's preview with one group it could not read ("lihat log").
    MoleUnreadable,
    /// Mole not installed: the "Cara memasang" card.
    MoleMissing,
}

/// The Clipboard page's states, after keputusan-v3.1.html §5. Fixed data
/// and a fixed "now": never the real clipboard, never the clock.
#[derive(Clone, Copy, PartialEq)]
enum ClipDemo {
    /// The design: six cards of 48, the first pinned and selected.
    Design,
    /// "denah" typed in the focused search field.
    Search,
    /// Nothing copied yet.
    Empty,
    /// "Riwayat clipboard" off.
    Off,
    /// A long history scrolled down a row: the scrollbar's thumb shows.
    Long,
}

/// The CLI page's states, after pengingat-cli.html p3, p4 and p5.
#[derive(Clone, Copy, PartialEq)]
enum CliDemo {
    /// Nothing run yet, the input not clicked.
    Empty,
    /// p3 "Siap mengetik": two commands done, the input focused.
    Ready,
    /// p4 "Riwayat & saran": "git" typed, three suggestions.
    Typing,
    /// p3 "Perintah berjalan": npm run build at 0:42.
    Running,
    /// A command that ended with exit 0.
    Done,
    /// p4 "Selesai dengan error": npm test, exit 1 after 3.8 s.
    Error,
    /// p5: the enlarged panel with a long git log.
    Tall,
    /// keputusan-v3.1 p2: the page followed Explorer after a git status.
    FromExplorer,
    /// keputusan-v3.1 p2: the folder menu open (2 windows, 1 favourite,
    /// 2 recent).
    FolderMenu,
}

/// `View` without its payloads, so scenes can be `const`.
#[derive(Clone, Copy, PartialEq)]
enum View0 {
    Idle,
    MediaCompact,
    PomodoroCompact,
    Panel(Page),
    Notice(DemoNotice),
    MediaLyric,
}

/// One notice of each badge style.
#[derive(Clone, Copy, PartialEq)]
enum DemoNotice {
    /// A due reminder: accent badge, "Selesai".
    Reminder,
    /// A finished focus phase: badge and "Mulai" in the break's sage.
    Pomodoro,
    Claude,
    Battery,
    /// An app's notification: the letter badge.
    App,
    /// The morning greeting.
    Greeting,
    /// Claude at work (the design's "Claude Code" state).
    ClaudeWorking,
    /// Disk C: low (keputusan-v3.1.html p4): Bata low, "Buka".
    System,
    /// The cleaner finished (deepclean-opsi-d.html p10 "Selesai").
    CleanDone,
    /// Mole finished: 2,4 GB, measured on C: (d2 "Notch selama mode Mole").
    MoleDone,
    /// Mole failed with exit 1.
    MoleFailed,
    /// "Disalin" after copying the first Clipboard card.
    Copied,
    /// The screenshot preview (keputusan-v3.1 p6): 1280 × 720 · 245 KB.
    Shot,
    /// "Disimpan · Pictures\Screenshots" after Simpan.
    ShotSaved,
    /// "Dihapus · clipboard dikosongkan" after Hapus.
    ShotDeleted,
}

/// The screenshot preview's id in the demo (`DemoNotice::Shot`).
const DEMO_SHOT: u64 = 1;
/// Its fixed figures: the design's 1280 × 720, a 245 KB file.
const DEMO_SHOT_SIZE: (u32, u32, u64) = (1280, 720, 250_880);

/// The preview's picture, drawn like the design's `.shot` (a dark title
/// bar, a cream page, an orange heading and striped lines) at 460 × 259,
/// the 1280 × 720 screenshot fitted to the 2× box. Never a real capture.
fn demo_shot_thumb() -> Thumb {
    let (w, h) = (460u32, 259u32);
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    let bar_h = (h as f32 * 0.18) as u32;
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32, y as f32);
            let rgb: [u8; 3] = if y < bar_h {
                [0x2B, 0x22, 0x1C]
            } else if (27.0..=203.0).contains(&fx) && (68.0..=88.0).contains(&fy) {
                [0xC9, 0x54, 0x2B]
            } else if fx >= 27.0
                && fx <= w as f32 - 36.0
                && fy >= 108.0
                && fy <= h as f32 - 26.0
                && ((fy - 108.0) as u32 / 15) % 2 == 0
            {
                [0xE6, 0xDC, 0xCF]
            } else {
                [0xF4, 0xEE, 0xE6]
            };
            bgra.extend([rgb[2], rgb[1], rgb[0], 255]);
        }
    }
    Thumb { w, h, bgra }
}

const BASE: Scene = Scene {
    name: "",
    kind: ShapeKind::Panel,
    view: View0::Idle,
    pomo: Pomo::Idle,
    cal_view: CalView::Month,
    status: CalStatus::Ok,
    day_page: 0,
    notes: 4,
    reminders: 5,
    strip: false,
    lyric_at: 2_000,
    lyrics_on: false,
    volume: None,
    custom_tiles: false,
    shelf_drag: false,
    search: false,
    cli: CliDemo::Empty,
    long_media: false,
    robot: None,
    clock: None,
    sheet: false,
    system_low: false,
    clean: None,
    clip: ClipDemo::Design,
};

const fn clipboard(name: &'static str, which: ClipDemo) -> Scene {
    Scene {
        name,
        view: View0::Panel(Page::Clipboard),
        clip: which,
        ..BASE
    }
}

/// A scene of Bata in the notch (studio-robot.html p3).
const fn bata(name: &'static str, kind: ShapeKind, view: View0, expr: Expr) -> Scene {
    Scene {
        name,
        kind,
        view,
        robot: Some(expr),
        clock: Some("14:32"),
        ..BASE
    }
}

/// Made-up lyrics: a metadata tag, an offset, a line timed twice, an
/// instrumental gap, a line too long for the notch and a short one.
const DEMO_LRC: &str = "[ar:Demo]\n[ti:Jalan Pulang]\n[offset:+0]\n\
[00:01.00][00:24.00] Lampu kota menyala satu per satu\n\
[00:05.20] Kita pulang lewat jalan yang biasa\n\
[00:10.00]\n\
[00:16.00] Langit sore memeluk atap-atap rumah di sepanjang jalan menuju pantai yang jauh di sana\n\
[00:26.00] Tanpa kata\n";

const fn lyric(name: &'static str, lyric_at: u64) -> Scene {
    Scene {
        name,
        kind: ShapeKind::MediaLyric,
        view: View0::MediaLyric,
        lyric_at,
        lyrics_on: true,
        ..BASE
    }
}

const fn panel(name: &'static str, page: Page) -> Scene {
    Scene {
        name,
        view: View0::Panel(page),
        ..BASE
    }
}

const fn notice(name: &'static str, which: DemoNotice) -> Scene {
    Scene {
        name,
        kind: ShapeKind::Notice,
        view: View0::Notice(which),
        ..BASE
    }
}

const fn cli(name: &'static str, which: CliDemo) -> Scene {
    Scene {
        name,
        view: View0::Panel(Page::Cli),
        kind: if matches!(which, CliDemo::Tall) {
            ShapeKind::PanelTall
        } else {
            ShapeKind::Panel
        },
        cli: which,
        ..BASE
    }
}

/// The cleaner over the System page.
const fn cleaner(name: &'static str, which: CleanDemo) -> Scene {
    Scene {
        name,
        kind: ShapeKind::PanelClean,
        view: View0::Panel(Page::System),
        clean: Some(which),
        ..BASE
    }
}

const SCENES: [Scene; 89] = [
    Scene {
        name: "idle",
        kind: ShapeKind::Idle,
        ..BASE
    },
    Scene {
        name: "pomo-running",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Running,
        ..BASE
    },
    Scene {
        name: "pomo-paused",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Paused,
        ..BASE
    },
    Scene {
        name: "pomo-ready",
        kind: ShapeKind::PomodoroCompact,
        view: View0::PomodoroCompact,
        pomo: Pomo::Ready,
        ..BASE
    },
    Scene {
        name: "media-compact",
        kind: ShapeKind::MediaCompact,
        view: View0::MediaCompact,
        ..BASE
    },
    // The wheel over the compact notch: the volume plate at 64 %.
    Scene {
        name: "media-volume",
        kind: ShapeKind::MediaCompact,
        view: View0::MediaCompact,
        volume: Some(0.64),
        ..BASE
    },
    Scene {
        volume: Some(0.64),
        ..panel("music-volume", Page::Media)
    },
    panel("cal-month", Page::Calendar),
    Scene {
        cal_view: CalView::Week,
        ..panel("cal-week", Page::Calendar)
    },
    Scene {
        cal_view: CalView::Day,
        ..panel("cal-day", Page::Calendar)
    },
    Scene {
        status: CalStatus::NeedsLogin,
        ..panel("cal-needslogin", Page::Calendar)
    },
    Scene {
        cal_view: CalView::Day,
        status: CalStatus::Offline,
        day_page: 1,
        ..panel("cal-day-offline-p1", Page::Calendar)
    },
    Scene {
        pomo: Pomo::Running,
        ..panel("pomodoro", Page::Pomodoro)
    },
    Scene {
        pomo: Pomo::Break,
        ..panel("pomodoro-break", Page::Pomodoro)
    },
    panel("reminders-5", Page::Reminders),
    Scene {
        reminders: 12,
        ..panel("reminders-12", Page::Reminders)
    },
    notice("reminder-notice", DemoNotice::Reminder),
    // Focus done: the short break waits, so badge and chip are sage.
    Scene {
        pomo: Pomo::Ready,
        ..notice("notice-pomodoro", DemoNotice::Pomodoro)
    },
    notice("notice-claude", DemoNotice::Claude),
    notice("notice-battery", DemoNotice::Battery),
    notice("notice-system", DemoNotice::System),
    notice("notice-app", DemoNotice::App),
    panel("notes-4", Page::Notes),
    Scene {
        notes: 6,
        ..panel("notes-6", Page::Notes)
    },
    Scene {
        notes: 30,
        ..panel("notes-30", Page::Notes)
    },
    panel("media", Page::Media),
    Scene {
        long_media: true,
        ..panel("media-long", Page::Media)
    },
    Scene {
        strip: true,
        ..panel("strip", Page::Calendar)
    },
    // A medium line hugged by the notch, one capped and scrolling, the
    // instrumental rest and a short line (both the minimum).
    lyric("lyric", 6_000),
    lyric("lyric-long", 17_000),
    lyric("lyric-instrumental", 11_000),
    lyric("lyric-short", 27_000),
    Scene {
        lyrics_on: true,
        ..panel("media-lyrics-on", Page::Media)
    },
    // Fixed figures (keputusan-v3.1.html p4); never the real machine.
    Scene {
        custom_tiles: true,
        ..panel("shortcuts-custom", Page::Shortcuts)
    },
    panel("mixer", Page::Mixer),
    panel("shelf", Page::Shelf),
    panel("shelf-empty", Page::Shelf),
    Scene {
        shelf_drag: true,
        ..panel("shelf-drop", Page::Shelf)
    },
    Scene {
        shelf_drag: true,
        ..panel("shelf-drop-files", Page::Shelf)
    },
    Scene {
        search: true,
        ..panel("search", Page::Search)
    },
    panel("system", Page::System),
    Scene {
        system_low: true,
        ..panel("system-low", Page::System)
    },
    // Fixed sizes (deepclean-opsi-d.html); never a real scan.
    cleaner("cleaner", CleanDemo::Ticks),
    cleaner("cleaner-confirm", CleanDemo::Armed),
    Scene {
        robot: Some(Expr::Happy),
        ..notice("cleaner-done", DemoNotice::CleanDone)
    },
    // Mole mode with the design's data; Mole is never looked for or run.
    cleaner("cleaner-mole", CleanDemo::MolePreview),
    cleaner("cleaner-mole-scanning", CleanDemo::MoleScanning),
    cleaner("cleaner-mole-confirm", CleanDemo::MoleArmed),
    cleaner("cleaner-mole-log", CleanDemo::MoleLog),
    cleaner("cleaner-mole-result", CleanDemo::MoleResult),
    cleaner("cleaner-mole-unreadable", CleanDemo::MoleUnreadable),
    cleaner("cleaner-mole-missing", CleanDemo::MoleMissing),
    Scene {
        robot: Some(Expr::Happy),
        ..notice("mole-done", DemoNotice::MoleDone)
    },
    Scene {
        robot: Some(Expr::Sad),
        ..notice("mole-failed", DemoNotice::MoleFailed)
    },
    cli("cli-empty", CliDemo::Empty),
    cli("cli-ready", CliDemo::Ready),
    cli("cli-typing", CliDemo::Typing),
    cli("cli-running", CliDemo::Running),
    cli("cli-done", CliDemo::Done),
    cli("cli-error", CliDemo::Error),
    cli("cli-tall", CliDemo::Tall),
    cli("cli-from-explorer", CliDemo::FromExplorer),
    cli("cli-folder-menu", CliDemo::FolderMenu),
    panel("shortcuts", Page::Shortcuts),
    clipboard("clipboard", ClipDemo::Design),
    clipboard("clipboard-search", ClipDemo::Search),
    clipboard("clipboard-empty", ClipDemo::Empty),
    clipboard("clipboard-off", ClipDemo::Off),
    clipboard("clipboard-long", ClipDemo::Long),
    // keputusan-v3.1 §5 "Saat menyalin": Bata happy over the preview.
    bata(
        "clipboard-copied",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::Copied),
        Expr::Happy,
    ),
    // keputusan-v3.1 p6: the preview with Bata happy, then "Disimpan"
    // (happy) and "Dihapus" (at rest). Fixed figures and a drawn picture.
    bata(
        "screenshot",
        ShapeKind::NoticeShot,
        View0::Notice(DemoNotice::Shot),
        Expr::Happy,
    ),
    bata(
        "screenshot-saved",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::ShotSaved),
        Expr::Happy,
    ),
    bata(
        "screenshot-deleted",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::ShotDeleted),
        Expr::Idle,
    ),
    Scene {
        name: "bata-sheet",
        sheet: true,
        ..BASE
    },
    bata("bata-kosong", ShapeKind::Idle, View0::Idle, Expr::Idle),
    Scene {
        clock: Some("23:48"),
        ..bata("bata-tidur", ShapeKind::Idle, View0::Idle, Expr::Sleep)
    },
    bata(
        "bata-sapaan",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::Greeting),
        Expr::Wave,
    ),
    Scene {
        pomo: Pomo::Running,
        ..bata(
            "bata-pomodoro",
            ShapeKind::PomodoroCompact,
            View0::PomodoroCompact,
            Expr::Focus,
        )
    },
    bata(
        "bata-musik",
        ShapeKind::MediaCompact,
        View0::MediaCompact,
        Expr::Music,
    ),
    Scene {
        lyric_at: 6_000,
        lyrics_on: true,
        ..bata(
            "bata-lirik",
            ShapeKind::MediaLyric,
            View0::MediaLyric,
            Expr::Sing,
        )
    },
    Scene {
        lyric_at: 17_000,
        lyrics_on: true,
        ..bata(
            "bata-lirik-long",
            ShapeKind::MediaLyric,
            View0::MediaLyric,
            Expr::Sing,
        )
    },
    bata(
        "bata-pengingat",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::Reminder),
        Expr::Alert,
    ),
    Scene {
        pomo: Pomo::Ready,
        ..bata(
            "bata-fase",
            ShapeKind::Notice,
            View0::Notice(DemoNotice::Pomodoro),
            Expr::Happy,
        )
    },
    bata(
        "bata-claude",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::ClaudeWorking),
        Expr::Think,
    ),
    // Other apps keep their badge: no robot.
    Scene {
        robot: None,
        ..bata(
            "bata-notifikasi",
            ShapeKind::Notice,
            View0::Notice(DemoNotice::App),
            Expr::Idle,
        )
    },
    bata(
        "bata-cli-running",
        ShapeKind::Idle,
        View0::Idle,
        Expr::Think,
    ),
    bata("bata-cli-done", ShapeKind::Idle, View0::Idle, Expr::Happy),
    bata("bata-cli-failed", ShapeKind::Idle, View0::Idle, Expr::Sad),
    bata(
        "bata-sistem",
        ShapeKind::Notice,
        View0::Notice(DemoNotice::System),
        Expr::Sad,
    ),
];

/// The design's CPU line: 60 points (60 s, one per second), ending on the ring's 33 %.
const DEMO_CPU: [f32; 60] = [
    22.0, 24.0, 26.0, 28.0, 30.0, 28.0, 27.0, 26.0, 25.0, 29.0, 33.0, 37.0, 41.0, 40.0, 39.0, 38.0,
    38.0, 42.0, 45.0, 48.0, 52.0, 48.0, 44.0, 40.0, 35.0, 32.0, 30.0, 29.0, 28.0, 30.0, 31.0, 32.0,
    33.0, 36.0, 40.0, 44.0, 47.0, 44.0, 42.0, 36.0, 31.0, 30.0, 29.0, 31.0, 33.0, 34.0, 36.0, 34.0,
    31.0, 30.0, 28.0, 29.0, 30.0, 32.0, 35.0, 33.0, 31.0, 32.0, 33.0, 33.0,
];

/// The System page's fixed figures (keputusan-v3.1.html `sysPanel`): CPU
/// 33 % on 8 cores, RAM 14,2 of 31,8 GB (44 %), C: and D:, three apps,
/// up 6 h 1 m. `low`: C: has 9,6 GB left, under the 10 GB warning.
fn demo_system(low: bool) -> SystemSample {
    let gb = |x: f64| (x * GIB as f64) as u64;
    let app = |name: &str, cpu: f32, mem: f64| ProcUsage {
        name: name.into(),
        cpu,
        mem: gb(mem),
    };
    SystemSample {
        kind: SampleKind::Page,
        cpu: 33.0,
        cores: 8,
        // Shown as 14,2 GB, and 44 % rather than the 45 % 14.2 would round to.
        ram_used: gb(14.1505),
        ram_total: gb(31.8),
        drives: vec![
            Drive {
                letter: 'C',
                free: gb(if low { 9.6 } else { 47.2 }),
                total: gb(338.4),
            },
            Drive {
                letter: 'D',
                free: gb(33.0),
                total: gb(137.0),
            },
        ],
        procs: vec![
            app("Chrome", 12.0, 3.1),
            app("Code", 7.0, 1.4),
            app("AutoCAD", 5.0, 2.2),
        ],
        uptime: Duration::from_secs(6 * 3600 + 60),
    }
}

/// The cleaner's sizes in the design: 0,1 / 1,8 / 0,4 / 0,1 GB, the rest
/// not measured ("—").
fn demo_scan() -> ScanResult {
    let gb = |x: f64| CatSize {
        bytes: (x * GIB as f64) as u64,
        files: 1,
        measured: true,
    };
    let unknown = CatSize::default();
    ScanResult {
        cats: [
            gb(0.1),
            gb(1.8),
            gb(0.4),
            gb(0.1),
            unknown,
            unknown,
            unknown,
        ],
        skipped_browsers: Vec::new(),
    }
}

/// Mole's preview in the design (`MG`): 1,8 / 0,4 / 0,1 / 0,1 GB, ± 2,4 GB
/// in 29 entries; `unreadable` leaves the app caches as "lihat log".
fn demo_mole_summary(unreadable: bool) -> PreviewSummary {
    let gb = |x: f64| (x * GIB as f64) as u64;
    let apps = if unreadable {
        GroupSize::Unreadable
    } else {
        GroupSize::Bytes(gb(0.4))
    };
    PreviewSummary {
        groups: [
            GroupSize::Bytes(gb(1.8)),
            apps,
            GroupSize::Bytes(gb(0.1)),
            GroupSize::Bytes(gb(0.1)),
        ],
        total: Some((gb(if unreadable { 2.0 } else { 2.4 }), 29)),
    }
}

/// Mole's log while it cleans (`logLines` in deepclean-opsi-d.html).
fn demo_mole_log() -> std::collections::VecDeque<String> {
    [
        "Clean Your Windows",
        "Free space: 47,24 GB",
        "\u{25B8} User essentials",
        "  User temp files (34 items, 61 MB)",
        "\u{25B8} Browser caches",
        "  Chrome CacheStorage (\u{2026} 922 MB)",
        "  Chrome Code Cache (\u{2026} 507 MB)",
        "\u{25B8} Developer tools \u{2026}",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Mole finished: 2,4 GB freed, C: from 47,2 to 49,6 GB; `failed`: exit 1.
fn demo_mole_report(failed: bool) -> MoleReport {
    let gb = |x: f64| (x * GIB as f64) as u64;
    MoleReport {
        run: CleanRun {
            exit: Some(if failed { 1 } else { 0 }),
            admin: AdminOutcome::NotAsked,
        },
        freed: gb(if failed { 0.4 } else { 2.4 }),
        drive: Some(('C', gb(47.2), gb(49.6))),
    }
}

/// The finished clean: 2,3 GB freed, C: now at 49,5 GB, 3 files in use.
fn demo_clean_report() -> CleanReport {
    let gb = |x: f64| (x * GIB as f64) as u64;
    CleanReport {
        freed: gb(2.3),
        drive: Some(('C', gb(49.5))),
        skipped: 3,
        skipped_browsers: Vec::new(),
        admin: AdminOutcome::NotAsked,
    }
}

/// `DI_DEMO_STEP`: a step number, or a scene's name (Pill), or
/// `notch:` and a name.
fn held_step(scenes: &[Scene], value: &str) -> Option<usize> {
    if let Ok(n) = value.trim().parse() {
        return Some(n);
    }
    let (offset, name) = match value.trim().strip_prefix("notch:") {
        Some(name) => (scenes.len(), name),
        None => (0, value.trim()),
    };
    scenes
        .iter()
        .position(|s| s.name == name)
        .map(|i| i + offset)
}

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}

pub fn run() {
    run_scenes(&SCENES);
}

/// `--demo-cli`: only the CLI page's scenes (`cli-empty`, `cli-ready`,
/// `cli-typing`, `cli-running`, `cli-done`, `cli-error`, `cli-tall`,
/// `cli-from-explorer`, `cli-folder-menu`), Pill
/// then Notch; `DI_DEMO_STEP` picks one by number or name as in `--demo`.
/// `--demo-robot`: only Bata's scenes, Pill then Notch; `DI_DEMO_STEP`
/// picks one by number or name as in `--demo`.
pub fn run_robot() {
    let scenes: Vec<Scene> = SCENES
        .iter()
        .copied()
        .filter(|s| s.name.starts_with("bata-"))
        .collect();
    run_scenes(&scenes);
}

pub fn run_cli() {
    let scenes: Vec<Scene> = SCENES
        .iter()
        .copied()
        .filter(|s| s.name.starts_with("cli-"))
        .collect();
    run_scenes(&scenes);
}

fn run_scenes(scenes: &[Scene]) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut win = IslandWindow::create(Some(wnd_proc)).expect("create island window");
    // Room for the enlarged CLI panel in every scene.
    win.set_tall(true);
    let mut renderer = Renderer::new().expect("create renderer");
    let media = MediaInfo {
        title: "Senja di Kemang".into(),
        artist: "Studio Ruang".into(),
        playing: true,
        art: Some(test_art()),
        source_app: "Spotify.exe".into(),
    };
    let long_media = MediaInfo {
        title: "Sampai Jadi Debu (Live at Synchronize Festival 2024, Extended Version)".into(),
        artist: "Banda Neira feat. Hindia, Kunto Aji, Nadin Amizah & Friends".into(),
        ..media.clone()
    };
    let shot_thumb = demo_shot_thumb();
    let today = Local::now().date_naive();
    let name = |var| std::env::var(var).unwrap_or_default();
    let palette = theme::island(
        Accent::from_name(&name("DI_ACCENT")).unwrap_or_default(),
        Surface::from_name(&name("DI_SURFACE")).unwrap_or_default(),
    );
    let calendar = demo_calendar(today, &palette);
    let notes = demo_notes(today);
    let notes30 = many_notes(today);
    let reminders5 = demo_reminders(today);
    let reminders12 = many_reminders(today);
    let (system_ok, system_low) = (demo_system(false), demo_system(true));
    let scan = demo_scan();
    let (mole_summary, mole_unreadable) = (demo_mole_summary(false), demo_mole_summary(true));
    let mole_log = demo_mole_log();
    let mole_report = demo_mole_report(false);
    let no_log = std::collections::VecDeque::new();
    let clips = demo_clips();
    let strip = Notice {
        key: "demo-wait".into(),
        source: "Claude".into(),
        title: tr(Key::ClaudeWaiting).into(),
        body: "Vale".into(),
        kind: NoticeKind::ClaudeWaiting {
            session_id: "demo".into(),
        },
    };
    let hold: Option<usize> = std::env::var("DI_DEMO_STEP")
        .ok()
        .and_then(|v| held_step(scenes, &v));
    let frozen_t: Option<f32> = env("DI_DEMO_T");
    let slide: f32 = env("DI_DEMO_SLIDE").unwrap_or(0.0);
    let clock_on = std::env::var("DI_CLOCK").map_or(true, |v| v != "0");
    let lyric_at: Option<u64> = env("DI_LYRIC_AT");
    let lyric_fade: Option<f32> = env("DI_DEMO_FADE");
    let lyric_scroll: Option<u64> = env("DI_LYRIC_SCROLL");
    let steps = 2 * scenes.len();
    println!("demo: 2 s per view, Pill then Notch ({steps} steps); kill the process to stop");
    let start = Instant::now();
    let mut settled = usize::MAX;
    loop {
        if !pump_messages() {
            return;
        }
        let elapsed = start.elapsed();
        let (step, into) = match hold {
            Some(n) => (n % steps, elapsed.as_millis()),
            None => (
                (elapsed.as_millis() / PHASE.as_millis()) as usize % steps,
                elapsed.as_millis() % PHASE.as_millis(),
            ),
        };
        let t = frozen_t.unwrap_or((into as f32 / ANIM_MS as f32).min(1.0));
        let (style, scene) = scene(scenes, step);
        if scene.sheet {
            if step != settled {
                println!("demo: step {step} {:?} {}", style, scene.name);
                settled = step;
                if let Err(error) = renderer.draw_robot_sheet(&win, palette) {
                    eprintln!("demo: draw failed: {error}");
                }
            }
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        let now = Instant::now();
        let at = lyric_at.unwrap_or(scene.lyric_at);
        let lyric_state = demo_lyrics(at, lyric_fade, now);
        let to = scene_shape(&renderer, &win, style, &scene, at);
        // How far a long line has scrolled: frozen, or live from the step's start.
        let scroll = match scene.view {
            View0::MediaLyric => {
                let text = lyric_state.shown(now).map_or("", |l| l.text);
                let box_w = lyric_box_w(to.w, scene.robot.is_some());
                renderer.lyric_text_width(text).map_or(0.0, |text_w| {
                    lyric_scroll_offset(text_w, box_w, lyric_scroll.unwrap_or(into as u64))
                })
            }
            _ => 0.0,
        };
        // The equalizer and a scrolling line animate; everything else redraws
        // only while morphing.
        let animated = scene.view == View0::MediaCompact
            || (scene.view == View0::MediaLyric && lyric_scroll.is_none() && scroll > 0.0);
        if step != settled || animated {
            if t >= 1.0 {
                if settled != step {
                    println!("demo: step {step} {:?} {}", style, scene.name);
                }
                settled = step;
            }
            let (prev_style, prev) = self::scene(scenes, (step + steps - 1) % steps);
            let prev_at = lyric_at.unwrap_or(prev.lyric_at);
            let from = scene_shape(&renderer, &win, prev_style, &prev, prev_at);
            let shape = lerp_shape(from, to, ease_out_back(t));
            let pomodoro = demo_pomodoro(scene.pomo, now);
            let view = match scene.view {
                View0::Idle => View::Idle,
                View0::MediaCompact => View::MediaCompact,
                View0::PomodoroCompact => View::PomodoroCompact,
                View0::Panel(page) => View::Panel(page),
                View0::Notice(which) => View::Notice(demo_notice(which, &renderer)),
                View0::MediaLyric => View::MediaLyric,
            };
            let page = match scene.view {
                View0::Panel(page) => page,
                _ => Page::Calendar,
            };
            let snapshot = CalendarSnapshot {
                status: scene.status,
                ..calendar.clone()
            };
            let cli = demo_cli(scene.cli, now);
            let notes: &[Note] = match scene.notes {
                30 => &notes30,
                n => &notes[..n.min(notes.len())],
            };
            let system = SystemView {
                sample: Some(if scene.system_low {
                    &system_low
                } else {
                    &system_ok
                }),
                eased: None,
                history: &DEMO_CPU,
                disk_warn_gb: 10,
                ram_warn_percent: 90,
            };
            let cleaner = scene.clean.map(|which| {
                let opts = MoleOpts::default();
                let previewed = MolePhase::Previewed { opts, ok: true };
                let phase = match which {
                    CleanDemo::Ticks | CleanDemo::Armed => MolePhase::Unknown,
                    CleanDemo::MolePreview | CleanDemo::MoleArmed | CleanDemo::MoleUnreadable => {
                        previewed
                    }
                    CleanDemo::MoleScanning => MolePhase::Scanning(opts),
                    CleanDemo::MoleLog => MolePhase::Running,
                    CleanDemo::MoleResult => MolePhase::Finished { ok: true },
                    CleanDemo::MoleMissing => MolePhase::Missing,
                };
                let with_log = matches!(which, CleanDemo::MoleLog | CleanDemo::MoleResult);
                let mole_mode = phase != MolePhase::Unknown;
                CleanerView {
                    mode: if mole_mode {
                        CleanMode::Mole
                    } else {
                        CleanMode::Own
                    },
                    checked: CleanCat::ALL.map(CleanCat::checked_at_start),
                    armed: matches!(which, CleanDemo::Armed | CleanDemo::MoleArmed),
                    scan: Some(&scan),
                    scanning: false,
                    busy: false,
                    mole: MoleView {
                        phase,
                        opts,
                        show_log: false,
                        version: Some("1.30.0"),
                        tested: true,
                        summary: match which {
                            CleanDemo::MoleUnreadable => Some(&mole_unreadable),
                            _ => Some(&mole_summary),
                        },
                        log: if with_log { &mole_log } else { &no_log },
                        report: (which == CleanDemo::MoleResult).then_some(&mole_report),
                    },
                }
            });
            let mixer_rows = demo_mixer();
            let shelf_cards = demo_shelf(matches!(scene.name, "shelf" | "shelf-drop-files"));
            let search_results = if scene.search {
                demo_search_results()
            } else {
                Vec::new()
            };
            let frame = Frame {
                view: &view,
                media: Some(if scene.long_media {
                    &long_media
                } else {
                    &media
                }),
                pomodoro: &pomodoro,
                now,
                today,
                page,
                strip: scene.strip.then_some(&strip),
                cal_view: scene.cal_view,
                cal_anchor: today,
                cal_selected: today,
                calendar: &snapshot,
                mixer: Some(&mixer_rows),
                shelf: &shelf_cards,
                shelf_drag: scene.shelf_drag,
                notes,
                notes_page: 0,
                reminders: if scene.reminders == 12 {
                    &reminders12
                } else {
                    &reminders5
                },
                reminders_page: 0,
                day_page: scene.day_page,
                page_slide: slide,
                // The demo slides between neighbours in the default order.
                page_from: {
                    let i = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
                    let from = if slide > 0.0 {
                        i.checked_sub(1)
                    } else if slide < 0.0 {
                        Some(i + 1).filter(|j| *j < Page::ALL.len())
                    } else {
                        None
                    };
                    from.map(|j| Page::ALL[j])
                },
                pages: Page::ALL.to_vec(),
                tiles: scene_tiles(&scene),
                palette,
                clock: match scene.clock {
                    Some(fixed) => Some(fixed.to_owned()),
                    None => clock_on.then(|| Local::now().format("%H:%M").to_string()),
                },
                lyric: lyric_state.shown(now),
                lyric_scroll: scroll,
                lyric_prev_scroll: 0.0,
                lyrics_on: scene.lyrics_on,
                lyric_offset_ms: if scene.lyrics_on { -300 } else { 0 },
                volume: scene.volume,
                cli: cli.view(),
                clip: clip_view(&clips, scene.clip),
                search: SearchView {
                    query: DEMO_SEARCH_QUERY,
                    selection: (DEMO_SEARCH_QUERY.len(), DEMO_SEARCH_QUERY.len()),
                    focused: scene.search,
                    results: &search_results,
                    selected: 0,
                },
                progress: Some(DEMO_PROGRESS),
                robot: scene.robot.map(|expr| RobotFrame {
                    expr,
                    pose: Pose::default(),
                }),
                system: Some(&system),
                cleaner: cleaner.as_ref(),
                shot: (scene.view == View0::Notice(DemoNotice::Shot)).then(|| {
                    let (w, h, bytes) = DEMO_SHOT_SIZE;
                    ShotView {
                        id: DEMO_SHOT,
                        thumb: &shot_thumb,
                        w,
                        h,
                        bytes,
                    }
                }),
            };
            if let Err(error) = renderer.draw(&win, style, shape, &frame, 1.0) {
                eprintln!("demo: draw failed: {error}");
            }
        }
        std::thread::sleep(Duration::from_millis(if animated { 33 } else { 16 }));
    }
}

/// The design's `CLIPS2`: kind, pinned, value. The image card's value is
/// unused (its meta comes from the size).
const DEMO_CLIPS: [(ClipKind, bool, &str); 6] = [
    (
        ClipKind::Text,
        true,
        "Denah lantai 2 sudah saya revisi, tolong dicek bagian tangga",
    ),
    (ClipKind::Link, false, "github.com/tw93/mole/tree/windows"),
    (ClipKind::Image, false, ""),
    (ClipKind::Code, false, "cargo test --locked -- --nocapture"),
    (ClipKind::Color, false, "#E2683C"),
    (
        ClipKind::Text,
        false,
        "Jl. Kemang Raya No. 12, Jakarta Selatan",
    ),
];
/// "Cari di 48 item…": the six cards are the newest of 48.
const DEMO_CLIP_TOTAL: usize = 48;

/// The fixed "now" the cards' ages count from (never the clock).
fn demo_clip_now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 27)
        .and_then(|d| d.and_hms_opt(14, 32, 0))
        .expect("valid time")
}

/// The six cards, copied a few minutes apart before `demo_clip_now`; the
/// screenshot 2 minutes ago, its thumbnail the design's terracotta
/// gradient with a pale frame.
fn demo_clips() -> Vec<Clip> {
    let now = demo_clip_now();
    let unix = |mins: i64| {
        Local
            .from_local_datetime(&(now - chrono::Duration::minutes(mins)))
            .earliest()
            .map_or(0, |t| t.timestamp())
    };
    DEMO_CLIPS
        .iter()
        .enumerate()
        .map(|(i, &(kind, pinned, value))| {
            let data = match kind {
                ClipKind::Image => ClipData::Image(ClipImage {
                    width: 1280,
                    height: 720,
                    thumb: demo_thumb(),
                    fingerprint: 1,
                    screenshot: true,
                    full: None,
                }),
                _ => ClipData::Text(value.into()),
            };
            Clip {
                id: i as u64 + 1,
                kind,
                data,
                pinned,
                at: unix(i as i64 * 2),
            }
        })
        .collect()
}

/// `.clip.img .th`: a 160° gradient #E2683C → #5A2A1C, a pale inner frame.
fn demo_thumb() -> Thumb {
    let (w, h) = (184u32, 104u32);
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    let (a, b) = ([0xE2, 0x68, 0x3C], [0x5A, 0x2A, 0x1C]);
    let inset = 28.0;
    for y in 0..h {
        for x in 0..w {
            let t = ((x as f32 * 0.34 + y as f32 * 0.94) / (w as f32 * 0.34 + h as f32 * 0.94))
                .clamp(0.0, 1.0);
            let mut rgb = [0f32; 3];
            for c in 0..3 {
                rgb[c] = a[c] as f32 + (b[c] as f32 - a[c] as f32) * t;
            }
            let (fx, fy) = (x as f32, y as f32);
            let on_frame = |v: f32, lo: f32, hi: f32| (v - lo).abs() < 1.5 || (v - hi).abs() < 1.5;
            let inside_x = fx >= inset - 1.5 && fx <= w as f32 - inset + 1.5;
            let inside_y = fy >= inset - 1.5 && fy <= h as f32 - inset + 1.5;
            let frame = (on_frame(fx, inset, w as f32 - inset) && inside_y)
                || (on_frame(fy, inset, h as f32 - inset) && inside_x);
            if frame {
                for (c, pale) in rgb.iter_mut().zip([0xF4, 0xED, 0xE4]) {
                    *c += (pale as f32 - *c) * 0.55;
                }
            }
            bgra.extend([rgb[2] as u8, rgb[1] as u8, rgb[0] as u8, 255]);
        }
    }
    Thumb { w, h, bgra }
}

/// The Clipboard page for `which`, from `clips` (`demo_clips`).
fn clip_view(clips: &[Clip], which: ClipDemo) -> ClipView<'_> {
    let now = demo_clip_now();
    match which {
        ClipDemo::Design => ClipView {
            items: clips.iter().collect(),
            total: DEMO_CLIP_TOTAL,
            selected: clips.first().map(|c| c.id),
            ..ClipView::empty(now)
        },
        ClipDemo::Search => {
            let query = "denah";
            let end = query.encode_utf16().count();
            let items: Vec<&Clip> = clips
                .iter()
                .filter(|c| {
                    c.plain_text()
                        .is_some_and(|t| t.to_lowercase().contains(query))
                })
                .collect();
            ClipView {
                selected: items.first().map(|c| c.id),
                items,
                total: DEMO_CLIP_TOTAL,
                query,
                selection: (end, end),
                focused: true,
                ..ClipView::empty(now)
            }
        }
        // Fourteen cards (the six again, then some): the panel shows three
        // rows of two, so it scrolls; one row down.
        ClipDemo::Long => ClipView {
            items: clips.iter().cycle().take(14).collect(),
            total: DEMO_CLIP_TOTAL,
            selected: clips.get(1).map(|c| c.id),
            scroll: 1,
            ..ClipView::empty(now)
        },
        ClipDemo::Empty => ClipView::empty(now),
        ClipDemo::Off => ClipView {
            enabled: false,
            ..ClipView::empty(now)
        },
    }
}

/// One notice per badge style, with the design's texts; the cleaner's are
/// fitted to the notice by `renderer` as the island does.
fn demo_notice(which: DemoNotice, renderer: &Renderer) -> Notice {
    let fits = |kind: NoticeKind| move |s: &str| renderer.notice_text_fits(&kind, false, s);
    let fits_either =
        |kind: NoticeKind| move |title: bool, s: &str| renderer.notice_text_fits(&kind, title, s);
    let notice = |source: &str, title: &str, body: String, kind| Notice {
        key: "demo".into(),
        source: source.into(),
        title: title.into(),
        body,
        kind,
    };
    match which {
        DemoNotice::Reminder => notice(
            tr(Key::SourceReminder),
            "Kirim revisi denah ke klien",
            format!(
                "{} · {}",
                tr(Key::Missed),
                crate::i18n::time_label(
                    crate::i18n::lang(),
                    NaiveTime::from_hms_opt(16, 30, 0).expect("valid time"),
                ),
            ),
            NoticeKind::Reminder { id: "demo".into() },
        ),
        DemoNotice::Pomodoro => notice(
            tr(Key::Pomodoro),
            tr(Key::FocusDone),
            crate::i18n::focus_done_body(crate::i18n::lang(), 5),
            NoticeKind::Pomodoro,
        ),
        DemoNotice::Claude => notice(
            "Claude",
            tr(Key::ClaudeWaiting),
            format!("vale · {}", tr(Key::DemoClaudeAsks)),
            NoticeKind::ClaudeWaiting {
                session_id: "demo".into(),
            },
        ),
        DemoNotice::Battery => notice(
            tr(Key::SourceBattery),
            tr(Key::BatteryLow),
            crate::i18n::battery_body(crate::i18n::lang(), 20),
            NoticeKind::BatteryLow,
        ),
        DemoNotice::App => notice(
            "Mail",
            "Rani · Mail",
            "Denah lantai 2 sudah saya kirim, cek ya".into(),
            NoticeKind::App { app_id: None },
        ),
        DemoNotice::Greeting => notice(
            tr(Key::SourceVale),
            tr(Key::GoodMorning),
            crate::i18n::greeting_body(crate::i18n::lang(), 3, 2),
            NoticeKind::Greeting,
        ),
        DemoNotice::Copied => notice(
            tr(Key::ClipboardTitle),
            tr(Key::ClipCopied),
            crate::clipboard::store::preview(DEMO_CLIPS[0].2, 120),
            NoticeKind::Copied,
        ),
        DemoNotice::Shot => notice(
            tr(Key::ShotTitle),
            tr(Key::ShotTitle),
            String::new(),
            NoticeKind::Screenshot {
                shot: DEMO_SHOT,
                secs: 8,
                delete_if_ignored: false,
            },
        ),
        DemoNotice::ShotSaved => notice(
            tr(Key::ShotTitle),
            tr(Key::ShotSaved),
            r"Pictures\Screenshots".into(),
            NoticeKind::ShotSaved,
        ),
        DemoNotice::ShotDeleted => {
            let lang = crate::i18n::lang();
            let (title, body) =
                crate::i18n::shot_deleted(lang, crate::screenshot::FileOutcome::Recycled, true);
            notice(tr(Key::ShotTitle), title, body, NoticeKind::ShotDeleted)
        }
        DemoNotice::ClaudeWorking => notice(
            "Claude",
            tr(Key::DemoClaudeWorking),
            format!("vale · {}", tr(Key::DemoFilesChanged)),
            NoticeKind::ClaudeDone {
                session_id: "demo".into(),
            },
        ),
        DemoNotice::System => alert_notice(
            crate::i18n::lang(),
            &Alert::Disk {
                letter: 'C',
                free: (9.6 * GIB as f64) as u64,
            },
        ),
        DemoNotice::CleanDone => clean::done_notice(
            crate::i18n::lang(),
            &demo_clean_report(),
            &fits(NoticeKind::CleanDone),
        ),
        DemoNotice::MoleDone => crate::mole::done_notice(
            crate::i18n::lang(),
            &demo_mole_report(false),
            &fits_either(NoticeKind::MoleDone),
        ),
        DemoNotice::MoleFailed => crate::mole::done_notice(
            crate::i18n::lang(),
            &demo_mole_report(true),
            &fits_either(NoticeKind::MoleFailed),
        ),
    }
}

/// `DEMO_LRC` through the island's own lyric state, on the line at `at` ms;
/// with `fade`, crossfading into it from the line before at that progress.
fn demo_lyrics(at: u64, fade: Option<f32>, now: Instant) -> LyricsState {
    let lines = lyrics::parse_lrc(DEMO_LRC);
    let query = Query {
        key: "demo".into(),
        artist: "Demo".into(),
        title: "Jalan Pulang".into(),
        duration_ms: 30_000,
        provisional: false,
    };
    let mut state = LyricsState::default();
    state.want(Some(&query), true);
    state.found(LyricsFound {
        key: "demo".into(),
        lines: Some(std::sync::Arc::new(lines.clone())),
        transient: false,
    });
    if let Some(f) = fade {
        let line = lyrics::current_line(&lines, at).map_or(0, |(i, _)| i);
        let before = line.checked_sub(1).map_or(0, |i| lines[i].at_ms);
        state.advance(before, now);
        let since = now
            .checked_sub(FADE.mul_f32(f.clamp(0.0, 1.0)))
            .unwrap_or(now);
        state.advance(at, since);
    } else {
        state.advance(at, now);
    }
    state
}

/// What one CLI scene shows, owned for the frame.
struct DemoCli {
    output: Output,
    echoes: Echoes,
    run: RunState,
    input: &'static str,
    focused: bool,
    suggestions: Vec<&'static str>,
    tall: bool,
    cwd: &'static str,
    from_explorer: bool,
    menu: Option<FolderMenuView>,
}

impl DemoCli {
    fn view(&self) -> CliView<'_> {
        let end = self.input.encode_utf16().count();
        CliView {
            output: &self.output,
            echoes: &self.echoes,
            scroll: 0,
            run: self.run,
            shell: Shell::PowerShell,
            cwd: self.cwd,
            input: self.input,
            selection: (end, end),
            focused: self.focused,
            suggestions: self.suggestions.clone(),
            selected: 0,
            tall: self.tall,
            from_explorer: self.from_explorer,
            menu: self.menu.as_ref(),
        }
    }
}

/// The design's terminals (`OUT_READY`, `OUT_RUN`, `OUT_ERR`, `OUT_TALL`),
/// with commands echoed the way the island echoes them.
fn demo_cli(which: CliDemo, now: Instant) -> DemoCli {
    let mut output = Output::default();
    let mut echoes = Echoes::default();
    let mut command = |output: &mut Output, cmd: &str, lines: &[&str]| {
        echoes.mark(output);
        output.push(cli::echo_line(cmd));
        for line in lines {
            output.push((*line).to_owned());
        }
    };
    let ready = |output: &mut Output, command: &mut dyn FnMut(&mut Output, &str, &[&str])| {
        command(
            output,
            "git status",
            &["On branch main · nothing to commit, working tree clean"],
        );
        command(
            output,
            "ls *.dwg",
            &["denah-lt1.dwg   denah-lt2.dwg   tampak-depan.dwg"],
        );
    };
    let secs = |s: f32| Duration::from_secs_f32(s);
    let mut run = RunState::Idle;
    match which {
        CliDemo::Empty => {}
        CliDemo::Ready | CliDemo::Typing => ready(&mut output, &mut command),
        CliDemo::Running => {
            command(
                &mut output,
                "npm run build",
                &[
                    "> rumah-kemang@1.4.0 build",
                    "> vite build",
                    "vite v6.0.7 building for production...",
                    "\u{2713} 1284 modules transformed.",
                    "rendering chunks (12/18)...",
                ],
            );
            run = RunState::Running {
                since: now.checked_sub(secs(42.3)).unwrap_or(now),
            };
        }
        CliDemo::Done => {
            ready(&mut output, &mut command);
            command(
                &mut output,
                "git pull",
                &["Already up to date.", &cli::exit_line(Some(0), false)],
            );
            run = RunState::Ended {
                code: Some(0),
                killed: false,
                took: secs(2.1),
            };
        }
        CliDemo::Error => {
            command(
                &mut output,
                "npm test",
                &[
                    "PASS src/denah.test.ts",
                    "FAIL src/rab.test.ts",
                    "  \u{25CF} hitungRAB \u{203A} total termasuk PPN",
                    "    Expected: 12.210.000 \u{00B7} Received: 11.000.000",
                    "Tests: 1 failed, 23 passed, 24 total",
                ],
            );
            run = RunState::Ended {
                code: Some(1),
                killed: false,
                took: secs(3.8),
            };
        }
        CliDemo::Tall => {
            let log: Vec<String> = [
                ("b83979b", "fix: shortcut programs by full path"),
                ("bc85b93", "fix: shortcuts keep the panel closed"),
                ("118800e", "docs: english readme for v2.2"),
                ("77d3f16", "feat: shortcuts page and cli"),
                ("4106ddd", "fix: lyrics wait for the new track"),
                ("9c1e2aa", "feat: synced lyrics in the notch"),
                ("5d0b7f1", "feat: reminders"),
                ("a2e44c9", "feat: idle clock"),
                ("e81f3b0", "feat: settings window"),
                ("0b9d2c6", "feat: pomodoro ring"),
                ("7f3a510", "feat: google calendar"),
                ("3c6e9d1", "init"),
            ]
            .iter()
            .map(|(hash, msg)| format!("{hash} {msg}"))
            .collect();
            let lines: Vec<&str> = log.iter().map(String::as_str).collect();
            ready(&mut output, &mut command);
            command(&mut output, "git log --oneline -12", &lines);
        }
        CliDemo::FromExplorer => {
            command(
                &mut output,
                "git status",
                &["On branch main \u{00B7} nothing to commit"],
            );
            output.push(crate::i18n::cli_moved_to(
                crate::i18n::lang(),
                r"D:\github\project-a",
            ));
        }
        CliDemo::FolderMenu => ready(&mut output, &mut command),
    }
    let menu = (which == CliDemo::FolderMenu).then(demo_folder_menu);
    DemoCli {
        output,
        echoes,
        run,
        input: if which == CliDemo::Typing { "git" } else { "" },
        focused: which != CliDemo::Empty,
        suggestions: if which == CliDemo::Typing {
            vec!["git status", "git pull", "git log --oneline -12"]
        } else {
            Vec::new()
        },
        tall: which == CliDemo::Tall,
        cwd: match which {
            CliDemo::FromExplorer | CliDemo::FolderMenu => r"D:\github\project-a",
            _ => "D:\\Proyek\\rumah-kemang",
        },
        from_explorer: which == CliDemo::FromExplorer,
        menu,
    }
}

/// The design's menu: two Explorer windows, one favourite, two recent
/// folders (2 minutes ago and yesterday), timed from a fixed "now" of
/// 27 October 2026, 14.32, so the screenshot never depends on the clock.
fn demo_folder_menu() -> FolderMenuView {
    let day = NaiveDate::from_ymd_opt(2026, 10, 27).expect("valid date");
    let at = |d: NaiveDate, h, m| d.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time"));
    let now = at(day, 14, 32);
    let explorer = [
        std::path::PathBuf::from(r"D:\github\project-a"),
        std::path::PathBuf::from(r"D:\Proyek\rumah-kemang\gambar"),
    ];
    let mut favorites = FolderList::new(ListKind::Favorites);
    favorites.set_favorite(std::path::Path::new(r"D:\github\dynamic-island"), true);
    let mut recent = FolderList::new(ListKind::Recent);
    let yesterday = day.pred_opt().expect("valid date");
    recent.push_recent(
        std::path::Path::new(r"C:\Users\hartono"),
        stamp(at(yesterday, 17, 10)),
    );
    recent.push_recent(
        std::path::Path::new(r"D:\Proyek\rumah-kemang"),
        stamp(at(day, 14, 30)),
    );
    let sections = crate::cli_folders::menu_rows(&explorer, &favorites, &recent, now, &|_| true);
    FolderMenuView {
        items: crate::cli_folders::menu_items(&sections),
    }
}

/// The scene's shape; a lyric scene's notch fits its line at `lyric_at` as
/// the island fits it on entering the view (measured, capped by the screen).
fn scene_shape(
    renderer: &Renderer,
    win: &IslandWindow,
    style: Style,
    scene: &Scene,
    lyric_at: u64,
) -> ShapeRect {
    let robot = scene.robot.is_some();
    if scene.kind != ShapeKind::MediaLyric {
        return shape_with_robot(style, scene.kind, robot);
    }
    let state = demo_lyrics(lyric_at, None, Instant::now());
    let text_w = state
        .shown(Instant::now())
        .and_then(|l| renderer.lyric_text_width(l.text));
    let w = lyric_width(text_w, robot, lyric_max_w(win.screen_w()));
    lyric_shape(style, w)
}

fn scene(scenes: &[Scene], step: usize) -> (Style, Scene) {
    let style = if step < scenes.len() {
        Style::Pill
    } else {
        Style::Notch
    };
    (style, scenes[step % scenes.len()])
}

/// Focus session 2 of 4 with 18:42 left (running or paused), a short break
/// ready to start, or one running with 3:20 left.
fn demo_pomodoro(which: Pomo, now: Instant) -> Pomodoro {
    let mut p = Pomodoro::new(PomoSettings::default());
    if which == Pomo::Idle {
        return p;
    }
    if which == Pomo::Ready {
        p.skip(now);
        return p;
    }
    if which == Pomo::Break {
        // Focus 1 → short break, 1:40 in.
        p.skip(now);
        p.start_pause(now.checked_sub(Duration::from_secs(100)).unwrap_or(now));
        return p;
    }
    // Focus 1 → short break → focus 2.
    p.skip(now);
    p.skip(now);
    let started = now.checked_sub(Duration::from_secs(378)).unwrap_or(now);
    p.start_pause(started);
    if which == Pomo::Paused {
        p.start_pause(now);
    }
    p
}

fn local(d: NaiveDate, h: u32, m: u32) -> EventTime {
    let t = d.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("valid time"));
    EventTime::At(
        Local
            .from_local_datetime(&t)
            .earliest()
            .expect("local time"),
    )
}

/// Today: an exhibition running over several days and a birthday (all
/// day), a review three hours before the current hour and a meeting the hour
/// after it (so the day view shows the "now" line between them, as in the
/// design), a coffee at 07.00 and a render from 22.00 past midnight: they
/// never fit one window, so the day view always has a second page ("+N
/// lagi", then "Kembali ke atas"). The rest of the week has an event most
/// days, one of them with a very long title.
fn demo_calendar(today: NaiveDate, p: &Palette) -> CalendarSnapshot {
    let day = |n: i64| {
        if n >= 0 {
            today + Days::new(n as u64)
        } else {
            today - Days::new(n.unsigned_abs())
        }
    };
    let ev = |id: &str, title: &str, start, end, color| CalEvent {
        id: id.into(),
        calendar_id: "primary".into(),
        title: title.into(),
        start,
        end,
        description: String::new(),
        color,
        editable: true,
    };
    // Keeps the review on today and the meeting before midnight.
    let h = Local::now().hour().clamp(3, 20);
    let events = vec![
        ev(
            "expo",
            "Pameran arsitektur Jakarta",
            EventTime::AllDay(day(-1)),
            EventTime::AllDay(day(3)),
            Some(p.sage),
        ),
        ev(
            "bday",
            "Ulang tahun Rani",
            EventTime::AllDay(today),
            EventTime::AllDay(day(1)),
            Some(p.plum),
        ),
        ev(
            "standup",
            "Kopi pagi dengan tim",
            local(today, 7, 0),
            local(today, 7, 45),
            Some(p.plum),
        ),
        ev(
            "review",
            "Review desain klien",
            local(today, h - 3, 0),
            local(today, h - 2, 30),
            None,
        ),
        ev(
            "rapat",
            "Rapat koordinasi MEP",
            local(today, h + 1, 0),
            local(today, h + 2, 0),
            Some(p.ochre),
        ),
        ev(
            "render",
            "Render final malam",
            local(today, 22, 0),
            local(day(1), 2, 0),
            None,
        ),
        ev(
            "survey",
            "Survey lokasi Kemang",
            local(day(1), 8, 30),
            local(day(1), 10, 0),
            Some(p.sage),
        ),
        ev(
            "present",
            "Presentasi",
            local(day(2), 13, 0),
            local(day(2), 14, 0),
            None,
        ),
        ev(
            "gym",
            "Gym",
            local(day(4), 7, 0),
            local(day(4), 8, 0),
            Some(p.plum),
        ),
        ev(
            "rab",
            "Kirim RAB",
            local(day(4), 16, 0),
            local(day(4), 17, 0),
            Some(p.ochre),
        ),
        ev(
            "family",
            "Keluarga",
            local(day(6), 11, 0),
            local(day(6), 13, 0),
            Some(p.sage),
        ),
        ev(
            "long",
            "Rapat koordinasi kontraktor, struktur, MEP dan pemilik proyek gedung kantor",
            local(day(-5), 9, 0),
            local(day(-5), 10, 0),
            None,
        ),
        ev(
            "site",
            "Cek progres lapangan",
            local(day(-12), 14, 0),
            local(day(-12), 15, 0),
            None,
        ),
    ];
    CalendarSnapshot {
        status: CalStatus::Ok,
        events,
        from: day(-30),
        to: day(30),
        fetched_at: Some(Local::now()),
    }
}

fn stamp(t: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map_or(0, |t| t.timestamp())
}

/// The design's four cards (newest first), then two more for the second
/// chunk: one with a single line, one from last year.
fn demo_notes(today: NaiveDate) -> Vec<Note> {
    let now = Local::now().timestamp();
    let at = |d: NaiveDate, h, m| stamp(d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()));
    let note = |id: &str, text: &str, updated| Note {
        id: id.into(),
        text: text.into(),
        updated,
    };
    vec![
        note(
            "n1",
            "Ide fasad\nKisi-kisi kayu vertikal, jarak 8 cm, bayangan sore jatuh ke ruang tamu.",
            now - 10 * 60,
        ),
        note(
            "n2",
            "Daftar material\nBata ekspos, terakota lantai 30×30, kaca low-e, baja ringan.",
            now - 3600,
        ),
        note(
            "n3",
            "\n  Catatan rapat\nKlien minta skylight di tangga. Cek ulang beban atap, \
             lalu kirim revisi potongan dan detail kusen jendela kamar tidur.",
            at(today - Days::new(1), 16, 0),
        ),
        note(
            "n4",
            "Belanja\nKopi, kertas kalkir A2, pensil 2B, cutter.",
            at(today - Days::new(4), 8, 0),
        ),
        note(
            "n5",
            "Perpanjang lisensi software",
            at(today - Days::new(9), 11, 0),
        ),
        note(
            "n6",
            "Arsip proyek lama\nPindahkan ke drive eksternal.",
            at(today - Days::new(400), 11, 0),
        ),
    ]
}

fn many_notes(today: NaiveDate) -> Vec<Note> {
    let base = stamp(today.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()));
    (0..30)
        .map(|i| Note {
            id: format!("m{i}"),
            text: format!("Catatan nomor {}", i + 1),
            updated: base - i64::from(i) * 7 * 3600,
        })
        .collect()
}

/// Today: one overdue ("Terlewat"), one later, one daily (repeat icon);
/// tomorrow one; and a weekly one later in the week (a date heading).
fn demo_reminders(today: NaiveDate) -> Vec<Reminder> {
    let now = Local::now().naive_local();
    let whole = |t: NaiveDateTime| t.with_second(0).unwrap_or(t);
    // The two later ones stay before midnight, so they stay under today.
    let left = i64::from(24 * 60 - 1 - (now.hour() * 60 + now.minute()));
    let later = |want: i64, share: i64, min: i64| {
        now + chrono::Duration::minutes(want.min(left * share / 3).max(min))
    };
    let r = |id: &str, text: &str, at: NaiveDateTime, repeat| Reminder {
        id: id.into(),
        text: text.into(),
        due_unix: stamp(at),
        repeat,
        done: false,
        days: Vec::new(),
    };
    let on = |d: NaiveDate, h, m| d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap());
    vec![
        r(
            "r1",
            "Minum vitamin",
            whole(now - chrono::Duration::hours(2)),
            Repeat::Once,
        ),
        r(
            "r2",
            "Kirim revisi denah lantai 2 ke klien",
            whole(later(90, 1, 1)),
            Repeat::Once,
        ),
        r(
            "r3",
            "Backup file proyek",
            whole(later(150, 2, 2)),
            Repeat::Daily,
        ),
        r(
            "r4",
            "Survey lokasi Kemang",
            on(today + Days::new(1), 9, 30),
            Repeat::Once,
        ),
        r(
            "r5",
            "Telepon vendor kaca soal kaca low-e untuk fasad depan dan skylight tangga",
            on(today + Days::new(3), 13, 0),
            Repeat::Weekly,
        ),
    ]
}

/// Twelve reminders over the coming days.
fn many_reminders(today: NaiveDate) -> Vec<Reminder> {
    let repeats = [Repeat::Once, Repeat::Daily, Repeat::Weekly];
    (0..12u32)
        .map(|i| {
            let day = today + Days::new(u64::from(i / 3));
            let at = day.and_time(NaiveTime::from_hms_opt(8 + (i % 3) * 4, 30, 0).unwrap());
            Reminder {
                id: format!("m{i}"),
                text: format!("Pengingat nomor {}", i + 1),
                due_unix: stamp(at),
                repeat: repeats[i as usize % 3],
                done: false,
                days: Vec::new(),
            }
        })
        .collect()
}

/// A 240×192 cover like the design's: a terracotta sky darkening downwards,
/// a pale sun, a thin horizon. Not square, to exercise the crop.
fn test_art() -> Vec<u8> {
    const W: u32 = 240;
    const H: u32 = 192;
    let row = W * 3; // 720 bytes: already 4-byte aligned
    let pixels = row * H;
    let mut bmp = Vec::with_capacity((54 + pixels) as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + pixels).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(W as i32).to_le_bytes());
    bmp.extend_from_slice(&(H as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0u8; 24]);
    let stops: [(f32, [f32; 3]); 3] = [
        (0.0, [0xE2 as f32, 0x68 as f32, 0x3C as f32]),
        (0.55, [0xB4 as f32, 0x45 as f32, 0x2A as f32]),
        (1.0, [0x5A as f32, 0x2A as f32, 0x1C as f32]),
    ];
    let sky = |t: f32| -> [f32; 3] {
        let (a, b) = if t < stops[1].0 {
            (stops[0], stops[1])
        } else {
            (stops[1], stops[2])
        };
        let k = (t - a.0) / (b.0 - a.0);
        std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * k)
    };
    let sun = [0xF2 as f32, 0xC2 as f32, 0x7A as f32];
    let line = [0xD8 as f32, 0xB8 as f32, 0xA0 as f32];
    // BMP rows run bottom-up.
    for y in (0..H).rev() {
        for x in 0..W {
            let (fx, fy) = (x as f32, y as f32);
            let mut c = sky((fy / (H - 1) as f32 + fx / (W - 1) as f32 * 0.15).min(1.0));
            // The sun's edge blends over a pixel, so it stays round scaled up.
            let (dx, dy) = (fx + 0.5 - W as f32 * 0.68, fy + 0.5 - H as f32 * 0.3);
            let edge = H as f32 * 0.19 - (dx * dx + dy * dy).sqrt();
            let cover = (edge + 0.5).clamp(0.0, 1.0);
            c = std::array::from_fn(|i| c[i] + (sun[i] - c[i]) * cover);
            if y == H * 78 / 100 {
                c = std::array::from_fn(|i| (c[i] + line[i]) / 2.0);
            }
            bmp.extend_from_slice(&[c[2] as u8, c[1] as u8, c[0] as u8]);
        }
    }
    bmp
}

/// Dispatches pending messages; false once WM_QUIT arrives.
fn pump_messages() -> bool {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The Pintasan page's tiles: the built-in eight, or a made-up own set (a
/// few built-ins, a web address, a folder, an app) for the custom scene.
fn scene_tiles(scene: &Scene) -> Vec<crate::tiles::Tile> {
    let mut tiles = ShortcutTiles::default();
    if scene.custom_tiles {
        for i in [7, 6, 5, 4] {
            tiles.toggle(i);
        }
        for (label, kind, target) in [
            ("Gmail", CustomKind::Url, "https://mail.google.com"),
            ("Proyek", CustomKind::Folder, r"D:\Proyek"),
            ("Editor", CustomKind::App, r"C:\Tools\editor.exe"),
            ("Docs", CustomKind::Url, "https://docs.google.com"),
        ] {
            if let Some(tile) = CustomTile::new(label, kind, target) {
                tiles.add(tile);
            }
        }
    }
    tiles.visible()
}

/// Made-up volumes for the Mixer scene: never the machine's own.
fn demo_mixer() -> Vec<crate::audio::MixerRow> {
    use crate::audio::{MixerRow, Target};
    let app = |name: &str, level: f32, muted: bool| MixerRow {
        target: Target::App(name.to_lowercase()),
        name: name.into(),
        level,
        muted,
    };
    vec![
        MixerRow {
            target: Target::Master,
            name: String::new(),
            level: 0.62,
            muted: false,
        },
        app("Spotify", 0.8, false),
        app("Chrome", 0.45, false),
        app("Teams", 0.2, true),
    ]
}

/// What the quick search's demo scene has typed.
const DEMO_SEARCH_QUERY: &str = "kal";

/// The demo's results, from a made-up list of apps (never the real Start
/// Menu): what `search::search` makes of "kal".
fn demo_search_results() -> Vec<crate::search::SearchResult> {
    use crate::search::{AppEntry, Labels};
    let apps: Vec<AppEntry> = ["Kalender", "Kalkulator", "Kamera", "Notepad", "Visual Studio Code"]
        .iter()
        .map(|name| AppEntry {
            name: (*name).into(),
            path: format!(r"C:\Demo\{name}.lnk"),
        })
        .collect();
    let lang = crate::i18n::lang();
    let web_title = move |q: &str| crate::i18n::search_web_title(lang, q);
    crate::search::search(
        DEMO_SEARCH_QUERY,
        &apps,
        lang,
        &Labels {
            app: tr(Key::SearchKindApp),
            setting: tr(Key::SearchKindSetting),
            copy: tr(Key::SearchCopy),
            web: tr(Key::SearchKindWeb),
            web_title: &web_title,
        },
    )
}

/// Made-up files for the shelf scene (and an empty shelf elsewhere): never
/// the machine's own. One of them is gone, as a card shows it.
fn demo_shelf(filled: bool) -> Vec<crate::shelf::ShelfCard> {
    use crate::shelf::{ShelfCard, ShelfKind};
    if !filled {
        return Vec::new();
    }
    let card = |name: &str, kind, stale| ShelfCard {
        name: name.into(),
        kind,
        stale,
    };
    vec![
        card("denah-lantai-2-revisi-final.pdf", ShelfKind::Document, false),
        card("foto-lokasi.png", ShelfKind::Image, false),
        card("Gambar kerja", ShelfKind::Folder, false),
        card("rab-v3.xlsx", ShelfKind::Document, false),
        card("render.mp4", ShelfKind::Media, false),
        card("arsip-lama.zip", ShelfKind::Archive, true),
    ]
}
