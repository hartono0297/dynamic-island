#![windows_subsystem = "windows"]

mod app;
mod calendar;
mod claude;
mod config;
mod google;
mod model;
mod notes;
mod pomodoro;
mod sources;
mod state;
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
            ui::demo::run();
        }
        #[cfg(debug_assertions)]
        Some("--demo-editor") => {
            attach_parent_console();
            ui::editor::run_demo(std::env::args().nth(2).as_deref());
        }
        Some("--install-hooks") => {
            attach_parent_console();
            claude::install::install_hooks();
        }
        Some(other) => {
            attach_parent_console();
            eprintln!("dynamic-island: unknown argument `{other}`");
            std::process::exit(2);
        }
    }
}
