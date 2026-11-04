//! Configuration loader for `.env` (shared between Rust GUI and Python Inference).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvMode {
    /// Show OpenCV camera debug window alongside Dynamic Island GUI
    Dev,
    /// Hide OpenCV camera window (`headless`), run in System Tray (Windows) / Menu Bar (macOS)
    Prod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcMode {
    /// Parent Rust spawns Python subprocess and reads/writes JSONL over stdio
    Stdio,
    /// Connect/Listen over local TCP socket (`127.0.0.1:<port>`)
    Tcp,
}

#[derive(Clone, Debug)]
pub struct GuiConfig {
    pub env_mode: EnvMode,
    pub ipc_mode: IpcMode,
    pub ipc_host: String,
    pub ipc_port: u16,
    pub idle_width: f32,
    pub idle_height: f32,
    pub alert_height: f32,
    pub alert_min_width: f32,
    pub alert_max_width: f32,
    pub alert_margin_x: f32,
    pub alert_margin_y: f32,
    pub text_alert: String,
    pub text_escalated: String,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            env_mode: EnvMode::Dev,
            ipc_mode: IpcMode::Stdio,
            ipc_host: "127.0.0.1".to_string(),
            ipc_port: 17890,
            idle_width: 140.0,
            idle_height: 8.0,
            alert_height: 44.0,
            alert_min_width: 240.0,
            alert_max_width: 460.0,
            alert_margin_x: 18.0,
            alert_margin_y: 10.0,
            text_alert: "Pengintip Terdeteksi".to_string(),
            text_escalated: "Bahaya! Layar Sedang Diintip".to_string(),
        }
    }
}

impl GuiConfig {
    /// Parse a `.env` file content string into key-value pairs.
    pub fn parse_dotenv(content: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim().to_string();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() {
                    map.insert(key, val);
                }
            }
        }
        map
    }

    pub fn from_map(map: &HashMap<String, String>) -> Self {
        let mut cfg = Self::default();
        let get = |k: &str| -> Option<String> {
            std::env::var(k).ok().or_else(|| map.get(k).cloned())
        };
        if let Some(m) = get("NOPEEK_ENV") {
            cfg.env_mode = if m.eq_ignore_ascii_case("prod") || m.eq_ignore_ascii_case("production") {
                EnvMode::Prod
            } else {
                EnvMode::Dev
            };
        }
        if let Some(ipc) = get("NOPEEK_IPC_MODE") {
            cfg.ipc_mode = if ipc.eq_ignore_ascii_case("tcp") {
                IpcMode::Tcp
            } else {
                IpcMode::Stdio
            };
        }
        if let Some(h) = get("NOPEEK_IPC_HOST") {
            cfg.ipc_host = h;
        }
        if let Some(p) = get("NOPEEK_IPC_PORT").and_then(|v| v.parse().ok()) {
            cfg.ipc_port = p;
        }
        if let Some(v) = get("NOPEEK_IDLE_WIDTH").and_then(|v| v.parse().ok()) {
            cfg.idle_width = v;
        }
        if let Some(v) = get("NOPEEK_IDLE_HEIGHT").and_then(|v| v.parse().ok()) {
            cfg.idle_height = v;
        }
        if let Some(v) = get("NOPEEK_ALERT_HEIGHT").and_then(|v| v.parse().ok()) {
            cfg.alert_height = v;
        }
        if let Some(v) = get("NOPEEK_ALERT_MIN_WIDTH").and_then(|v| v.parse().ok()) {
            cfg.alert_min_width = v;
        }
        if let Some(v) = get("NOPEEK_ALERT_MAX_WIDTH").and_then(|v| v.parse().ok()) {
            cfg.alert_max_width = v;
        }
        if let Some(v) = get("NOPEEK_ALERT_MARGIN_X").and_then(|v| v.parse().ok()) {
            cfg.alert_margin_x = v;
        }
        if let Some(v) = get("NOPEEK_ALERT_MARGIN_Y").and_then(|v| v.parse().ok()) {
            cfg.alert_margin_y = v;
        }
        if let Some(v) = get("NOPEEK_TEXT_ALERT") {
            cfg.text_alert = v;
        }
        if let Some(v) = get("NOPEEK_TEXT_ESCALATED") {
            cfg.text_escalated = v;
        }
        cfg
    }

    pub fn load_from_path(path: &Path) -> Self {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let map = Self::parse_dotenv(&content);
        Self::from_map(&map)
    }

    pub fn find_and_load() -> (Self, Option<PathBuf>) {
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            let candidate = d.join(".env");
            if candidate.is_file() {
                return (Self::load_from_path(&candidate), Some(candidate));
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        (Self::from_map(&HashMap::new()), None)
    }
}
