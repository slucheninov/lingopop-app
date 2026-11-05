//! The macOS popup as a non-activating `NSPanel` (port of the frozen app's `UI/PopupWindow.swift`).
//!
//! Tauri / tao creates the popup as a `TaoWindow` (an `NSWindow` subclass). At setup, on the
//! main thread, [`configure_popup_panel`] swaps its class to [`LingoPopPopupPanel`], an
//! `NSPanel` subclass that adds NO instance variables, so the existing allocation (which is at
//! least `NSWindow`-sized plus tao's ivars) stays valid; tao's ivars are left untouched in place
//! and its `set_focusable` keeps working on them. The panel is then configured once:
//! `borderless | nonactivatingPanel`, level `NSPopUpMenuWindowLevel`, collection behaviour
//! `canJoinAllSpaces | fullScreenAuxiliary | stationary`, `hidesOnDeactivate = false`,
//! `becomesKeyOnlyIfNeeded = true`, no shadow (the webview draws its own border).
//!
//! [`MacPopupWindow`] is the `NoActivateWindow` the presenter calls ON THE MAIN THREAD only:
//! `show_at` sets the frame (desktop Quartz points converted to AppKit) and uses
//! `orderFrontRegardless`, which never activates LingoPop; `hide` is `orderOut:`. Every call
//! off the main thread, or for any handle other than the configured panel, is refused.
use crate::{NoActivateWindow, PlatformError, Rect, WindowHandle};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{define_class, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSPanel, NSResponder, NSScreen, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::MacMonitors;
use crate::macos_geometry::MacGeometry;

/// `NSPopUpMenuWindowLevel` (`kCGPopUpMenuWindowLevel` = 101): above floating windows and the
/// Dock, below the menu bar's own menus and screen savers.
const POPUP_MENU_WINDOW_LEVEL: isize = 101;

define_class!(
    // SAFETY: NSPanel may be subclassed; the subclass adds no ivars and does not implement
    // Drop, and the two overrides below match NSWindow's `BOOL` signatures.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LingoPopPopupPanel"]
    pub struct LingoPopPopupPanel;

    impl LingoPopPopupPanel {
        /// A borderless window refuses key status by default; allowing it lets the
        /// webview take keyboard focus only when a view needs it (`becomesKeyOnlyIfNeeded`).
        /// A non-activating panel becoming key never activates LingoPop.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

/// The configured panel's address (0 = none). Show / hide refuse every other handle.
static CONFIGURED: AtomicUsize = AtomicUsize::new(0);

fn not_main() -> PlatformError {
    PlatformError::Os {
        api: "popup panel off main thread",
        code: 0,
    }
}

fn unconfigured() -> PlatformError {
    PlatformError::Os {
        api: "popup panel not configured",
        code: 0,
    }
}

/// Converts the tao popup window (`ns_window` from Tauri's `WebviewWindow::ns_window()`) into
/// the non-activating panel and returns its handle. MAIN THREAD ONLY, called once at setup.
/// When the class swap is not safe (unexpected class layout) the window is still configured
/// as a plain borderless floating window, a fixed line is logged and the handle is returned:
/// it then is not a non-activating panel, which the owner-run focus check would catch.
pub fn configure_popup_panel(ns_window: *mut c_void) -> Result<WindowHandle, PlatformError> {
    let mtm = MainThreadMarker::new().ok_or_else(not_main)?;
    if ns_window.is_null() {
        return Err(unconfigured());
    }
    let object = ns_window.cast::<AnyObject>();
    // SAFETY: `ns_window` is the live NSWindow pointer Tauri returned for the popup on this
    // (main) thread; reading its class has no side effects.
    let current: &AnyClass = unsafe { &*objc2::ffi::object_getClass(object) };
    let window_class = NSWindow::class();
    let panel_class = LingoPopPopupPanel::class();
    let is_window = is_subclass(current, window_class);
    let swappable = is_window && panel_class.instance_size() <= current.instance_size();
    if !is_window {
        return Err(unconfigured());
    }
    if swappable && !std::ptr::eq(current, panel_class) {
        // SAFETY: the object is an NSWindow (subclass) instance whose allocation is at least
        // `current.instance_size()` bytes; LingoPopPopupPanel derives from NSWindow through
        // NSPanel, adds no ivars and needs no more than that size (checked above), so every
        // ivar the new class (and NSWindow / NSPanel) addresses lies inside the allocation and
        // keeps its value. tao's own ivars stay in place, unused by the new class.
        unsafe {
            objc2::ffi::object_setClass(object, panel_class);
        }
    } else if !swappable {
        crate::error::platform_log("popup panel class swap refused: unexpected window layout");
    }
    // SAFETY: the pointer is a live NSWindow (now possibly LingoPopPopupPanel) used on the
    // main thread. Retaining it keeps the pointer valid for the life of the process: the
    // Retained is deliberately leaked, the popup is never destroyed while LingoPop runs.
    let window: Retained<NSWindow> =
        unsafe { Retained::retain(ns_window.cast::<NSWindow>()) }.ok_or_else(unconfigured)?;
    configure(&window, swappable, mtm);
    let address = Retained::as_ptr(&window) as usize;
    std::mem::forget(window);
    CONFIGURED.store(address, Ordering::SeqCst);
    Ok(WindowHandle(address as u64))
}

fn is_subclass(mut class: &AnyClass, ancestor: &AnyClass) -> bool {
    loop {
        if std::ptr::eq(class, ancestor) {
            return true;
        }
        match class.superclass() {
            Some(parent) => class = parent,
            None => return false,
        }
    }
}

fn configure(window: &NSWindow, panel: bool, _mtm: MainThreadMarker) {
    let mut style = NSWindowStyleMask::Borderless;
    if panel {
        style |= NSWindowStyleMask::NonactivatingPanel;
    }
    window.setStyleMask(style);
    window.setLevel(POPUP_MENU_WINDOW_LEVEL);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    window.setHidesOnDeactivate(false);
    window.setHasShadow(false);
    if let Some(panel) = panel.then(|| window.downcast_ref::<NSPanel>()).flatten() {
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        // setFloatingPanel resets the level to NSFloatingWindowLevel; restore ours.
        window.setLevel(POPUP_MENU_WINDOW_LEVEL);
    }
}

/// The configured panel for `handle`, on the main thread; otherwise a fixed error.
fn panel_for(handle: WindowHandle) -> Result<(&'static NSWindow, MainThreadMarker), PlatformError> {
    let mtm = MainThreadMarker::new().ok_or_else(not_main)?;
    let address = CONFIGURED.load(Ordering::SeqCst);
    if address == 0 || handle.0 != address as u64 {
        return Err(unconfigured());
    }
    // SAFETY: `address` is the NSWindow retained and leaked by `configure_popup_panel`, so it
    // stays alive for the process; it is only dereferenced on the main thread (checked above).
    Ok((unsafe { &*(address as *const NSWindow) }, mtm))
}

/// `NoActivateWindow` over the configured panel. Main thread only (the presenter dispatches).
#[derive(Debug, Clone)]
pub struct MacPopupWindow {
    monitors: MacMonitors,
}

impl MacPopupWindow {
    /// `monitors` is the same topology cache `native()` hands to the rest of the bundle.
    pub fn new(monitors: MacMonitors) -> Self {
        Self { monitors }
    }
}

fn appkit_frame(
    geometry: &MacGeometry,
    rect: Rect,
    mtm: MainThreadMarker,
) -> Result<NSRect, PlatformError> {
    if rect.is_empty() {
        return Err(PlatformError::InvalidRegion);
    }
    let native = if geometry.displays().is_empty() {
        // Topology not read yet: the primary screen is the first NSScreen.
        let primary_height = NSScreen::screens(mtm)
            .firstObject()
            .map(|screen| screen.frame().size.height)
            .ok_or(PlatformError::InvalidRegion)?;
        crate::macos_geometry::NativeRect::new(
            rect.x.into(),
            primary_height - f64::from(rect.y) - f64::from(rect.height),
            rect.width.into(),
            rect.height.into(),
        )
    } else {
        geometry.desktop_to_appkit(rect)?
    };
    Ok(NSRect::new(
        NSPoint::new(native.x, native.y),
        NSSize::new(native.width, native.height),
    ))
}

impl NoActivateWindow for MacPopupWindow {
    fn show_at(&self, window: WindowHandle, rect: Rect) -> Result<(), PlatformError> {
        let (panel, mtm) = panel_for(window)?;
        let frame = appkit_frame(&self.monitors.geometry(), rect, mtm)?;
        panel.setFrame_display(frame, true);
        panel.orderFrontRegardless();
        Ok(())
    }

    fn hide(&self, window: WindowHandle) -> Result<(), PlatformError> {
        let (panel, _) = panel_for(window)?;
        panel.orderOut(None);
        Ok(())
    }
}
