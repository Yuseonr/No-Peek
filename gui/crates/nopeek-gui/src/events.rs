//! Cross-thread messages between service threads and the UI thread.
//! Adapted from `oyenza-studio/crates/poni/src/events.rs`.

use nopeek_core::ipc::{IpcCommand, IpcEvent};
use std::sync::mpsc::Sender;

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

#[cfg(windows)]
pub const WM_APP_EVENT: u32 = WM_APP + 1;
#[cfg(windows)]
pub const WM_APP_FRAME: u32 = WM_APP + 2;
#[cfg(windows)]
pub const WM_APP_TRAY: u32 = WM_APP + 3;

/// An HWND handle that safely crosses thread boundaries for `PostMessageW`.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendHwnd(pub isize);
#[cfg(windows)]
unsafe impl Send for SendHwnd {}
#[cfg(windows)]
unsafe impl Sync for SendHwnd {}

#[cfg(windows)]
impl SendHwnd {
    pub fn new(h: HWND) -> Self {
        Self(h.0 as isize)
    }
    pub fn hwnd(self) -> HWND {
        HWND(self.0 as *mut core::ffi::c_void)
    }
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    /// Inbound IPC event from Python Inference worker
    Ipc(IpcEvent),
    /// Notification that the Python worker process exited
    IpcExited { code: Option<i32> },
    /// Outbound command triggered from System Tray or Menu Bar
    TrayCommand(IpcCommand),
    /// Request to quit the application
    Quit,
}

#[cfg(windows)]
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<AppEvent>,
    hwnd: SendHwnd,
}

#[cfg(windows)]
impl EventSink {
    pub fn new(tx: Sender<AppEvent>, hwnd: SendHwnd) -> Self {
        Self { tx, hwnd }
    }

    pub fn send(&self, ev: AppEvent) {
        if self.tx.send(ev).is_ok() {
            unsafe {
                let _ = PostMessageW(self.hwnd.hwnd(), WM_APP_EVENT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<AppEvent>,
}

#[cfg(target_os = "macos")]
impl EventSink {
    pub fn new(tx: Sender<AppEvent>) -> Self {
        Self { tx }
    }

    pub fn send(&self, ev: AppEvent) {
        if self.tx.send(ev).is_ok() {
            dispatch2::DispatchQueue::main().exec_async(|| {
                crate::mac::app::drain_events();
            });
        }
    }
}
