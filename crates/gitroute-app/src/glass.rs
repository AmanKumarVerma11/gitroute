//! The sidebar's Liquid Glass. egui draws the whole window into one Metal
//! layer, so glass can't float above egui's own content. Instead a native
//! glass view goes behind egui's transparent sidebar and egui's layer is
//! raised over it: the glass blurs what's behind the window, and egui's text
//! draws on top. Page content doesn't scroll under the glass.
//!
//! macOS 26 gets NSGlassEffectView (Liquid Glass); earlier versions get
//! NSVisualEffectView's sidebar material. Needs the wgpu renderer: with
//! OpenGL, egui draws into the view itself and any native view covers it.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSGlassEffectView, NSTitlebarSeparatorStyle, NSToolbar, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowButton, NSWindowOrderingMode, NSWindowToolbarStyle,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The sidebar's width, the glass pane's gap to the window edges, and its
/// corner radius. The egui side lays out to the same numbers.
pub const SIDEBAR_WIDTH: f32 = 236.0;
pub const INSET: f32 = 8.0;
pub const RADIUS: f32 = 18.0;
/// The window's title area once the unified toolbar is in: the top band
/// where clicks drag the window, so egui content starts below it.
pub const TITLEBAR_HEIGHT: f32 = 52.0;

pub struct Glass {
    view: Retained<NSView>,
    host: Retained<NSView>,
}

impl Glass {
    /// Puts the glass under the window's egui view. `None` if the window
    /// isn't an AppKit window or this isn't the main thread.
    pub fn install(handle: &impl HasWindowHandle) -> Option<Glass> {
        let RawWindowHandle::AppKit(handle) = handle.window_handle().ok()?.as_raw() else {
            return None;
        };
        let mtm = MainThreadMarker::new()?;
        // SAFETY: winit hands out the window's live content view.
        let host: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.cast().as_ptr())? };
        let height = host.frame().size.height;
        let frame = NSRect::new(
            NSPoint::new(INSET.into(), INSET.into()),
            NSSize::new(
                (SIDEBAR_WIDTH - 2.0 * INSET).into(),
                height - 2.0 * f64::from(INSET),
            ),
        );
        let view: Retained<NSView> = if objc2::available!(macos = 26.0) {
            let glass = NSGlassEffectView::initWithFrame(mtm.alloc(), frame);
            glass.setCornerRadius(RADIUS.into());
            Retained::into_super(glass)
        } else {
            let effect = NSVisualEffectView::initWithFrame(mtm.alloc(), frame);
            effect.setMaterial(NSVisualEffectMaterial::Sidebar);
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect.setState(NSVisualEffectState::FollowsWindowActiveState);
            effect.setWantsLayer(true);
            unsafe {
                let layer: *mut AnyObject = msg_send![&*effect, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: f64::from(RADIUS)];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
            }
            Retained::into_super(effect)
        };
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
        view.setWantsLayer(true);
        host.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
        // An empty unified toolbar gives the macOS 26 layout: a taller title
        // area with the traffic lights inset inside the floating sidebar
        // (measured: close button at x 19, top 19, versus 9, 9 without).
        if let Some(window) = host.window() {
            window.setToolbar(Some(&NSToolbar::new(mtm)));
            window.setToolbarStyle(NSWindowToolbarStyle::Unified);
            window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
        }
        let glass = Glass { view, host };
        glass.keep_egui_on_top();
        Some(glass)
    }

    /// Orders the glass layer behind egui's Metal layer. Cheap; called again
    /// now and then in case the Metal layer is replaced.
    pub fn keep_egui_on_top(&self) {
        unsafe {
            let glass: *mut AnyObject = msg_send![&*self.view, layer];
            if !glass.is_null() {
                let _: () = msg_send![glass, setZPosition: -1.0f64];
            }
            for layer in sublayers(msg_send![&*self.host, layer]) {
                if class_name(layer).contains("Metal") {
                    let _: () = msg_send![layer, setZPosition: 1.0f64];
                }
            }
        }
        if std::env::var_os("GITROUTE_DEBUG_GLASS").is_some() {
            eprintln!("gitroute glass: {}", self.describe());
        }
    }

    /// The host's layers, back to front, for `GITROUTE_DEBUG_GLASS`.
    fn describe(&self) -> String {
        let root: *mut AnyObject = unsafe { msg_send![&*self.host, layer] };
        let layers: Vec<String> = sublayers(root)
            .into_iter()
            .map(|l| {
                let z: f64 = unsafe { msg_send![l, zPosition] };
                format!("{}(z={z})", class_name(l))
            })
            .collect();
        let buttons: Vec<String> = self
            .host
            .window()
            .map(|window| {
                let height = self.host.frame().size.height;
                [
                    NSWindowButton::CloseButton,
                    NSWindowButton::MiniaturizeButton,
                    NSWindowButton::ZoomButton,
                ]
                .into_iter()
                .filter_map(|kind| window.standardWindowButton(kind))
                .map(|b| {
                    let r = b.convertRect_toView(b.bounds(), None);
                    let top = height - r.origin.y - r.size.height;
                    format!("({:.0}, {top:.0})", r.origin.x)
                })
                .collect()
            })
            .unwrap_or_default();
        format!(
            "{} under {}, layers {layers:?}, traffic lights at {buttons:?}",
            class_name(Retained::as_ptr(&self.view) as *mut AnyObject),
            class_name(Retained::as_ptr(&self.host) as *mut AnyObject),
        )
    }
}

fn sublayers(layer: *mut AnyObject) -> Vec<*mut AnyObject> {
    if layer.is_null() {
        return Vec::new();
    }
    unsafe {
        let list: *mut AnyObject = msg_send![layer, sublayers];
        if list.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![list, count];
        (0..count)
            .map(|i| msg_send![list, objectAtIndex: i])
            .collect()
    }
}

fn class_name(object: *mut AnyObject) -> String {
    // SAFETY: callers pass live Objective-C objects.
    unsafe { (*object).class().name().to_string_lossy().into_owned() }
}
