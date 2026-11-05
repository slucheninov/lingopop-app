//! The macOS popup as a non-activating `NSPanel` (port of the frozen app's `UI/PopupWindow.swift`).
//!
//! Tauri / tao creates the popup as a `TaoWindow` (tao 0.37.2 `platform_impl/macos/window.rs`:
//! `define_class!` `TaoWindow`, superclass `NSWindow`, ivars `WindowState { focusable }`,
//! overrides `canBecomeMainWindow` / `canBecomeKeyWindow` / `sendEvent:`). At setup, on the
//! main thread, [`configure_popup_panel`] swaps its class to [`LingoPopPopupPanel`] (the
//! `object_setClass` technique of tauri-nspanel), an `NSPanel` subclass that adds NO instance
//! variables. The swap is refused unless the runtime class is EXACTLY `TaoWindow` or plain
//! `NSWindow` (by name), inherits `NSWindow`, and the panel class needs no more bytes than the
//! existing allocation; the allocation, AppKit's own `NSWindow` state and tao's ivars stay in
//! place. A refused or failed conversion is an `Err` (fixed-text log), never an ordinary
//! window reported as configured. The panel is then configured once: `borderless |
//! nonactivatingPanel`, level `NSPopUpMenuWindowLevel`, collection behaviour `canJoinAllSpaces |
//! fullScreenAuxiliary | stationary`, `hidesOnDeactivate = false`, `becomesKeyOnlyIfNeeded =
//! true`, no shadow (the webview draws its own border).
//!
//! RULE: after the conversion the popup is no longer a `TaoWindow`: no tao / Tauri API that
//! relies on `TaoWindow`'s overrides or ivars may be called on the popup window. In
//! particular never `set_focusable` (it writes `WindowState.focusable` through a typed
//! `TaoWindow` reference), and never rely on tao's `canBecomeKeyWindow` / `sendEvent:`
//! (window dragging) behaviour; the shell sets `focusable(false)` only on the builder, before
//! the conversion, and positions / shows / hides the popup only through [`MacPopupWindow`].
//!
//! [`MacPopupWindow`] is the `NoActivateWindow` the presenter calls ON THE MAIN THREAD only:
//! `show_at` sets the frame (desktop Quartz points converted to AppKit) and uses
//! `orderFrontRegardless`, which never activates LingoPop; `hide` is `orderOut:`. Every call
//! off the main thread, or for any handle other than the configured panel, is refused.
use crate::error::platform_log;
use crate::{NoActivateWindow, PlatformError, Rect, WindowHandle};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{define_class, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSPanel, NSResponder, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::MacMonitors;
use crate::macos_geometry::MacGeometry;

/// `NSPopUpMenuWindowLevel` (`kCGPopUpMenuWindowLevel` = 101): above floating windows and the
/// Dock, below the menu bar's own menus and screen savers.
const POPUP_MENU_WINDOW_LEVEL: isize = 101;

/// Runtime classes the swap accepts (exact names): tao 0.37.2's window class, and plain
/// `NSWindow` (the owner-run test window). Anything else, including a KVO-generated subclass,
/// is refused.
const SWAPPABLE_CLASSES: [&str; 2] = ["TaoWindow", "NSWindow"];

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

/// Logs the fixed reason and returns the matching error (no window data in either).
fn refused(api: &'static str) -> PlatformError {
    platform_log(&format!("popup panel conversion refused: {api}"));
    PlatformError::Os { api, code: 0 }
}

/// Converts the popup window into the non-activating panel and returns its handle. MAIN
/// THREAD ONLY, called once at setup with the Tauri `WebviewWindow` (any
/// `HasWindowHandle` whose handle is AppKit). The `NSWindow` is derived as
/// `[handle.ns_view window]` and validated (non-nil, an `NSWindow`) before use, then retained
/// for the life of the process: later presenter calls use the cached handle only. Any refusal
/// (wrong thread, no AppKit handle, unexpected class or layout, failed swap) is an `Err`.
pub fn configure_popup_panel(window: &impl HasWindowHandle) -> Result<WindowHandle, PlatformError> {
    let mtm = MainThreadMarker::new().ok_or_else(not_main)?;
    let handle = window
        .window_handle()
        .map_err(|_| refused("popup window handle unavailable"))?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err(refused("popup window handle is not AppKit"));
    };
    // SAFETY: raw-window-handle's contract: the `ns_view` of a `WindowHandle` borrowed from
    // `window` is a valid, live `NSView` pointer for as long as `handle` lives (it does across
    // this function), and AppKit objects are used on the main thread (checked above). It is
    // only read as an object here and validated by `downcast_ref` before any method call.
    let object: &AnyObject = unsafe { appkit.ns_view.cast::<AnyObject>().as_ref() };
    let view = object
        .downcast_ref::<NSView>()
        .ok_or_else(|| refused("popup window handle is not an NSView"))?;
    let ns_window: Retained<NSWindow> = view
        .window()
        .ok_or_else(|| refused("popup view has no window"))?;
    convert(&ns_window)?;
    configure(&ns_window, mtm)?;
    // Retained once here for the process lifetime: the popup is never destroyed while
    // LingoPop runs, so the cached address stays valid for every later main-thread call.
    let address = Retained::into_raw(ns_window) as usize;
    CONFIGURED.store(address, Ordering::SeqCst);
    Ok(WindowHandle(address as u64))
}

/// The guarded `object_setClass` swap (idempotent for an already converted panel).
fn convert(window: &NSWindow) -> Result<(), PlatformError> {
    let current: &AnyClass = window.class();
    let panel_class = LingoPopPopupPanel::class();
    if std::ptr::eq(current, panel_class) {
        return Ok(());
    }
    let name = current.name().to_str().unwrap_or("");
    if !SWAPPABLE_CLASSES.contains(&name) {
        return Err(refused("popup window class is not TaoWindow or NSWindow"));
    }
    if !is_subclass(current, NSWindow::class()) {
        return Err(refused("popup window class does not inherit NSWindow"));
    }
    if panel_class.instance_size() > current.instance_size() {
        return Err(refused("popup panel class needs a larger allocation"));
    }
    let object = (window as *const NSWindow).cast_mut().cast::<AnyObject>();
    // SAFETY: `object` is a live NSWindow on the main thread (the caller holds a Retained).
    // Its runtime class is exactly `TaoWindow` or `NSWindow` (checked by name above), both
    // direct NSWindow subclasses (or NSWindow itself). LingoPopPopupPanel derives from NSWindow
    // through NSPanel, adds no ivars and needs no more than the existing allocation (checked
    // above; NSPanel adds no instance storage over NSWindow), so every ivar NSWindow / NSPanel
    // addresses lies inside the allocation and keeps its initialized value. tao's own ivars
    // stay in place, unread by the new class; the module RULE forbids tao APIs that read them.
    let previous = unsafe { objc2::ffi::object_setClass(object, panel_class) };
    if previous.is_null() || !std::ptr::eq(window.class(), panel_class) {
        return Err(refused("popup panel class swap failed"));
    }
    Ok(())
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

fn configure(window: &NSWindow, _mtm: MainThreadMarker) -> Result<(), PlatformError> {
    let panel = window
        .downcast_ref::<NSPanel>()
        .ok_or_else(|| refused("popup window is not an NSPanel after the swap"))?;
    window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    window.setHidesOnDeactivate(false);
    window.setHasShadow(false);
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    // setFloatingPanel resets the level to NSFloatingWindowLevel; set ours afterwards.
    window.setLevel(POPUP_MENU_WINDOW_LEVEL);
    Ok(())
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

/// OWNER-RUN TEST SUPPORT ONLY (`tests/macos_popup.rs`): a plain `NSWindow` created on the
/// main thread, exposed through an AppKit `HasWindowHandle` (its content view) so the safe
/// test driver goes through [`configure_popup_panel`] like the shell, plus read-only probes.
#[doc(hidden)]
pub struct OwnerRunWindow {
    window: Retained<NSWindow>,
    view: Retained<NSView>,
}

#[doc(hidden)]
impl OwnerRunWindow {
    /// Main thread only (`Err` elsewhere).
    pub fn new() -> Result<Self, PlatformError> {
        let mtm = MainThreadMarker::new().ok_or_else(not_main)?;
        // SAFETY: `+new` on NSWindow (`-init` = the designated initializer with a zero
        // content rect) on the main thread (the marker proves it); the result is owned.
        let window: Retained<NSWindow> = unsafe { NSWindow::new(mtm) };
        // SAFETY: the window is owned by this struct (a Retained), so it must not release
        // itself when closed; called on the main thread.
        unsafe { window.setReleasedWhenClosed(false) };
        let view = window.contentView().ok_or_else(unconfigured)?;
        Ok(Self { window, view })
    }
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }
    pub fn is_key(&self) -> bool {
        self.window.isKeyWindow()
    }
    /// The class swap made it an `NSPanel` with the non-activating style.
    pub fn is_nonactivating_panel(&self) -> bool {
        self.window.downcast_ref::<NSPanel>().is_some()
            && self
                .window
                .styleMask()
                .contains(NSWindowStyleMask::NonactivatingPanel)
    }
}

impl HasWindowHandle for OwnerRunWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let view = std::ptr::NonNull::from(&*self.view).cast::<std::ffi::c_void>();
        let raw = RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(view));
        // SAFETY: the view is retained by `self`, so the pointer stays valid for the borrow.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle source that must never be read: the main-thread check comes first.
    struct Untouchable;
    impl HasWindowHandle for Untouchable {
        fn window_handle(
            &self,
        ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            panic!("the window handle was read off the main thread");
        }
    }

    #[test]
    fn off_main_conversion_is_refused_before_the_handle_is_read() {
        // libtest runs tests on worker threads, never the process main thread.
        assert!(matches!(
            configure_popup_panel(&Untouchable),
            Err(PlatformError::Os {
                api: "popup panel off main thread",
                code: 0
            })
        ));
    }

    #[test]
    fn only_tao_and_plain_windows_are_swappable() {
        assert_eq!(SWAPPABLE_CLASSES, ["TaoWindow", "NSWindow"]);
    }

    #[test]
    fn off_main_show_and_hide_are_refused() {
        let popup = MacPopupWindow::new(MacMonitors::new());
        assert!(popup
            .show_at(WindowHandle(1), Rect::new(0, 0, 10, 10))
            .is_err());
        assert!(popup.hide(WindowHandle(1)).is_err());
    }
}
