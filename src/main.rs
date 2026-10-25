#![windows_subsystem = "windows"]

mod app;
mod calendar;
mod claude;
mod config;
mod google;
mod i18n;
mod model;
mod notes;
mod pomodoro;
mod reminders;
mod sources;
mod state;
mod theme;
mod ui;

use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// The release binary uses the GUI subsystem, so CLI modes must attach to the
/// launching console before anything they print becomes visible.
fn attach_parent_console() {
    // Fails harmlessly when there is no parent console (e.g. started by a hook runner with pipes).
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Demo modes (debug builds) read no config: `DI_LANG=en` picks English.
#[cfg(debug_assertions)]
fn demo_lang_from_env() {
    let code = std::env::var("DI_LANG").unwrap_or_default();
    i18n::set_lang(i18n::Lang::from_code(&code).unwrap_or_default());
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => app::run(),
        // No console attach: the hook prints nothing, and attaching can swap an
        // unset stdin for the console's, leaving the hook blocked on keyboard input.
        Some("--hook") => claude::hook::run_hook(),
        Some("--probe-notifications") => {
            attach_parent_console();
            sources::notifications::probe();
        }
        #[cfg(debug_assertions)]
        Some("--probe-media") => {
            attach_parent_console();
            sources::media::probe();
        }
        #[cfg(debug_assertions)]
        Some("--probe-system") => {
            attach_parent_console();
            sources::battery::probe();
        }
        #[cfg(debug_assertions)]
        Some("--demo") => {
            attach_parent_console();
            demo_lang_from_env();
            ui::demo::run();
        }
        #[cfg(debug_assertions)]
        Some("--demo-editor") => {
            attach_parent_console();
            demo_lang_from_env();
            ui::editor::run_demo(std::env::args().nth(2).as_deref());
        }
        #[cfg(debug_assertions)]
        Some("--settings") => {
            attach_parent_console();
            demo_lang_from_env();
            let args: Vec<String> = std::env::args().skip(2).collect();
            ui::settings::run_demo(&args);
        }
        Some("--install-hooks") => {
            attach_parent_console();
            claude::install::install_hooks();
        }
        Some("--uninstall-hooks") => {
            attach_parent_console();
            claude::install::uninstall_hooks();
        }
        Some(other) => {
            attach_parent_console();
            eprintln!("dynamic-island: unknown argument `{other}`");
            std::process::exit(2);
        }
    }
}
