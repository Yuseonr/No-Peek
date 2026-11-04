//! IPC JSON Lines (NDJSON) protocol contract between Python Inference and Rust GUI.

use serde::{Deserialize, Serialize};

/// Risk level emitted by Python's temporal state machine (`analysis.py`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RiskLevel {
    #[default]
    Monitoring,
    Pending,
    Alert,
    Escalated,
}

impl RiskLevel {
    pub fn is_alert_active(self) -> bool {
        matches!(self, Self::Alert | Self::Escalated)
    }
}

/// Horizontal position of an observer relative to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HorizontalZone {
    Left,
    Center,
    Right,
}

/// Summary of a single observer sent over IPC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObserverSummary {
    pub id: u32,
    pub state: RiskLevel,
    pub position: HorizontalZone,
    pub distance_cm: f32,
    pub risk_s: f32,
}

/// Incoming JSON line message from Python Inference (`stdout`) to Rust GUI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcEvent {
    Ready {
        version: String,
        source: String,
        headless: bool,
    },
    StateUpdate {
        ts: f64,
        level: RiskLevel,
        alert_active: bool,
        icon: String,
        message: String,
        observer_count: usize,
        max_risk_s: f32,
        #[serde(default)]
        observers: Vec<ObserverSummary>,
        calibrated: bool,
        calibration_mode: Option<String>,
        auto_calibrate: bool,
        owner_id: Option<u32>,
        paused: bool,
        fps: f32,
    },
    Calibration {
        calibrated: bool,
        owner_id: Option<u32>,
        mode: Option<String>,
        message: String,
    },
    Error {
        fatal: bool,
        message: String,
    },
}

/// Outgoing JSON line command from Rust GUI (System Tray / Menu Bar) to Python (`stdin`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    Calibrate,
    ToggleAutoCalibrate,
    Reset,
    Pause,
    Resume,
    Quit,
}

impl IpcEvent {
    pub fn parse_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

impl IpcCommand {
    pub fn to_jsonl(&self) -> String {
        let mut s = serde_json::to_string(self).expect("valid JSON command");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_state_update_and_commands() {
        let json_line = r#"{"type":"state_update","ts":1700000000.5,"level":"ALERT","alert_active":true,"icon":"eye_warning","message":"Pengintip Terdeteksi (Kiri)","observer_count":1,"max_risk_s":2.4,"observers":[{"id":2,"state":"ALERT","position":"LEFT","distance_cm":95.0,"risk_s":2.4}],"calibrated":true,"calibration_mode":"AUTO","auto_calibrate":true,"owner_id":1,"paused":false,"fps":29.8}"#;
        let ev = IpcEvent::parse_line(json_line).expect("should parse state_update");
        match ev {
            IpcEvent::StateUpdate { level, alert_active, message, observer_count, .. } => {
                assert_eq!(level, RiskLevel::Alert);
                assert!(alert_active);
                assert_eq!(message, "Pengintip Terdeteksi (Kiri)");
                assert_eq!(observer_count, 1);
            }
            _ => panic!("expected StateUpdate"),
        }

        let cmd = IpcCommand::Calibrate.to_jsonl();
        assert_eq!(cmd, "{\"cmd\":\"calibrate\"}\n");
    }
}
