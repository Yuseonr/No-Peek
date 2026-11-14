//! macOS Menu Bar item (NSStatusBar / NSStatusItem) and context menu.
//! Adapted from `oyenza-studio/crates/poni/src/mac/menubar.rs`.

use crate::mac::gfx::Bitmap;
use nopeek_core::icons::{self, Icon};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, sel, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSObject, NSObjectProtocol, NSSize, NSString};
use std::cell::RefCell;

use nopeek_core::controls::{menu, MenuEntry};
pub use nopeek_core::controls::{MenuAction as MenuCmd, TrayState};

define_class!(
    /// Target handling Menu Bar item clicks.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NoPeekMenuTarget"]
    pub struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            let cmd = CMDS.with(|c| c.borrow().get(sender.tag() as usize).copied());
            if let Some(cmd) = cmd {
                crate::mac::app::on_menu(cmd);
            }
        }
    }
);

thread_local! {
    static CMDS: RefCell<Vec<MenuCmd>> = const { RefCell::new(Vec::new()) };
}

pub struct MenuBar {
    item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    pub menu: Retained<NSMenu>,
}

impl MenuBar {
    pub fn add(mtm: MainThreadMarker, state: &TrayState) -> Self {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            if let Some(img) = mark_image() {
                button.setImage(Some(&img));
            } else {
                button.setTitle(&NSString::from_str("No-Peek"));
            }
            button.setToolTip(Some(&NSString::from_str("No-Peek: Dynamic Island")));
        }
        let target: Retained<MenuTarget> =
            unsafe { objc2::msg_send![MenuTarget::alloc(mtm), init] };
        let items = menu(state);
        let menu = build(mtm, &target, &items);
        item.setMenu(Some(&menu));
        Self { item, target, menu }
    }

    pub fn refresh(&mut self, mtm: MainThreadMarker, state: &TrayState) {
        let items = menu(state);
        self.menu = build(mtm, &self.target, &items);
        self.item.setMenu(Some(&self.menu));
    }

    pub fn update_tooltip(&self, mtm: MainThreadMarker, tooltip: &str) {
        if let Some(button) = self.item.button(mtm) {
            button.setToolTip(Some(&NSString::from_str(tooltip)));
        }
    }
}

fn build(mtm: MainThreadMarker, target: &MenuTarget, items: &[MenuEntry]) -> Retained<NSMenu> {
    let mut cmds = Vec::new();
    let menu = build_level(mtm, target, items, &mut cmds);
    CMDS.with(|c| *c.borrow_mut() = cmds);
    menu
}

fn build_level(
    mtm: MainThreadMarker,
    target: &MenuTarget,
    items: &[MenuEntry],
    cmds: &mut Vec<MenuCmd>,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
    menu.setAutoenablesItems(false);
    for entry in items {
        if matches!(entry, MenuEntry::Separator) {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let label = match entry {
            MenuEntry::Info(label)
            | MenuEntry::Action { label, .. }
            | MenuEntry::Submenu { label, .. } => label,
            MenuEntry::Separator => unreachable!("separator handled above"),
        };
        let action = if matches!(entry, MenuEntry::Action { .. }) {
            Some(sel!(pick:))
        } else {
            None
        };
        let mi = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                action,
                &NSString::from_str(""),
            )
        };
        match entry {
            MenuEntry::Action {
                action,
                checked,
                enabled,
                ..
            } => {
                let t: &AnyObject = target.as_ref();
                unsafe { mi.setTarget(Some(t)) };
                mi.setTag(cmds.len() as isize);
                mi.setEnabled(*enabled);
                mi.setState(if *checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                cmds.push(*action);
            }
            MenuEntry::Submenu { items, .. } => {
                let child = build_level(mtm, target, items, cmds);
                mi.setSubmenu(Some(&child));
            }
            MenuEntry::Info(_) => mi.setEnabled(false),
            MenuEntry::Separator => unreachable!("separator handled above"),
        }
        menu.addItem(&mi);
    }
    menu
}

/// Create a monochrome template NSImage from Icon::EyeMonitoring for the macOS Menu Bar.
fn mark_image() -> Option<Retained<NSImage>> {
    const PT: f64 = 18.0;
    const SCALE: f32 = 2.0;
    let px = (PT as f32 * SCALE) as usize;
    let bmp = Bitmap::new(px, px, SCALE)?;
    let ctx = bmp.ctx();

    let s = 14.0 / 24.0;
    let ox = (PT - 24.0 * s) / 2.0;
    let oy = (PT - 24.0 * s) / 2.0;

    CGContext::translate_ctm(Some(ctx), ox as CGFloat, oy as CGFloat);
    CGContext::scale_ctm(Some(ctx), s as CGFloat, s as CGFloat);
    CGContext::set_rgb_stroke_color(Some(ctx), 0.0, 0.0, 0.0, 1.0);
    CGContext::set_line_width(Some(ctx), icons::STROKE as CGFloat);

    for part in icons::parts(Icon::EyeMonitoring) {
        if let Ok(segs) = nopeek_core::svgpath::parse(part.d) {
            let p = crate::mac::gfx::cg_path(&segs);
            CGContext::begin_path(Some(ctx));
            CGContext::add_path(Some(ctx), Some(&p));
            if part.fill {
                CGContext::set_rgb_fill_color(Some(ctx), 0.0, 0.0, 0.0, 1.0);
                CGContext::fill_path(Some(ctx));
            } else {
                CGContext::stroke_path(Some(ctx));
            }
        }
    }

    let cg = bmp.image()?;
    let img = NSImage::initWithCGImage_size(
        NSImage::alloc(),
        &cg,
        NSSize::new(PT as CGFloat, PT as CGFloat),
    );
    img.setTemplate(true);
    Some(img)
}
