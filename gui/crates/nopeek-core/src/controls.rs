use crate::ipc::IpcCommand;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayState {
    pub status_text: String,
    pub mode_text: String,
    pub auto_calibrate: bool,
    pub paused: bool,
    pub ready: bool,
    pub preview: bool,
    pub mirror: bool,
    pub confidence: f32,
    pub alert_after_s: f64,
    pub escalated_after_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Command(IpcCommand),
    OpenSettings,
    Quit,
}

pub enum MenuEntry {
    Info(String),
    Action {
        label: String,
        action: MenuAction,
        checked: bool,
        enabled: bool,
    },
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
    Separator,
}

pub fn settings(state: &TrayState) -> Vec<MenuEntry> {
    let item = |label: &str, command, checked| MenuEntry::Action {
        label: label.into(),
        action: MenuAction::Command(command),
        checked,
        enabled: state.ready,
    };
    vec![
        MenuEntry::Info(state.status_text.clone()),
        MenuEntry::Action {
            label: "Preview kamera".into(),
            action: MenuAction::Command(IpcCommand::SetPreview {
                enabled: !state.preview,
            }),
            checked: state.preview,
            enabled: state.ready && !state.paused,
        },
        MenuEntry::Submenu {
            label: "Sensitivitas deteksi".into(),
            items: [("Peka", 35), ("Seimbang", 50), ("Selektif", 70)]
                .into_iter()
                .map(|(label, percent)| {
                    item(
                        label,
                        IpcCommand::SetConfidence { percent },
                        (state.confidence * 100.0 - percent as f32).abs() < 0.01,
                    )
                })
                .collect(),
        },
        MenuEntry::Submenu {
            label: "Jeda peringatan".into(),
            items: [2, 3, 5]
                .into_iter()
                .map(|seconds| {
                    item(
                        &format!("{seconds} detik"),
                        IpcCommand::SetAlertDelay { seconds },
                        (state.alert_after_s - seconds as f64).abs() < 0.001,
                    )
                })
                .collect(),
        },
        MenuEntry::Info(format!(
            "Eskalasi setelah {:.0} detik",
            state.escalated_after_s
        )),
        item(
            "Cermin posisi kamera",
            IpcCommand::ToggleMirror,
            state.mirror,
        ),
        item(
            "Kalibrasi otomatis",
            IpcCommand::ToggleAutoCalibrate,
            state.auto_calibrate,
        ),
        item(
            if state.paused {
                "Lanjutkan deteksi"
            } else {
                "Jeda deteksi"
            },
            if state.paused {
                IpcCommand::Resume
            } else {
                IpcCommand::Pause
            },
            false,
        ),
        MenuEntry::Submenu {
            label: "Pengguna".into(),
            items: vec![
                item("Kalibrasi", IpcCommand::Calibrate, false),
                item("Reset", IpcCommand::Reset, false),
            ],
        },
        MenuEntry::Info("Pengaturan tersimpan otomatis.".into()),
    ]
}

pub const PANEL_WIDTH: f64 = 380.0;
pub const PANEL_HEIGHT: f64 = 540.0;
pub const PREVIEW_RECT: [f64; 4] = [404.0, 72.0, 332.0, 249.0];
pub fn panel_width(preview: bool) -> f64 {
    if preview {
        760.0
    } else {
        PANEL_WIDTH
    }
}

#[derive(Clone, Debug)]
pub struct PanelControl {
    pub rect: [f64; 4],
    pub label: String,
    pub action: Option<MenuAction>,
    pub checked: bool,
    pub enabled: bool,
    pub checkbox: bool,
}

fn panel_control(entry: &MenuEntry, rect: [f64; 4]) -> Option<PanelControl> {
    match entry {
        MenuEntry::Info(label) => Some(PanelControl {
            rect,
            label: label.clone(),
            action: None,
            checked: false,
            enabled: true,
            checkbox: false,
        }),
        MenuEntry::Action {
            label,
            action,
            checked,
            enabled,
        } => Some(PanelControl {
            rect,
            label: label.clone(),
            action: Some(*action),
            checked: *checked,
            enabled: *enabled,
            checkbox: matches!(
                action,
                MenuAction::Command(
                    IpcCommand::ToggleMirror
                        | IpcCommand::ToggleAutoCalibrate
                        | IpcCommand::SetPreview { .. }
                )
            ),
        }),
        _ => None,
    }
}

pub fn panel_layout(state: &TrayState) -> Vec<PanelControl> {
    let mut controls = Vec::new();
    let mut y = 24.0;
    for (index, entry) in settings(state).iter().enumerate() {
        match entry {
            MenuEntry::Submenu { label, items } => {
                if let Some(control) =
                    panel_control(&MenuEntry::Info(label.clone()), [24.0, y, 332.0, 20.0])
                {
                    controls.push(control);
                }
                y += 24.0;
                let width = (332.0 - 8.0 * (items.len() - 1) as f64) / items.len() as f64;
                for (index, item) in items.iter().enumerate() {
                    if let Some(control) =
                        panel_control(item, [24.0 + index as f64 * (width + 8.0), y, width, 30.0])
                    {
                        controls.push(control);
                    }
                }
                y += 46.0;
            }
            _ => {
                let height = if index == 0 { 40.0 } else { 28.0 };
                if let Some(control) = panel_control(entry, [24.0, y, 332.0, height]) {
                    controls.push(control);
                }
                y += height + 8.0;
            }
        }
    }
    controls
}

pub fn menu(state: &TrayState) -> Vec<MenuEntry> {
    vec![
        MenuEntry::Info(state.status_text.clone()),
        MenuEntry::Action {
            label: "Pengaturan...".into(),
            action: MenuAction::OpenSettings,
            checked: false,
            enabled: true,
        },
        MenuEntry::Separator,
        MenuEntry::Action {
            label: "Keluar".into(),
            action: MenuAction::Quit,
            checked: false,
            enabled: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(items: &[MenuEntry]) -> Vec<(MenuAction, bool, bool)> {
        items
            .iter()
            .flat_map(|item| match item {
                MenuEntry::Action {
                    action,
                    checked,
                    enabled,
                    ..
                } => vec![(*action, *checked, *enabled)],
                MenuEntry::Submenu { items, .. } => actions(items),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn native_menu_uses_acknowledged_preferences_and_disables_startup_actions() {
        let initial = actions(&settings(&TrayState::default()));
        assert!(initial
            .iter()
            .filter(|(a, _, _)| *a != MenuAction::Quit)
            .all(|(_, _, enabled)| !enabled));
        let state = TrayState {
            ready: true,
            preview: true,
            mirror: false,
            confidence: 0.35,
            alert_after_s: 3.0,
            escalated_after_s: 8.0,
            ..Default::default()
        };
        let ready = actions(&settings(&state));
        for command in [
            IpcCommand::SetConfidence { percent: 35 },
            IpcCommand::SetAlertDelay { seconds: 3 },
        ] {
            assert!(ready.contains(&(MenuAction::Command(command), true, true)));
            let serialized = command.to_jsonl();
            assert_eq!(
                serde_json::from_str::<IpcCommand>(&serialized).unwrap(),
                command
            );
        }
        assert!(ready.contains(&(MenuAction::Command(IpcCommand::ToggleMirror), false, true)));
        assert!(actions(&menu(&state)).contains(&(MenuAction::OpenSettings, false, true)));
        assert!(!ready
            .iter()
            .any(|(action, _, _)| *action == MenuAction::Command(IpcCommand::TogglePreview)));
        let event = crate::ipc::IpcEvent::parse_line(r#"{"type":"settings","confidence":0.35,"alert_after_s":3,"escalated_after_s":8,"preview":true,"mirror":false}"#).unwrap();
        assert!(matches!(
            event,
            crate::ipc::IpcEvent::Settings {
                preview: true,
                mirror: false,
                ..
            }
        ));
    }

    #[test]
    fn floating_panel_controls_fit_and_cover_all_preferences() {
        let state = TrayState {
            ready: true,
            confidence: 0.5,
            alert_after_s: 2.0,
            ..Default::default()
        };
        let controls = panel_layout(&state);
        for control in &controls {
            let [x, y, width, height] = control.rect;
            assert!(x >= 0.0 && y >= 0.0 && width > 0.0 && height > 0.0);
            assert!(x + width <= PANEL_WIDTH && y + height <= PANEL_HEIGHT);
        }
        for command in [
            IpcCommand::SetConfidence { percent: 50 },
            IpcCommand::SetAlertDelay { seconds: 2 },
            IpcCommand::ToggleMirror,
            IpcCommand::ToggleAutoCalibrate,
            IpcCommand::Pause,
            IpcCommand::Calibrate,
            IpcCommand::Reset,
        ] {
            assert!(controls.iter().any(|control| control.action
                == Some(MenuAction::Command(command))
                && control.enabled));
        }
        assert!(panel_layout(&TrayState {
            paused: true,
            ..state
        })
        .iter()
        .any(|control| control.action == Some(MenuAction::Command(IpcCommand::Resume))));
        let preview = panel_layout(&TrayState {
            ready: true,
            preview: true,
            ..Default::default()
        });
        assert!(preview.iter().any(|control| control.action
            == Some(MenuAction::Command(IpcCommand::SetPreview {
                enabled: false
            }))
            && control.checked
            && control.enabled));
        let paused = panel_layout(&TrayState {
            ready: true,
            paused: true,
            ..Default::default()
        });
        assert!(paused.iter().any(|control| control.action
            == Some(MenuAction::Command(IpcCommand::SetPreview {
                enabled: true
            }))
            && !control.enabled));
        assert_eq!(panel_width(false), PANEL_WIDTH);
        assert!(PREVIEW_RECT[0] >= PANEL_WIDTH);
        assert!(PREVIEW_RECT[0] + PREVIEW_RECT[2] <= panel_width(true));
        assert!(PREVIEW_RECT[1] + PREVIEW_RECT[3] <= PANEL_HEIGHT);
    }
}
