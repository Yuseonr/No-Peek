#![cfg_attr(windows, windows_subsystem = "windows")]

//! No-Peek Dynamic Island GUI Entry Point for Windows and macOS.
//! See `plan.md` for architectural specifications.

mod clock;
mod events;
mod services;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod gfx;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod window;

#[cfg(target_os = "macos")]
mod mac;

use nopeek_core::config::GuiConfig;

#[cfg(windows)]
fn run_app(cfg: GuiConfig) -> i32 {
    use nopeek_core::config::EnvMode;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    if cfg.env_mode == EnvMode::Dev {
        use windows::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS).or_else(|_| AllocConsole());
        }
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let code = match app::run(cfg) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("[nopeek-gui] Fatal error: {err}");
            1
        }
    };

    unsafe {
        CoUninitialize();
    }

    code
}

#[cfg(target_os = "macos")]
fn run_app(cfg: GuiConfig) -> i32 {
    match mac::app::run(cfg) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("[nopeek-gui] Fatal error: {err}");
            1
        }
    }
}

fn main() {
    let (cfg, env_path) = GuiConfig::find_and_load();

    println!(
        "[nopeek-gui] Starting Dynamic Island | env={:?} | ipc={:?} | .env={:?}",
        cfg.env_mode, cfg.ipc_mode, env_path
    );

    let exit_code = run_app(cfg);
    std::process::exit(exit_code);
}
