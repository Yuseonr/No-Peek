//! Layered borderless NSPanel window for macOS Dynamic Island.
//! Adapted from `oyenza-studio/crates/poni/src/mac/window.rs`.

use crate::mac::notch::ScreenInfo;
use nopeek_core::geom::Rect;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSGraphicsContext, NSPanel,
    NSResponder, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize,
};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use std::cell::RefCell;

/// Above the menu bar (24) and status items, below pop-up menus.
const ISLAND_LEVEL: isize = 24 + 3;

define_class!(
    /// The Dynamic Island's panel: borderless, non-activating, click-through.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NoPeekIslandPanel"]
    pub struct IslandPanel;

    unsafe impl NSObjectProtocol for IslandPanel {}

    impl IslandPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }
    }
);

define_class!(
    /// View drawing the Dynamic Island with flipped (y-down) coordinates.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NoPeekIslandView"]
    pub struct IslandView;

    unsafe impl NSObjectProtocol for IslandView {}

    impl IslandView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            if let Some(ctx) = NSGraphicsContext::currentContext() {
                let cg = ctx.CGContext();
                let r = Rect::new(
                    dirty.origin.x as f32,
                    dirty.origin.y as f32,
                    dirty.size.width as f32,
                    dirty.size.height as f32,
                );
                crate::mac::app::draw_into(&cg, r);
            }
        }

        /// CADisplayLink vblank tick.
        #[unsafe(method(step:))]
        fn step(&self, link: &CADisplayLink) {
            crate::mac::app::on_frame_tick(link.timestamp(), link.targetTimestamp());
        }
    }
);

pub struct IslandWindow {
    pub panel: Retained<IslandPanel>,
    pub view: Retained<IslandView>,
    pub link: Option<Retained<CADisplayLink>>,
    pub frame: NSRect,
}

thread_local! {
    static WINDOW: RefCell<Option<IslandWindow>> = const { RefCell::new(None) };
}

pub fn with_window<R>(f: impl FnOnce(&mut IslandWindow) -> R) -> Option<R> {
    WINDOW.with(|w| w.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// Create the NSPanel, NSView, and CADisplayLink.
pub fn create(mtm: MainThreadMarker) {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(500.0, 100.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<IslandPanel> = unsafe {
        msg_send![
            IslandPanel::alloc(mtm),
            initWithContentRect: rect,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };

    panel.setLevel(ISLAND_LEVEL);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setMovable(false);
    panel.setIgnoresMouseEvents(true);
    unsafe { panel.setReleasedWhenClosed(false) };

    let view: Retained<IslandView> =
        unsafe { msg_send![IslandView::alloc(mtm), initWithFrame: rect] };
    view.setWantsLayer(true);
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    panel.setContentView(Some(&view));

    // CADisplayLink drives frames at native refresh rate (up to 120Hz ProMotion).
    let has_link: bool =
        unsafe { msg_send![&*view, respondsToSelector: sel!(displayLinkWithTarget:selector:)] };
    let link = has_link.then(|| {
        let target: &AnyObject = view.as_ref();
        let l = unsafe { view.displayLinkWithTarget_selector(target, sel!(step:)) };
        l.setPreferredFrameRateRange(CAFrameRateRange {
            minimum: 60.0,
            maximum: 120.0,
            preferred: 120.0,
        });
        unsafe { l.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        l.setPaused(true);
        l
    });

    let frame = panel.frame();
    WINDOW.with(|w| {
        *w.borrow_mut() = Some(IslandWindow {
            panel,
            view,
            link,
            frame,
        });
    });
}

/// Place the panel hanging from the top edge of `screen`.
pub fn place(screen: &ScreenInfo, left: f64, w: f64, h: f64) {
    with_window(|win| {
        let x = (screen.x + left).round();
        let y = screen.y + screen.h - h;
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        win.panel.setFrame_display(frame, false);
        win.frame = frame;
    });
}

pub fn show() {
    with_window(|win| win.panel.orderFrontRegardless());
}

pub fn redraw() {
    with_window(|win| win.view.setNeedsDisplay(true));
}

/// Control CADisplayLink execution for the Zero Idle CPU invariant.
pub fn set_display_link_active(active: bool) {
    with_window(|win| {
        if let Some(ref l) = win.link {
            l.setPaused(!active);
        }
    });
}
