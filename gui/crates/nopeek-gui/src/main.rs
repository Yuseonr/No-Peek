#![windows_subsystem = "windows"]

//! No-Peek Dynamic Island GUI Entry Point for Windows.
//! See `plan.md` for architectural specifications.

mod app;
mod clock;
mod events;
mod gfx;
mod services;
mod tray;
mod window;

use nopeek_core::config::{EnvMode, GuiConfig};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

fn main() {
    let (cfg, env_path) = GuiConfig::find_and_load();

    #[cfg(windows)]
    if cfg.env_mode == EnvMode::Dev {
        use windows::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS).or_else(|_| AllocConsole());
        }
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    println!(
        "[nopeek-gui] Starting Dynamic Island | env={:?} | ipc={:?} | .env={:?}",
        cfg.env_mode, cfg.ipc_mode, env_path
    );

    let exit_code = match app::run(cfg) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("[nopeek-gui] Fatal error: {err}");
            1
        }
    };

    unsafe {
        CoUninitialize();
    }

    std::process::exit(exit_code);
}
