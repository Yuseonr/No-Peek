//! System Tray icon and context menu for Windows.
//! Adapted from `oyenza-studio/crates/poni/src/tray.rs`.

use crate::events::WM_APP_TRAY;
use nopeek_core::ipc::IpcCommand;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    pub fn add(hwnd: HWND, tooltip: &str) -> Result<Self> {
        unsafe {
            let icon = LoadIconW(None, IDI_APPLICATION)?;
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WM_APP_TRAY,
                hIcon: icon,
                ..Default::default()
            };
            let tip: Vec<u16> = tooltip.encode_utf16().collect();
            let len = tip.len().min(nid.szTip.len() - 1);
            nid.szTip[..len].copy_from_slice(&tip[..len]);

            if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                return Err(windows::core::Error::from_win32());
            }
        }
        Ok(Self { hwnd })
    }

    pub fn update_tooltip(&self, tooltip: &str) {
        unsafe {
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                uFlags: NIF_TIP,
                ..Default::default()
            };
            let tip: Vec<u16> = tooltip.encode_utf16().collect();
            let len = tip.len().min(nid.szTip.len() - 1);
            nid.szTip[..len].copy_from_slice(&tip[..len]);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn remove(self) {
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrayState {
    pub status_text: String,
    pub mode_text: String,
    pub auto_calibrate: bool,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Command(IpcCommand),
    Quit,
}

pub fn show_context_menu(owner: HWND, pt: POINT, state: &TrayState) -> Option<TrayAction> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let mut actions: Vec<TrayAction> = Vec::new();

        // Helper closures to append items
        let append_info = |hmenu: HMENU, text: &str| {
            let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(
                hmenu,
                MF_STRING | MF_GRAYED | MF_DISABLED,
                0,
                PCWSTR(w.as_ptr()),
            );
        };

        let mut append_item = |hmenu: HMENU, label: &str, action: TrayAction, checked: bool| {
            actions.push(action);
            let id = actions.len();
            let w: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
            let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(hmenu, flags, id, PCWSTR(w.as_ptr()));
        };

        let append_sep = |hmenu: HMENU| {
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        };

        // Status lines (info)
        append_info(menu, &state.status_text);
        append_info(menu, &state.mode_text);
        append_sep(menu);

        // Calibration commands
        append_item(
            menu,
            "Calibrate Owner Now",
            TrayAction::Command(IpcCommand::Calibrate),
            false,
        );
        append_item(
            menu,
            "Auto-Calibrate Owner",
            TrayAction::Command(IpcCommand::ToggleAutoCalibrate),
            state.auto_calibrate,
        );
        append_item(
            menu,
            "Reset Tracking",
            TrayAction::Command(IpcCommand::Reset),
            false,
        );
        append_sep(menu);

        // Pause / Resume
        if state.paused {
            append_item(
                menu,
                "Resume Detection",
                TrayAction::Command(IpcCommand::Resume),
                false,
            );
        } else {
            append_item(
                menu,
                "Pause Detection",
                TrayAction::Command(IpcCommand::Pause),
                false,
            );
        }
        append_sep(menu);

        // Quit
        append_item(menu, "Quit No-Peek", TrayAction::Quit, false);

        let prev = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            owner,
            None,
        );
        let _ = PostMessageW(owner, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        if !prev.is_invalid() && prev != owner && IsWindow(prev).as_bool() {
            let _ = SetForegroundWindow(prev);
        }

        let idx = chosen.0 as usize;
        if idx == 0 {
            None
        } else {
            actions.get(idx - 1).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_state_defaults() {
        let state = TrayState::default();
        assert!(!state.auto_calibrate);
        assert!(!state.paused);
        assert!(state.status_text.is_empty());
    }
}
