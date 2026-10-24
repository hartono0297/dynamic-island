#![windows_subsystem = "windows"]

mod app;
mod claude;
mod config;
mod model;
mod sources;
mod state;

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
        Some("--hook") => {
            attach_parent_console();
            claude::hook::run_hook();
        }
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
        Some("--install-hooks") => claude::install::install_hooks(),
        Some(other) => {
            attach_parent_console();
            eprintln!("dynamic-island: unknown argument `{other}`");
            std::process::exit(2);
        }
    }
}
