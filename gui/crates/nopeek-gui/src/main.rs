//! No-Peek Dynamic Island GUI Entry Point.
//! See `plan.md` at the repository root for the full Win32/Direct2D (Windows)
//! and AppKit/CoreGraphics (macOS) implementation blueprint derived from `../oyenza-studio`.

use nopeek_core::config::GuiConfig;
use nopeek_core::island::Island;

fn main() {
    let (cfg, path) = GuiConfig::find_and_load();
    let island = Island::new(&cfg);
    println!(
        "No-Peek GUI Scaffolding Ready | mode={:?} | ipc={:?} | idle={}x{} DIP | dotenv={:?}",
        cfg.env_mode,
        cfg.ipc_mode,
        island.current().w,
        island.current().h,
        path
    );
}
