use nopeek_core::controls::{
    panel_layout, panel_width, MenuAction, TrayState, PANEL_HEIGHT, PANEL_WIDTH, PREVIEW_RECT,
};
use nopeek_core::ipc::IpcCommand;
use nopeek_core::preview::PreviewFrame;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSButtonType, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSImage, NSImageScaling, NSImageView, NSPanel, NSTextField,
    NSView, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use std::cell::RefCell;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NoPeekSettingsTarget"]
    pub struct SettingsTarget;
    unsafe impl NSObjectProtocol for SettingsTarget {}
    unsafe impl NSWindowDelegate for SettingsTarget {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            crate::mac::app::on_menu(MenuAction::Command(IpcCommand::SetPreview { enabled: false }));
        }
    }
    impl SettingsTarget {
        #[unsafe(method(change:))]
        fn change(&self, sender: &NSButton) {
            let action = PANEL.with(|cell| cell.borrow().as_ref().and_then(|panel| panel.actions.get(sender.tag() as usize).copied()));
            if let Some(action) = action {
                crate::mac::app::on_menu(action);
            }
        }
    }
);

struct SettingsPanel {
    window: Retained<NSPanel>,
    target: Retained<SettingsTarget>,
    actions: Vec<MenuAction>,
    last_state: Option<TrayState>,
    widgets: Vec<Widget>,
    preview: Option<Retained<NSImageView>>,
    preview_label: Option<Retained<NSTextField>>,
}

enum Widget {
    Label(Retained<NSTextField>),
    Button(Retained<NSButton>),
}

thread_local! {
    static PANEL: RefCell<Option<SettingsPanel>> = const { RefCell::new(None) };
}

fn create(mtm: MainThreadMarker) -> SettingsPanel {
    let rect = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
    );
    let window = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect,
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::UtilityWindow,
        NSBackingStoreType::Buffered,
        false,
    );
    window.setTitle(&NSString::from_str("No-Peek / Pengaturan"));
    window.setFloatingPanel(true);
    window.setHidesOnDeactivate(false);
    unsafe { window.setReleasedWhenClosed(false) };
    window.center();
    let target: Retained<SettingsTarget> = unsafe { msg_send![SettingsTarget::alloc(mtm), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    SettingsPanel {
        window,
        target,
        actions: Vec::new(),
        last_state: None,
        widgets: Vec::new(),
        preview: None,
        preview_label: None,
    }
}

fn render(panel: &mut SettingsPanel, mtm: MainThreadMarker, state: &TrayState) {
    panel
        .window
        .setContentSize(NSSize::new(panel_width(state.preview), PANEL_HEIGHT));
    if !panel.widgets.is_empty() {
        panel.actions.clear();
        for (widget, control) in panel.widgets.iter().zip(panel_layout(state)) {
            match widget {
                Widget::Label(label) => label.setStringValue(&NSString::from_str(&control.label)),
                Widget::Button(button) => {
                    button.setTitle(&NSString::from_str(&control.label));
                    button.setEnabled(control.enabled);
                    button.setState(if control.checked {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    if let Some(action) = control.action {
                        button.setTag(panel.actions.len() as isize);
                        panel.actions.push(action);
                    }
                }
            }
        }
        panel.last_state = Some(state.clone());
        sync_preview(panel, state.preview);
        return;
    }
    let content = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(panel_width(state.preview), PANEL_HEIGHT),
        ),
    );
    panel.actions.clear();
    for control in panel_layout(state) {
        let [x, y, w, h] = control.rect;
        let rect = NSRect::new(NSPoint::new(x, PANEL_HEIGHT - y - h), NSSize::new(w, h));
        if let Some(action) = control.action {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&control.label),
                    Some(panel.target.as_ref()),
                    Some(sel!(change:)),
                    mtm,
                )
            };
            button.setFrame(rect);
            button.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            if control.checkbox {
                button.setButtonType(NSButtonType::Switch);
            } else if matches!(
                action,
                MenuAction::Command(
                    nopeek_core::ipc::IpcCommand::SetConfidence { .. }
                        | nopeek_core::ipc::IpcCommand::SetAlertDelay { .. }
                )
            ) {
                button.setButtonType(NSButtonType::PushOnPushOff);
            }
            button.setState(if control.checked {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            button.setEnabled(control.enabled);
            button.setTag(panel.actions.len() as isize);
            panel.actions.push(action);
            content.addSubview(&button);
            panel.widgets.push(Widget::Button(button));
        } else {
            let label = NSTextField::labelWithString(&NSString::from_str(&control.label), mtm);
            label.setFrame(rect);
            label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            label.setMaximumNumberOfLines(2);
            content.addSubview(&label);
            panel.widgets.push(Widget::Label(label));
        }
    }
    let [x, y, w, h] = PREVIEW_RECT;
    let preview = NSImageView::initWithFrame(
        NSImageView::alloc(mtm),
        NSRect::new(NSPoint::new(x, PANEL_HEIGHT - y - h), NSSize::new(w, h)),
    );
    preview.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    content.addSubview(&preview);
    let label = NSTextField::labelWithString(&NSString::from_str("Preview kamera / Lokal"), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(x, PANEL_HEIGHT - 52.0),
        NSSize::new(w, 24.0),
    ));
    label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    content.addSubview(&label);
    panel.preview = Some(preview);
    panel.preview_label = Some(label);
    panel.window.setContentView(Some(&content));
    panel.last_state = Some(state.clone());
    sync_preview(panel, state.preview);
}

fn sync_preview(panel: &SettingsPanel, enabled: bool) {
    if let Some(preview) = &panel.preview {
        preview.setHidden(!enabled);
        if !enabled {
            preview.setImage(None);
        }
    }
    if let Some(label) = &panel.preview_label {
        label.setHidden(!enabled);
    }
}

pub fn set_frame(frame: PreviewFrame) -> Result<(), String> {
    PANEL.with(|cell| {
        if let Some(panel) = cell.borrow().as_ref() {
            if panel.window.isVisible()
                && panel
                    .last_state
                    .as_ref()
                    .is_some_and(|state| state.preview && !state.paused && state.ready)
            {
                let bitmap = crate::mac::gfx::Bitmap::from_preview(&frame)?;
                let image = bitmap
                    .image()
                    .ok_or_else(|| "Could not create preview image".to_string())?;
                use objc2::AllocAnyThread;
                let native = NSImage::initWithCGImage_size(
                    NSImage::alloc(),
                    &image,
                    NSSize::new(frame.width as f64, frame.height as f64),
                );
                if let Some(view) = &panel.preview {
                    view.setImage(Some(&native));
                }
            }
        }
        Ok(())
    })
}

pub fn show(mtm: MainThreadMarker, state: &TrayState) {
    PANEL.with(|cell| {
        let mut slot = cell.borrow_mut();
        let panel = slot.get_or_insert_with(|| create(mtm));
        render(panel, mtm, state);
        NSApplication::sharedApplication(mtm).activate();
        panel.window.makeKeyAndOrderFront(None);
    });
}

pub fn refresh(mtm: MainThreadMarker, state: &TrayState) {
    PANEL.with(|cell| {
        if let Some(panel) = cell.borrow_mut().as_mut() {
            if panel.window.isVisible() && panel.last_state.as_ref() != Some(state) {
                render(panel, mtm, state);
            }
        }
    });
}
