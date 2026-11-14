//! System Tray icon and context menu for Windows.
//! Adapted from `oyenza-studio/crates/poni/src/tray.rs`.

use crate::events::WM_APP_TRAY;
use nopeek_core::controls::{menu as menu_entries, MenuEntry};
pub use nopeek_core::controls::{MenuAction as TrayAction, TrayState};
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

struct Popup(HMENU);
impl Drop for Popup {
    fn drop(&mut self) {
        if let Err(error) = unsafe { DestroyMenu(self.0) } {
            eprintln!("[nopeek-gui] Failed to release menu: {error}");
        }
    }
}

fn build_menu(items: &[MenuEntry], actions: &mut Vec<TrayAction>) -> Result<Popup> {
    let popup = Popup(unsafe { CreatePopupMenu()? });
    for entry in items {
        unsafe {
            match entry {
                MenuEntry::Separator => AppendMenuW(popup.0, MF_SEPARATOR, 0, PCWSTR::null())?,
                MenuEntry::Info(label) => {
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    AppendMenuW(popup.0, MF_STRING | MF_GRAYED, 0, PCWSTR(wide.as_ptr()))?;
                }
                MenuEntry::Action {
                    label,
                    action,
                    checked,
                    enabled,
                } => {
                    actions.push(*action);
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    let flags = MF_STRING
                        | if *checked { MF_CHECKED } else { MF_UNCHECKED }
                        | if *enabled { MF_ENABLED } else { MF_GRAYED };
                    AppendMenuW(popup.0, flags, actions.len(), PCWSTR(wide.as_ptr()))?;
                }
                MenuEntry::Submenu { label, items } => {
                    let child = build_menu(items, actions)?;
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    AppendMenuW(
                        popup.0,
                        MF_STRING | MF_POPUP,
                        child.0 .0 as usize,
                        PCWSTR(wide.as_ptr()),
                    )?;
                    std::mem::forget(child);
                }
            }
        }
    }
    Ok(popup)
}

pub fn show_context_menu(owner: HWND, pt: POINT, state: &TrayState) -> Option<TrayAction> {
    let mut actions = Vec::new();
    let popup = match build_menu(&menu_entries(state), &mut actions) {
        Ok(menu) => menu,
        Err(error) => {
            eprintln!("[nopeek-gui] Failed to build settings menu: {error}");
            return None;
        }
    };
    unsafe {
        let prev = GetForegroundWindow();
        if !SetForegroundWindow(owner).as_bool() {
            eprintln!("[nopeek-gui] Windows declined menu foreground activation");
        }
        let chosen = TrackPopupMenu(
            popup.0,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            owner,
            None,
        );
        if let Err(error) = PostMessageW(owner, WM_NULL, WPARAM(0), LPARAM(0)) {
            eprintln!("[nopeek-gui] Failed to dismiss menu focus: {error}");
        }
        if !prev.is_invalid()
            && prev != owner
            && IsWindow(prev).as_bool()
            && !SetForegroundWindow(prev).as_bool()
        {
            eprintln!("[nopeek-gui] Windows declined restoring foreground window");
        }
        let index = chosen.0 as usize;
        index.checked_sub(1).and_then(|i| actions.get(i).copied())
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
