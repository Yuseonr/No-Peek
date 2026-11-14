use crate::events::{AppEvent, EventSink};
use nopeek_core::controls::{
    panel_layout, panel_width, MenuAction, TrayState, PANEL_HEIGHT, PANEL_WIDTH, PREVIEW_RECT,
};
use nopeek_core::ipc::IpcCommand;
use nopeek_core::preview::PreviewFrame;
use std::cell::RefCell;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetStockObject, InvalidateRect, StretchDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLACK_BRUSH, COLOR_WINDOW, DEFAULT_GUI_FONT, DIB_RGB_COLORS, HBRUSH,
    PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("NoPeek.Settings");
const PREVIEW_CLASS: PCWSTR = w!("NoPeek.SettingsPreview");

struct SettingsPanel {
    hwnd: HWND,
    children: Vec<HWND>,
    actions: Vec<Option<MenuAction>>,
    sink: EventSink,
    preview: HWND,
    frame: Option<PreviewFrame>,
    preview_enabled: bool,
    last_state: Option<TrayState>,
    last_dpi: u32,
}

fn stop_preview() {
    let sink = PANEL.with(|cell| cell.borrow().as_ref().map(|panel| panel.sink.clone()));
    if let Some(sink) = sink {
        sink.send(AppEvent::TrayCommand(IpcCommand::SetPreview {
            enabled: false,
        }));
    }
}

unsafe extern "system" fn preview_wndproc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    if message != WM_PAINT {
        return DefWindowProcW(hwnd, message, wp, lp);
    }
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut paint);
    if dc.is_invalid() {
        eprintln!("[nopeek-gui] Could not begin preview paint");
        return LRESULT(0);
    }
    let result = (|| -> Result<()> {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect)?;
        if FillRect(dc, &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0)) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        PANEL.with(|cell| {
            if let Some(frame) = cell
                .borrow()
                .as_ref()
                .and_then(|panel| panel.frame.as_ref())
            {
                let scale = ((rect.right - rect.left) as f64 / frame.width as f64)
                    .min((rect.bottom - rect.top) as f64 / frame.height as f64);
                let width = (frame.width as f64 * scale) as i32;
                let height = (frame.height as f64 * scale) as i32;
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: frame.width as i32,
                        biHeight: -(frame.height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let rows = StretchDIBits(
                    dc,
                    (rect.right - width) / 2,
                    (rect.bottom - height) / 2,
                    width,
                    height,
                    0,
                    0,
                    frame.width as i32,
                    frame.height as i32,
                    Some(frame.bgra.as_ptr().cast()),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if rows == 0 || rows == -1 {
                    return Err(windows::core::Error::from_win32());
                }
            }
            Ok(())
        })
    })();
    if let Err(error) = result {
        eprintln!("[nopeek-gui] Preview paint failed: {error}");
    }
    if !EndPaint(hwnd, &paint).as_bool() {
        eprintln!("[nopeek-gui] Could not finish preview paint");
    }
    LRESULT(0)
}

impl Drop for SettingsPanel {
    fn drop(&mut self) {
        if let Err(error) = unsafe { DestroyWindow(self.hwnd) } {
            eprintln!("[nopeek-gui] Failed to release settings window: {error}");
        }
    }
}

thread_local! {
    static PANEL: RefCell<Option<SettingsPanel>> = const { RefCell::new(None) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match message {
        WM_CLOSE => {
            stop_preview();
            let _was_visible = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wp.0 & 0xffff;
            if id == 2 {
                stop_preview();
                let _was_visible = ShowWindow(hwnd, SW_HIDE);
            } else if wp.0 >> 16 == BN_CLICKED as usize {
                let selected = PANEL.with(|cell| {
                    let panel = cell.borrow();
                    panel.as_ref().and_then(|panel| {
                        id.checked_sub(100)
                            .and_then(|index| panel.actions.get(index).copied().flatten())
                            .map(|action| (panel.sink.clone(), action))
                    })
                });
                if let Some((sink, action)) = selected {
                    match action {
                        MenuAction::Command(command) => sink.send(AppEvent::TrayCommand(command)),
                        MenuAction::Quit => sink.send(AppEvent::Quit),
                        MenuAction::OpenSettings => sink.send(AppEvent::OpenSettings),
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}

fn create(owner: HWND, state: &TrayState, sink: EventSink) -> Result<SettingsPanel> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(windows::core::Error::from_win32());
        }
        let preview_class = WNDCLASSEXW {
            lpfnWndProc: Some(preview_wndproc),
            lpszClassName: PREVIEW_CLASS,
            ..class
        };
        if RegisterClassExW(&preview_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(windows::core::Error::from_win32());
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_CONTROLPARENT;
        let scale = GetDpiForWindow(owner) as f64 / 96.0;
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: (PANEL_WIDTH * scale) as i32,
            bottom: (PANEL_HEIGHT * scale) as i32,
        };
        AdjustWindowRectEx(&mut bounds, style, false, ex_style)?;
        let hwnd = CreateWindowExW(
            ex_style,
            CLASS,
            w!("No-Peek / Pengaturan"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            owner,
            None,
            instance,
            None,
        )?;
        let mut panel = SettingsPanel {
            hwnd,
            children: Vec::new(),
            actions: Vec::new(),
            sink,
            preview: HWND::default(),
            frame: None,
            preview_enabled: false,
            last_state: None,
            last_dpi: 0,
        };
        for (index, control) in panel_layout(state).iter().enumerate() {
            let label = wide(&control.label);
            let class = if control.action.is_some() {
                w!("BUTTON")
            } else {
                w!("STATIC")
            };
            let button_style = if control.checkbox {
                BS_CHECKBOX
            } else if matches!(
                control.action,
                Some(MenuAction::Command(
                    nopeek_core::ipc::IpcCommand::SetConfidence { .. }
                        | nopeek_core::ipc::IpcCommand::SetAlertDelay { .. }
                ))
            ) {
                BS_CHECKBOX | BS_PUSHLIKE
            } else {
                BS_PUSHBUTTON
            };
            let style = WS_CHILD
                | WS_VISIBLE
                | if control.action.is_some() {
                    WS_TABSTOP | WINDOW_STYLE(button_style as u32)
                } else {
                    WINDOW_STYLE::default()
                };
            let child = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR(label.as_ptr()),
                style,
                0,
                0,
                1,
                1,
                hwnd,
                HMENU((100 + index) as *mut _),
                instance,
                None,
            )?;
            panel.children.push(child);
            panel.actions.push(control.action);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let _previous_font =
                SendMessageW(child, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
        panel.preview = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PREVIEW_CLASS,
            w!("Preview kamera / Lokal"),
            WS_CHILD,
            0,
            0,
            1,
            1,
            hwnd,
            None,
            instance,
            None,
        )?;
        update(&mut panel, state)?;
        Ok(panel)
    }
}

fn update(panel: &mut SettingsPanel, state: &TrayState) -> Result<()> {
    let dpi = unsafe { GetDpiForWindow(panel.hwnd) };
    if panel.last_state.as_ref() == Some(state) && panel.last_dpi == dpi {
        return Ok(());
    }
    let scale = dpi as f64 / 96.0;
    unsafe {
        let mut current = RECT::default();
        GetClientRect(panel.hwnd, &mut current)?;
        let target_width = (panel_width(state.preview) * scale) as i32;
        if current.right != target_width {
            let mut bounds = RECT {
                left: 0,
                top: 0,
                right: target_width,
                bottom: (PANEL_HEIGHT * scale) as i32,
            };
            AdjustWindowRectEx(
                &mut bounds,
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
                false,
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_CONTROLPARENT,
            )?;
            SetWindowPos(
                panel.hwnd,
                None,
                0,
                0,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
        }
        let [x, y, w, h] = PREVIEW_RECT;
        SetWindowPos(
            panel.preview,
            None,
            (x * scale) as i32,
            (y * scale) as i32,
            (w * scale) as i32,
            (h * scale) as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        let _was_visible = ShowWindow(
            panel.preview,
            if state.preview { SW_SHOWNA } else { SW_HIDE },
        );
    }
    panel.preview_enabled = state.preview && !state.paused && state.ready;
    if !panel.preview_enabled {
        panel.frame = None;
    }
    for (child, control) in panel.children.iter().zip(panel_layout(state)) {
        let [x, y, w, h] = control.rect;
        let label = wide(&control.label);
        unsafe {
            SetWindowTextW(*child, PCWSTR(label.as_ptr()))?;
            SetWindowPos(
                *child,
                None,
                (x * scale) as i32,
                (y * scale) as i32,
                (w * scale) as i32,
                (h * scale) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
            let _was_disabled = EnableWindow(*child, control.enabled);
            if control.action.is_some() {
                let _previous_state = SendMessageW(
                    *child,
                    BM_SETCHECK,
                    WPARAM(usize::from(control.checked)),
                    LPARAM(0),
                );
            }
        }
    }
    panel.last_state = Some(state.clone());
    panel.last_dpi = dpi;
    Ok(())
}

pub fn show(owner: HWND, state: &TrayState, sink: EventSink) -> Result<()> {
    PANEL.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(create(owner, state, sink)?);
        }
        if let Some(panel) = slot.as_mut() {
            update(panel, state)?;
            unsafe {
                let _was_visible = ShowWindow(panel.hwnd, SW_SHOW);
                if !SetForegroundWindow(panel.hwnd).as_bool() {
                    eprintln!("[nopeek-gui] Windows declined settings foreground activation");
                }
            }
        }
        Ok(())
    })
}

pub fn refresh(state: &TrayState) -> Result<()> {
    PANEL.with(|cell| {
        if let Some(panel) = cell.borrow_mut().as_mut() {
            update(panel, state)?;
        }
        Ok(())
    })
}

pub fn set_frame(frame: PreviewFrame) -> std::result::Result<(), String> {
    frame.validate()?;
    PANEL.with(|cell| {
        if let Some(panel) = cell.borrow_mut().as_mut() {
            if panel.preview_enabled && unsafe { IsWindowVisible(panel.hwnd).as_bool() } {
                panel.frame = Some(frame);
                if !unsafe { InvalidateRect(panel.preview, None, false).as_bool() } {
                    return Err(format!(
                        "Could not invalidate preview: {}",
                        windows::core::Error::from_win32()
                    ));
                }
            }
        }
        Ok(())
    })
}

pub fn translate_dialog_message(message: &MSG) -> bool {
    PANEL.with(|cell| {
        cell.borrow().as_ref().is_some_and(|panel| unsafe {
            IsWindowVisible(panel.hwnd).as_bool() && IsDialogMessageW(panel.hwnd, message).as_bool()
        })
    })
}

pub fn close() {
    let panel = PANEL.with(|cell| cell.borrow_mut().take());
    drop(panel);
}
