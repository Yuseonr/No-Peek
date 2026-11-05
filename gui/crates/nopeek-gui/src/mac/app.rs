//! Main AppKit application coordinator and event loop for macOS.
//! Adapted from `oyenza-studio/crates/poni/src/mac/app.rs`.

use crate::clock;
use crate::events::{AppEvent, EventSink};
use crate::mac::gfx::{scale_about, Gfx, TextStyle, Weight};
use crate::mac::menubar::{MenuBar, MenuCmd, TrayState};
use crate::mac::notch::{pick_screen, ScreenInfo};
use crate::mac::window;
use crate::services::ipc_worker::IpcWorker;
use nopeek_core::config::{EnvMode, GuiConfig};
use nopeek_core::geom::{Color, Point, Rect};
use nopeek_core::ipc::{IpcEvent, RiskLevel};
use nopeek_core::island::{fit_alert_width, Geom, Island, ICON_SIZE, ICON_TEXT_GAP, SHOULDER};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_core_graphics::CGContext;
use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub struct App {
    cfg: GuiConfig,
    island: Island,
    gfx: Gfx,
    worker: IpcWorker,
    menubar: MenuBar,
    screen: ScreenInfo,
    rx: Receiver<AppEvent>,
    sink: EventSink,
    tray_state: TrayState,
    last_vblank: f64,
}

pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|c| match c.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

/// Render the Dynamic Island into the active Core Graphics context.
pub fn draw_into(cg: &CGContext, dirty: Rect) {
    let _ = dirty;
    with_app(|app| {
        let mut canvas = app.gfx.canvas(cg);
        let g = app.island.current();
        let scale = app.screen.scale;
        let _ = scale;

        // Center island horizontally on the canvas
        let canvas_w = (app.cfg.alert_max_width + 2.0 * (SHOULDER + 16.0)).ceil();
        let body_x = (canvas_w - g.w) / 2.0;

        let border_color = match app.island.level {
            RiskLevel::Alert => Some(Color::rgba(1.0, 1.0, 1.0, 0.08)),
            RiskLevel::Escalated => Some(Color::rgba(1.0, 0.27, 0.23, 0.35)),
            _ => None,
        };

        // Draw island squircle notch
        let body = Rect::new(body_x, 0.0, g.w, g.h);
        canvas.notch(body, g.r, SHOULDER, Color::black(1.0), border_color);

        // Content pop animation (icon + alert text)
        let alpha = app.island.content_alpha();
        if alpha > 0.002 {
            let notch_h = app.screen.notch.map(|n| n.h).unwrap_or(0.0);
            let content_y = notch_h;
            let content_h = g.h - notch_h;
            let content_rect = Rect::new(body_x, content_y, g.w, content_h);

            let pop_scale = app.island.content_scale();
            let center = Point::new(
                content_rect.x + content_rect.w / 2.0,
                content_rect.y + content_rect.h / 2.0,
            );
            let xf = scale_about(center, pop_scale);

            let icon_x = content_rect.x + app.cfg.alert_margin_x;
            let icon_y = content_rect.y + (content_rect.h - ICON_SIZE) / 2.0;
            let icon_rect = Rect::new(icon_x, icon_y, ICON_SIZE, ICON_SIZE);

            let text_x = icon_x + ICON_SIZE + ICON_TEXT_GAP;
            let text_w = (content_rect.right() - app.cfg.alert_margin_x - text_x).max(1.0);
            let text_rect = Rect::new(text_x, content_rect.y, text_w, content_rect.h);

            let is_escalated = app.island.level == RiskLevel::Escalated;
            let (text_color, icon_color, weight) = if is_escalated {
                (
                    Color::rgba(1.0, 0.94, 0.94, 1.0),
                    Color::rgba(1.0, 0.27, 0.23, 1.0),
                    Weight::Bold,
                )
            } else {
                (
                    Color::rgba(0.96, 0.96, 0.96, 1.0),
                    Color::rgba(1.0, 0.62, 0.04, 1.0),
                    Weight::Semibold,
                )
            };

            let style = TextStyle::new(13.5).weight(weight).color(text_color);

            canvas.layered(content_rect, alpha, xf, |c| {
                c.icon(app.island.icon, icon_rect, icon_color);
                c.text(&app.island.text, text_rect, &style);
            });
        }
    });
}

/// CADisplayLink tick: step spring physics, re-render, and pause when settled.
pub fn on_frame_tick(timestamp: f64, target_timestamp: f64) {
    with_app(|app| {
        let now = target_timestamp;
        let dt = if app.last_vblank > 0.0 && now > app.last_vblank {
            ((now - app.last_vblank) as f32).clamp(0.001, 0.1)
        } else {
            1.0 / 60.0
        };
        app.last_vblank = now;

        app.island.step(dt, timestamp);
        window::redraw();

        // Zero idle CPU: park CADisplayLink when settled
        if app.island.settled() {
            window::set_display_link_active(false);
        }
    });
}

/// Drain queued IPC and tray events from the mpsc channel.
pub fn drain_events() {
    with_app(|app| {
        let mtm = match MainThreadMarker::new() {
            Some(m) => m,
            None => return,
        };

        while let Ok(event) = app.rx.try_recv() {
            match event {
                AppEvent::Ipc(ipc_ev) => match ipc_ev {
                    IpcEvent::Ready {
                        source, headless, ..
                    } => {
                        app.tray_state.mode_text = if headless {
                            format!("Source: {source} (Headless)")
                        } else {
                            format!("Source: {source} (Preview)")
                        };
                        app.tray_state.status_text = "Status: Monitoring (Safe)".into();
                        app.menubar.update_tooltip(mtm, "No-Peek: Monitoring");
                        app.menubar.refresh(mtm, &app.tray_state);
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
                        app.tray_state.auto_calibrate = auto_calibrate;
                        app.tray_state.paused = paused;

                        let mode_str = calibration_mode.as_deref().unwrap_or("AUTO");
                        let lock_str = if calibrated {
                            format!("Owner Locked - {mode_str}")
                        } else {
                            "Searching Owner...".into()
                        };

                        app.tray_state.status_text = match level {
                            RiskLevel::Monitoring | RiskLevel::Pending => {
                                format!("Status: Monitoring ({lock_str})")
                            }
                            RiskLevel::Alert => format!("Status: Alert! ({message})"),
                            RiskLevel::Escalated => format!("Status: DANGER! ({message})"),
                        };

                        let tip = match level {
                            RiskLevel::Monitoring | RiskLevel::Pending => "No-Peek: Monitoring",
                            RiskLevel::Alert => "No-Peek: ALERT (Peeker Detected)",
                            RiskLevel::Escalated => "No-Peek: DANGER (Screen Compromised)",
                        };
                        app.menubar.update_tooltip(mtm, tip);
                        app.menubar.refresh(mtm, &app.tray_state);

                        // Text measurement via Core Text
                        let is_escalated = level == RiskLevel::Escalated;
                        let (measured_w, _) = app.gfx.measure_text(&message, is_escalated);

                        // Geometry adaptation for physical notch vs flat screens
                        let now = clock::mono();
                        if let Some(notch) = app.screen.notch {
                            match level {
                                RiskLevel::Monitoring | RiskLevel::Pending => {
                                    app.island.set_target(
                                        Geom::new(notch.w, notch.h, 10.0),
                                        now,
                                        false,
                                    );
                                }
                                RiskLevel::Alert | RiskLevel::Escalated => {
                                    let target_w =
                                        fit_alert_width(measured_w, &app.cfg).max(notch.w + 24.0);
                                    let target_h = notch.h + app.cfg.alert_height;
                                    app.island
                                        .set_state(level, message, measured_w, now, &app.cfg);
                                    app.island.set_target(
                                        Geom::new(target_w, target_h, 20.0),
                                        now,
                                        true,
                                    );
                                }
                            }
                        } else {
                            app.island
                                .set_state(level, message, measured_w, now, &app.cfg);
                        }

                        if !app.island.settled() {
                            window::set_display_link_active(true);
                        }
                    }
                    IpcEvent::Calibration { message, .. } => {
                        app.tray_state.status_text = format!("Calibration: {message}");
                        app.menubar.refresh(mtm, &app.tray_state);
                    }
                    IpcEvent::Error { fatal, message } => {
                        eprintln!("[nopeek-gui] Worker error: fatal={fatal} | {message}");
                        app.menubar
                            .update_tooltip(mtm, &format!("No-Peek Error: {message}"));
                    }
                },
                AppEvent::TrayCommand(cmd) => {
                    let _ = app.worker.send_command(&cmd);
                }
                AppEvent::IpcExited { code } => {
                    eprintln!("[nopeek-gui] Python worker exited with code: {code:?}");
                }
                AppEvent::Quit => {
                    let ns_app = NSApplication::sharedApplication(mtm);
                    ns_app.terminate(None);
                }
            }
        }
    });
}

/// Menu Bar action callback.
pub fn on_menu(cmd: MenuCmd) {
    with_app(|app| match cmd {
        MenuCmd::Command(c) => app.sink.send(AppEvent::TrayCommand(c)),
        MenuCmd::Quit => app.sink.send(AppEvent::Quit),
    });
}

/// Run the native macOS Dynamic Island application.
pub fn run(cfg: GuiConfig) -> Result<i32, String> {
    let mtm = MainThreadMarker::new().ok_or("No-Peek GUI must run on the main thread")?;
    let ns_app = NSApplication::sharedApplication(mtm);

    // Accessory activation policy: No Dock icon, lives in Menu Bar only
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let screen = pick_screen(mtm).ok_or("Failed to query display screen geometry")?;
    let _scale = screen.scale;

    let canvas_w = (cfg.alert_max_width + 2.0 * (SHOULDER + 16.0)).ceil() as f64;
    let canvas_h =
        (screen.notch.map_or(0.0, |n| n.h as f64) + cfg.alert_height as f64 + 16.0).ceil();
    let left = screen.cx - canvas_w / 2.0;

    window::create(mtm);
    window::place(&screen, left, canvas_w, canvas_h);
    window::show();

    let (tx, rx) = mpsc::channel();
    let sink = EventSink::new(tx);

    let tray_state = TrayState {
        status_text: "Status: Initializing camera & models...".into(),
        mode_text: match cfg.env_mode {
            EnvMode::Dev => "Mode: Development (Preview)".into(),
            EnvMode::Prod => "Mode: Production (Headless)".into(),
        },
        auto_calibrate: true,
        paused: false,
    };

    let menubar = MenuBar::add(mtm, &tray_state);

    let worker = IpcWorker::spawn(&cfg, sink.clone())
        .map_err(|e| format!("Failed to spawn Python inference worker: {e}"))?;

    let island = Island::new(&cfg);
    let gfx = Gfx::new();

    let app_inst = App {
        cfg,
        island,
        gfx,
        worker,
        menubar,
        screen,
        rx,
        sink,
        tray_state,
        last_vblank: 0.0,
    };

    APP.with(|c| *c.borrow_mut() = Some(app_inst));

    // Initial frame draw
    window::redraw();

    // Run the native AppKit event loop
    ns_app.run();

    Ok(0)
}
