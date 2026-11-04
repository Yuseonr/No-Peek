//! Win32 window creation, monitor positioning, and VBlank frame clock.
//! Adapted from `oyenza-studio/crates/poni/src/window.rs`.

use crate::clock;
use crate::events::{SendHwnd, WM_APP_FRAME};
use nopeek_core::frametime::{period_for_hz, VsyncTimeline, OFF_RHYTHM_LIMIT};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput};
use windows::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromPoint, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ISLAND_CLASS: PCWSTR = w!("NoPeek.Island");
pub const HELPER_CLASS: PCWSTR = w!("NoPeek.Helper");

pub type WndProc =
    unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> windows::Win32::Foundation::LRESULT;

pub fn register_classes(island_proc: WndProc, helper_proc: WndProc) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(island_proc),
            hInstance: hinst.into(),
            hCursor: cursor,
            lpszClassName: ISLAND_CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let wc2 = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(helper_proc),
            hInstance: hinst.into(),
            hCursor: cursor,
            lpszClassName: HELPER_CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&wc2) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        Ok(())
    }
}

/// Dynamic Island: borderless, click-through, top-most, non-activating, layered tool window.
/// `WS_EX_TRANSPARENT` makes mouse/click events pass completely through to windows underneath.
pub fn create_island_window() -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            ISLAND_CLASS,
            w!("No-Peek Island"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            hinst,
            None,
        )
    }
}

/// Hidden helper window: owns the system tray icon and handles IPC/app notification messages.
pub fn create_helper_window() -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            HELPER_CLASS,
            w!("No-Peek Helper"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            hinst,
            None,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    pub handle: isize,
    pub rect: RECT,
    pub work: RECT,
    pub dpi: u32,
}

impl MonitorInfo {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    pub fn top(&self) -> i32 {
        self.rect.top
    }

    pub fn center_x(&self) -> i32 {
        (self.rect.left + self.rect.right) / 2
    }
}

pub fn monitor_info(hmon: HMONITOR) -> Option<MonitorInfo> {
    unsafe {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        Some(MonitorInfo {
            handle: hmon.0 as isize,
            rect: mi.rcMonitor,
            work: mi.rcWork,
            dpi: dx.max(96),
        })
    }
}

pub fn pick_primary_monitor() -> MonitorInfo {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        monitor_info(hmon).unwrap_or(MonitorInfo {
            handle: 0,
            rect: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            work: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            dpi: 96,
        })
    }
}

pub fn refresh_hz(hmon: HMONITOR) -> u32 {
    unsafe {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(hmon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return 0;
        }
        let mut dm = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm)
            .as_bool()
        {
            dm.dmDisplayFrequency
        } else {
            0
        }
    }
}

fn output_for(hmon: HMONITOR) -> Option<IDXGIOutput> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            let mut j = 0;
            while let Ok(output) = adapter.EnumOutputs(j) {
                if output.GetDesc().is_ok_and(|d| d.Monitor == hmon) {
                    return Some(output);
                }
                j += 1;
            }
            i += 1;
        }
        None
    }
}

enum Vblank {
    Output(IDXGIOutput),
    Dwm,
    Timer,
}

impl Vblank {
    fn for_monitor(hmon: HMONITOR) -> Self {
        match output_for(hmon) {
            Some(o) => Vblank::Output(o),
            None => Vblank::Dwm,
        }
    }

    fn wait(&self, period: f64, next_due: &mut f64) -> bool {
        match self {
            Vblank::Output(o) => unsafe { o.WaitForVBlank().is_ok() },
            Vblank::Dwm => unsafe { DwmFlush().is_ok() },
            Vblank::Timer => {
                let now = clock::mono();
                if *next_due < now || *next_due > now + 2.0 * period {
                    *next_due = now + period;
                }
                let sleep_s = (*next_due - now).max(0.001);
                std::thread::sleep(Duration::from_secs_f64(sleep_s));
                *next_due += period;
                true
            }
        }
    }
}

fn f64_load(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Ordering::Acquire))
}

fn f64_store(a: &AtomicU64, v: f64) {
    a.store(v.to_bits(), Ordering::Release);
}

struct ClockShared {
    active: Mutex<bool>,
    cv: Condvar,
    pending: AtomicBool,
    stop: AtomicBool,
    latest: AtomicU64,
    period: AtomicU64,
    monitor: AtomicIsize,
    resumed: AtomicBool,
    periods: AtomicU32,
    nominal_hz: AtomicU32,
    display_changed: AtomicBool,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameTick {
    pub vblank: f64,
    pub period: f64,
    pub resumed: bool,
    pub periods: u32,
}

/// Posts `WM_APP_FRAME` synchronized with the display's VBlank while active,
/// and parks/sleeps at 0% CPU when inactive (`island.settled() == true`).
pub struct FrameClock {
    shared: Arc<ClockShared>,
}

impl FrameClock {
    pub fn start(hwnd: SendHwnd, monitor: isize) -> Self {
        let hz = refresh_hz(HMONITOR(monitor as *mut core::ffi::c_void));
        let shared = Arc::new(ClockShared {
            active: Mutex::new(false),
            cv: Condvar::new(),
            pending: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            latest: AtomicU64::new(clock::mono().to_bits()),
            period: AtomicU64::new(period_for_hz(hz).to_bits()),
            monitor: AtomicIsize::new(monitor),
            resumed: AtomicBool::new(true),
            periods: AtomicU32::new(1),
            nominal_hz: AtomicU32::new(hz),
            display_changed: AtomicBool::new(false),
        });
        let s = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("nopeek-frame-clock".into())
            .spawn(move || clock_thread(s, hwnd))
            .expect("spawn frame clock thread");
        Self { shared }
    }

    /// Wake (true) or park (false) the VBlank clock thread.
    pub fn set_active(&self, on: bool) {
        let mut a = self.shared.active.lock().unwrap();
        if *a != on {
            *a = on;
            self.shared.cv.notify_all();
        }
    }

    pub fn set_monitor(&self, hmon: isize) {
        self.shared.monitor.store(hmon, Ordering::Relaxed);
        self.shared.display_changed.store(true, Ordering::Release);
    }

    pub fn period(&self) -> f64 {
        f64_load(&self.shared.period)
    }

    pub fn tick(&self) -> FrameTick {
        FrameTick {
            vblank: f64_load(&self.shared.latest),
            period: self.period(),
            resumed: self.shared.resumed.swap(false, Ordering::AcqRel),
            periods: self.shared.periods.load(Ordering::Acquire),
        }
    }

    pub fn frame_handled(&self) {
        self.shared.pending.store(false, Ordering::Release);
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}

fn clock_thread(s: Arc<ClockShared>, hwnd: SendHwnd) {
    let mut monitor = s.monitor.load(Ordering::Relaxed);
    let hmon = |m: isize| HMONITOR(m as *mut core::ffi::c_void);
    let mut src = Vblank::for_monitor(hmon(monitor));
    let mut timeline = VsyncTimeline::new(f64_load(&s.period));
    let mut hz = s.nominal_hz.load(Ordering::Relaxed);
    let mut checked_at = clock::mono();
    let mut bad = 0u32;
    let mut timer_since = 0.0f64;
    let mut next_due = 0.0f64;
    let mut parked = true;

    loop {
        {
            let mut active = s.active.lock().unwrap();
            if !*active {
                parked = true;
            }
            while !*active && !s.stop.load(Ordering::Relaxed) {
                active = s.cv.wait(active).unwrap();
            }
        }
        if s.stop.load(Ordering::Relaxed) {
            break;
        }

        let mut recheck = s.display_changed.swap(false, Ordering::AcqRel)
            || timeline.off_rhythm() >= OFF_RHYTHM_LIMIT
            || clock::mono() - checked_at > 1.0;
        if parked {
            parked = false;
            recheck = true;
            timeline.reset();
            s.resumed.store(true, Ordering::Release);
        }

        let m = s.monitor.load(Ordering::Relaxed);
        if recheck || m != monitor {
            checked_at = clock::mono();
            let now_hz = refresh_hz(hmon(m));
            if m != monitor || now_hz != hz {
                monitor = m;
                hz = now_hz;
                s.nominal_hz.store(hz, Ordering::Relaxed);
                src = Vblank::for_monitor(hmon(m));
                bad = 0;
                timeline.set_nominal_period(period_for_hz(hz));
            } else if timeline.off_rhythm() >= OFF_RHYTHM_LIMIT {
                timeline.reset();
            }
        } else if matches!(src, Vblank::Timer) && clock::mono() - timer_since > 5.0 {
            src = Vblank::for_monitor(hmon(m));
            bad = 0;
        }

        let period = timeline.period();
        let t0 = clock::mono();
        let ok = src.wait(period, &mut next_due);
        let raw = clock::mono();

        if !ok || raw - t0 < 0.15 * period {
            bad += 1;
            if bad >= 3 && !matches!(src, Vblank::Timer) {
                src = Vblank::Timer;
                timer_since = raw;
            }
            if !ok || bad >= 3 {
                continue;
            }
        } else {
            bad = 0;
        }

        let (t, n) = timeline.lock(raw);
        if n == 0 {
            continue;
        }
        f64_store(&s.latest, t);
        f64_store(&s.period, timeline.period());

        if !s.pending.swap(true, Ordering::AcqRel) {
            s.periods.store(n.min(1000), Ordering::Release);
            unsafe {
                let _ = PostMessageW(hwnd.hwnd(), WM_APP_FRAME, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_info_geometry_math() {
        let mon = MonitorInfo {
            handle: 1,
            rect: RECT {
                left: 100,
                top: 0,
                right: 1920 + 100,
                bottom: 1080,
            },
            work: RECT {
                left: 100,
                top: 0,
                right: 1920 + 100,
                bottom: 1040,
            },
            dpi: 144,
        };
        assert_eq!(mon.scale(), 1.5);
        assert_eq!(mon.center_x(), 1060);
        assert_eq!(mon.top(), 0);
    }

    #[test]
    fn pick_primary_monitor_returns_valid_rect() {
        let mon = pick_primary_monitor();
        assert!(mon.rect.right > mon.rect.left);
        assert!(mon.rect.bottom > mon.rect.top);
        assert!(mon.dpi >= 96);
    }
}
