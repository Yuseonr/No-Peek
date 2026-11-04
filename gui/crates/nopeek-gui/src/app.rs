//! Main application coordinator, event loop, and Win32 message handling.
//! Adapted from `oyenza-studio/crates/poni/src/app.rs`.

use crate::clock;
use crate::events::{AppEvent, EventSink, SendHwnd, WM_APP_EVENT, WM_APP_FRAME, WM_APP_TRAY};
use crate::gfx::Gfx;
use crate::services::ipc_worker::IpcWorker;
use crate::tray::{show_context_menu, Tray, TrayAction, TrayState};
use crate::window::{
    create_helper_window, create_island_window, pick_primary_monitor, register_classes, FrameClock,
    MonitorInfo,
};
use nopeek_core::config::{EnvMode, GuiConfig};
use nopeek_core::ipc::{IpcEvent, RiskLevel};
use nopeek_core::island::{Island, CANVAS_MARGIN, SHOULDER};
use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver};
use windows::core::Result;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub struct App {
    hwnd: HWND,
    helper_hwnd: HWND,
    cfg: GuiConfig,
    island: Island,
    gfx: Gfx,
    clock: FrameClock,
    worker: IpcWorker,
    tray: Option<Tray>,
    tray_state: TrayState,
    monitor: MonitorInfo,
    rx: Receiver<AppEvent>,
    sink: EventSink,
}

impl App {
    pub fn new(
        hwnd: HWND,
        helper_hwnd: HWND,
        sink: EventSink,
        rx: Receiver<AppEvent>,
        cfg: GuiConfig,
    ) -> Result<Self> {
        let monitor = pick_primary_monitor();
        let scale = monitor.scale();

        // Calculate fixed canvas size centered at top edge
        let canvas_w =
            ((cfg.alert_max_width + 2.0 * (SHOULDER + CANVAS_MARGIN)) * scale).ceil() as i32;
        let canvas_h = ((cfg.alert_height + CANVAS_MARGIN) * scale).ceil() as i32;
        let x = monitor.center_x() - canvas_w / 2;
        let y = monitor.top();

        eprintln!("[nopeek-gui] Step 1: Monitor picked: scale={scale}, top={y}, x={x}, w={canvas_w}, h={canvas_h}");
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                canvas_w,
                canvas_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }

        eprintln!("[nopeek-gui] Step 2: Initializing Gfx...");
        let gfx = Gfx::new(hwnd, canvas_w as u32, canvas_h as u32).map_err(|e| {
            eprintln!("[nopeek-gui] Gfx::new failed: {e:?}");
            e
        })?;

        eprintln!("[nopeek-gui] Step 3: Starting FrameClock...");
        let clock = FrameClock::start(SendHwnd::new(hwnd), monitor.handle);
        let island = Island::new(&cfg);

        let tray_state = TrayState {
            status_text: "Status: Initializing camera & models...".into(),
            mode_text: match cfg.env_mode {
                EnvMode::Dev => "Mode: Development (Preview)".into(),
                EnvMode::Prod => "Mode: Production (Headless)".into(),
            },
            auto_calibrate: true,
            paused: false,
        };

        eprintln!("[nopeek-gui] Step 4: Adding System Tray icon...");
        let tray = match Tray::add(helper_hwnd, "No-Peek Dynamic Island") {
            Ok(t) => {
                eprintln!("[nopeek-gui] System tray icon registered.");
                Some(t)
            }
            Err(e) => {
                eprintln!("[nopeek-gui] Warning: Shell_TrayWnd not available or Tray::add failed: {e:?}. Continuing without tray icon.");
                None
            }
        };

        eprintln!("[nopeek-gui] Step 5: Spawning IPC worker...");
        let worker = IpcWorker::spawn(&cfg, sink.clone()).map_err(|e| {
            eprintln!("[nopeek-gui] IpcWorker::spawn failed: {e:?}");
            windows::core::Error::new(windows::core::HRESULT(-1), e.to_string())
        })?;
        eprintln!("[nopeek-gui] Step 6: App::new completed successfully!");

        Ok(Self {
            hwnd,
            helper_hwnd,
            cfg,
            island,
            gfx,
            clock,
            worker,
            tray,
            tray_state,
            monitor,
            rx,
            sink,
        })
    }

    fn update_tray_tooltip(&mut self, tip: &str) {
        if let Some(ref mut tray) = self.tray {
            tray.update_tooltip(tip);
        }
    }

    pub fn start(&mut self) {
        // Initial frame draw and immediately park the clock at 0% CPU
        self.gfx.render(&self.island, self.monitor.scale());
        self.clock.set_active(false);
    }

    pub fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::Ipc(ipc_ev) => match ipc_ev {
                    IpcEvent::Ready {
                        source, headless, ..
                    } => {
                        self.tray_state.mode_text = if headless {
                            format!("Source: {source} (Headless)")
                        } else {
                            format!("Source: {source} (Preview)")
                        };
                        self.tray_state.status_text = "Status: Monitoring (Safe)".into();
                        self.update_tray_tooltip("No-Peek: Monitoring");
                    }
                    IpcEvent::StateUpdate {
                        level,
                        message,
                        auto_calibrate,
                        calibrated,
                        calibration_mode,
                        paused,
                        ..
                    } => {
                        self.tray_state.auto_calibrate = auto_calibrate;
                        self.tray_state.paused = paused;

                        let mode_str = calibration_mode.as_deref().unwrap_or("AUTO");
                        let lock_str = if calibrated {
                            format!("Owner Locked - {mode_str}")
                        } else {
                            "Searching Owner...".into()
                        };

                        self.tray_state.status_text = match level {
                            RiskLevel::Monitoring | RiskLevel::Pending => {
                                format!("Status: Monitoring ({lock_str})")
                            }
                            RiskLevel::Alert => {
                                format!("Status: Alert! ({message})")
                            }
                            RiskLevel::Escalated => {
                                format!("Status: DANGER! ({message})")
                            }
                        };

                        let tip = match level {
                            RiskLevel::Monitoring | RiskLevel::Pending => "No-Peek: Monitoring",
                            RiskLevel::Alert => "No-Peek: ALERT (Peeker Detected)",
                            RiskLevel::Escalated => "No-Peek: DANGER (Screen Compromised)",
                        };
                        self.update_tray_tooltip(tip);

                        let (measured_w, _) = self
                            .gfx
                            .measure_text(&message, level == RiskLevel::Escalated);
                        self.island
                            .set_state(level, message, measured_w, clock::mono(), &self.cfg);

                        if !self.island.settled() {
                            self.clock.set_active(true);
                        }
                    }
                    IpcEvent::Calibration { message, .. } => {
                        self.tray_state.status_text = format!("Calibration: {message}");
                    }
                    IpcEvent::Error { fatal, message } => {
                        eprintln!("[nopeek-gui] Worker error: fatal={fatal} | {message}");
                        self.update_tray_tooltip(&format!("No-Peek Error: {message}"));
                    }
                },
                AppEvent::TrayCommand(cmd) => {
                    let _ = self.worker.send_command(&cmd);
                }
                AppEvent::IpcExited { code } => {
                    eprintln!("[nopeek-gui] Python worker exited with code: {code:?}");
                }
                AppEvent::Quit => unsafe {
                    PostQuitMessage(0);
                },
            }
        }
    }

    pub fn handle_frame(&mut self) {
        let tick = self.clock.tick();
        let _ = tick.resumed;
        let dt = (tick.period * tick.periods as f64) as f32;
        self.island.step(dt, tick.vblank);
        self.gfx.render(&self.island, self.monitor.scale());

        // Zero-idle invariant: when the island settles back to idle thickness, park VBlank clock
        if self.island.settled() {
            self.clock.set_active(false);
        }
        self.clock.frame_handled();
    }

    pub fn handle_tray(&mut self, lparam: LPARAM) {
        let msg = (lparam.0 & 0xffff) as u32;
        if msg == WM_RBUTTONUP || msg == WM_CONTEXTMENU {
            let mut pt = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            if let Some(action) = show_context_menu(self.helper_hwnd, pt, &self.tray_state) {
                match action {
                    TrayAction::Command(cmd) => {
                        self.sink.send(AppEvent::TrayCommand(cmd));
                    }
                    TrayAction::Quit => {
                        self.sink.send(AppEvent::Quit);
                    }
                }
            }
        }
    }

    pub fn shutdown(mut self) -> i32 {
        if let Some(tray) = self.tray.take() {
            tray.remove();
        }
        0
    }
}

pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|c| match c.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

unsafe extern "system" fn island_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_FRAME => {
            with_app(|app| app.handle_frame());
            LRESULT(0)
        }
        WM_APP_EVENT => {
            with_app(|app| app.handle_events());
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // Click-through: whole window is transparent to mouse clicks
            LRESULT(HTTRANSPARENT as isize)
        }
        WM_DPICHANGED => {
            with_app(|app| {
                let mon = pick_primary_monitor();
                app.clock.set_monitor(mon.handle);
                app.monitor = mon;
                let scale = mon.scale();
                let canvas_w = ((app.cfg.alert_max_width + 2.0 * (SHOULDER + CANVAS_MARGIN))
                    * scale)
                    .ceil() as i32;
                let canvas_h = ((app.cfg.alert_height + CANVAS_MARGIN) * scale).ceil() as i32;
                let x = mon.center_x() - canvas_w / 2;
                let y = mon.top();
                unsafe {
                    let _ = SetWindowPos(
                        app.hwnd,
                        HWND_TOPMOST,
                        x,
                        y,
                        canvas_w,
                        canvas_h,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn helper_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            with_app(|app| app.handle_tray(lp));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub fn run(cfg: GuiConfig) -> Result<i32> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    register_classes(island_wndproc, helper_wndproc)?;
    let hwnd = create_island_window()?;
    let helper_hwnd = create_helper_window()?;

    let (tx, rx) = mpsc::channel();
    let sink = EventSink::new(tx, SendHwnd::new(hwnd));

    let app = App::new(hwnd, helper_hwnd, sink, rx, cfg)?;
    APP.with(|c| *c.borrow_mut() = Some(app));
    with_app(|app| app.start());

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let code = APP.with(|c| c.borrow_mut().take().map(|a| a.shutdown()).unwrap_or(0));
    Ok(code)
}
