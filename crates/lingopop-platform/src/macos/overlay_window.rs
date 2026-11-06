//! `AppKitOverlay`: the native region-selection overlay (port of `UI/CaptureOverlayWindow.swift`
//! of the frozen Swift app), the `OverlayBackend` the portable lifecycle in `macos/overlay.rs`
//! drives. MAIN THREAD ONLY: the engine posts `create` / `draw` / `cleanup` to it.
//!
//! Like the Swift original: one borderless panel per display covering its frame at
//! `NSScreenSaverWindowLevel` (can join all Spaces, full-screen auxiliary, no shadow, clear
//! background), a view that dims the display with black at alpha 0.3 (`DIM_ALPHA`), cuts the
//! selection out (`NSCompositingOperationCopy` with a clear fill), strokes it white at 1.5 pt
//! and draws the `W × H` label (11 pt medium, white, black 0.6 shadow) below the selection;
//! the app is activated and the crosshair cursor set; input comes from ONE local `NSEvent`
//! monitor (left down / dragged / up and keyDown) that consumes the events the overlay uses
//! (mouse events and Escape) and lets every other key pass. Deviation: LingoPop resigning
//! active ends the pick (an Escape for the engine) instead of leaving a pending pick behind
//! panels that AppKit's `hidesOnDeactivate` default would hide in the Swift app.
//!
//! The windows never become the remembered external source target (they belong to our own pid).
//! Nothing here logs user data. Every symbol is available on macOS 13.
use super::event_monitors::quartz_point;
use super::foreground::identity;
use crate::macos_dismiss::ESCAPE_KEY_CODE;
use crate::macos_geometry::MacGeometry;
use crate::macos_overlay::{selection_in_display, OverlayBackend, OverlaySink};
use crate::macos_targets::AppIdentity;
use crate::overlay::{size_label, OverlayInput, BORDER_PT, DIM_ALPHA, LABEL_PT};
use crate::{PlatformError, Rect};
use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationDidResignActiveNotification, NSBackingStoreType,
    NSBezierPath, NSColor, NSCompositingOperation, NSCursor, NSEvent, NSEventMask, NSEventType,
    NSFont, NSFontAttributeName, NSFontWeightMedium, NSForegroundColorAttributeName, NSPanel,
    NSRectFill, NSRectFillUsingOperation, NSResponder, NSRunningApplication,
    NSScreenSaverWindowLevel, NSShadow, NSShadowAttributeName, NSStringDrawing, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSDictionary, NSNotificationCenter, NSObject, NSObjectProtocol, NSOperationQueue, NSPoint,
    NSRect, NSSize, NSString,
};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::Arc;

fn os(api: &'static str) -> PlatformError {
    PlatformError::Os { api, code: 0 }
}

define_class!(
    // SAFETY: NSPanel may be subclassed; the subclass adds no ivars and does not implement
    // Drop, and the two overrides match NSWindow's `BOOL` signatures.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LingoPopOverlayPanel"]
    struct OverlayPanel;

    impl OverlayPanel {
        /// A borderless window refuses key status by default (Swift: `canBecomeKey` true).
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

impl OverlayPanel {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        // SAFETY: `initWithContentRect:styleMask:backing:defer:` is NSWindow's designated
        // initializer, called once on a fresh allocation with valid arguments.
        unsafe {
            msg_send![super(this),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false]
        }
    }
}

/// What a view draws: the part of the selection on its display (display-local, top-left) and
/// the full selection for the size label.
#[derive(Clone, Copy)]
struct ViewSelection {
    clipped: Rect,
    full: Rect,
}

struct ViewIvars {
    selection: Cell<Option<ViewSelection>>,
}

define_class!(
    // SAFETY: NSView may be subclassed; the ivars are plain `Cell`s used on the main thread
    // only (`MainThreadOnly`), and the overrides match NSView's signatures.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LingoPopOverlayView"]
    #[ivars = ViewIvars]
    struct OverlayView;

    impl OverlayView {
        /// Top-left origin, so display-local desktop points are view coordinates.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.paint();
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::crosshairCursor());
        }
    }
);

fn ns_rect(r: Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(f64::from(r.x), f64::from(r.y)),
        NSSize::new(f64::from(r.width), f64::from(r.height)),
    )
}

impl OverlayView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(ViewIvars {
            selection: Cell::new(None),
        });
        // SAFETY: `initWithFrame:` is NSView's designated initializer on a fresh allocation.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn set_selection(&self, selection: Option<ViewSelection>) {
        self.ivars().selection.set(selection);
        self.setNeedsDisplay(true);
    }

    fn paint(&self) {
        let bounds = self.bounds();
        NSColor::blackColor()
            .colorWithAlphaComponent(f64::from(DIM_ALPHA) / 255.0)
            .setFill();
        NSRectFill(bounds);
        let Some(sel) = self.ivars().selection.get() else {
            return;
        };
        // Swift: a selection of one point or less is not drawn.
        if sel.clipped.width <= 1 || sel.clipped.height <= 1 {
            return;
        }
        let clipped = ns_rect(sel.clipped);
        NSColor::clearColor().setFill();
        NSRectFillUsingOperation(clipped, NSCompositingOperation::Copy);
        NSColor::whiteColor().setStroke();
        let border = NSBezierPath::bezierPathWithRect(clipped);
        border.setLineWidth(BORDER_PT);
        border.stroke();
        self.draw_label(sel, bounds);
    }

    fn draw_label(&self, sel: ViewSelection, bounds: NSRect) {
        let label = NSString::from_str(&size_label(sel.full));
        let font = NSFont::systemFontOfSize_weight(LABEL_PT, {
            // SAFETY: a read-only AppKit constant.
            unsafe { NSFontWeightMedium }
        });
        let white = NSColor::whiteColor();
        let shadow = NSShadow::new();
        shadow.setShadowColor(Some(&NSColor::blackColor().colorWithAlphaComponent(0.6)));
        shadow.setShadowOffset(NSSize::new(0.0, -1.0));
        shadow.setShadowBlurRadius(2.0);
        // SAFETY: read-only AppKit attribute-name constants.
        let keys: [&NSString; 3] = unsafe {
            [
                NSFontAttributeName,
                NSForegroundColorAttributeName,
                NSShadowAttributeName,
            ]
        };
        let values: [&AnyObject; 3] = [&font, &white, &shadow];
        let attributes: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&keys, &values);
        // SAFETY: the dictionary holds the font, colour and shadow objects AppKit expects for
        // these attribute names.
        let size = unsafe { label.sizeWithAttributes(Some(&attributes)) };
        let clipped = ns_rect(sel.clipped);
        // Swift (y-up): x = max(minX + 4, maxX - w - 4), y = max(4, minY - h - 4), i.e. below
        // the selection but never off the bottom of the display. This view is flipped.
        let x =
            (clipped.origin.x + 4.0).max(clipped.origin.x + clipped.size.width - size.width - 4.0);
        let y = (clipped.origin.y + clipped.size.height + 4.0)
            .min(bounds.size.height - size.height - 4.0);
        // SAFETY: same attribute dictionary as above; drawing happens inside `drawRect:`.
        unsafe { label.drawAtPoint_withAttributes(NSPoint::new(x, y), Some(&attributes)) };
    }
}

struct Entry {
    panel: Retained<OverlayPanel>,
    view: Retained<OverlayView>,
    /// The display's desktop rect (Quartz points).
    display: Rect,
}

/// The registered center and the token that removes the resign-active observer again.
type ResignObserver = (
    Retained<NSNotificationCenter>,
    Retained<ProtocolObject<dyn NSObjectProtocol>>,
);

struct Native {
    entries: Vec<Entry>,
    monitor: Option<Retained<AnyObject>>,
    resign: Option<ResignObserver>,
}

thread_local! {
    /// The live overlay; touched only on the main thread.
    static ACTIVE: RefCell<Option<Native>> = const { RefCell::new(None) };
    /// Remembered by `create` before activating, consumed by `restore_previous_app`.
    static PREVIOUS: Cell<Option<AppIdentity>> = const { Cell::new(None) };
}

/// The frontmost application (full identity), unless it is this process (nothing to hand back
/// to).
fn frontmost_other_app() -> Option<AppIdentity> {
    autoreleasepool(|_| {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        identity(&app).filter(|id| id.pid as u32 != std::process::id())
    })
}

/// Whether the previous app gets focus back: only while LingoPop is still the active app (not
/// after the user switched away), never to ourselves, an exited app or a process that is not
/// the one we remembered. "Same process" is `AppIdentity::same_process` (pid AND a readable,
/// equal kernel start time), and the remembered app must also have a launch date that the
/// current one repeats exactly: two missing launch dates are no evidence of identity, and a
/// date that disappeared (`Some` then `None`) is not a match either, so such an app is never
/// refocused (fail safe; `same_process` alone treats a missing date as compatible).
fn should_restore(
    own_active: bool,
    terminated: bool,
    is_self: bool,
    previous: &AppIdentity,
    current: &AppIdentity,
) -> bool {
    own_active
        && !terminated
        && !is_self
        && previous.launch.is_some()
        && previous.launch == current.launch
        && previous.same_process(current)
}

/// The production overlay backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct AppKitOverlay;

impl AppKitOverlay {
    pub fn new() -> Self {
        Self
    }
}

/// Where the event happened, in Quartz desktop points: the event's own CGEvent location, and
/// only when AppKit gives no CGEvent the current `NSEvent.mouseLocation` (the same fallback as
/// the dismiss monitors).
fn mouse_point(event: &NSEvent, geometry: &MacGeometry) -> crate::Point {
    quartz_point(event).unwrap_or_else(|| {
        let p = NSEvent::mouseLocation();
        geometry.appkit_point_to_desktop(p.x, p.y)
    })
}

fn install_monitor(
    geometry: MacGeometry,
    sink: Arc<OverlaySink>,
) -> Result<Retained<AnyObject>, PlatformError> {
    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseDragged
        | NSEventMask::LeftMouseUp
        | NSEventMask::KeyDown;
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid NSEvent for the duration of the handler call.
        let ev = unsafe { event.as_ref() };
        let kind = ev.r#type();
        let input = if kind == NSEventType::LeftMouseDown {
            Some(OverlayInput::Down(mouse_point(ev, &geometry)))
        } else if kind == NSEventType::LeftMouseDragged {
            Some(OverlayInput::Move(mouse_point(ev, &geometry)))
        } else if kind == NSEventType::LeftMouseUp {
            Some(OverlayInput::Up(mouse_point(ev, &geometry)))
        } else if kind == NSEventType::KeyDown && ev.keyCode() == ESCAPE_KEY_CODE {
            Some(OverlayInput::Escape)
        } else {
            None
        };
        match input {
            // Consumed: the overlay is the only receiver of these events.
            Some(input) if sink.push(input) => std::ptr::null_mut(),
            _ => event.as_ptr(),
        }
    });
    // SAFETY: the handler returns either the event it was given (still owned by AppKit, the
    // documented "pass it on" result) or NULL to consume it; it retains nothing.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
        .ok_or_else(|| os("addLocalMonitorForEventsMatchingMask"))
}

/// LingoPop resigning active (the user switched away) ends the pick like Escape.
fn install_resign_observer(sink: Arc<OverlaySink>) -> ResignObserver {
    let block = RcBlock::new(move |_| {
        sink.push(OverlayInput::Escape);
    });
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: AppKit's static notification name, no object filter, delivered on the main queue;
    // the block captures only the thread-safe sink.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidResignActiveNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    };
    (center, token)
}

impl OverlayBackend for AppKitOverlay {
    fn create(&self, geometry: &MacGeometry, sink: Arc<OverlaySink>) -> Result<(), PlatformError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| os("region overlay off main thread"))?;
        // A leftover overlay (a cleanup that never ran) must not survive into a new one.
        self.cleanup();
        let mut native = Native {
            entries: Vec::new(),
            monitor: None,
            resign: None,
        };
        let result = (|| {
            for d in geometry.displays() {
                let display = d.desktop_bounds.enclosing_rect()?;
                let frame = geometry.desktop_to_appkit(display)?;
                let frame = NSRect::new(
                    NSPoint::new(frame.x, frame.y),
                    NSSize::new(frame.width, frame.height),
                );
                let panel = OverlayPanel::new(mtm, frame);
                // SAFETY: the Rust `Retained` owns the window; AppKit must not release it on
                // `close` (a double release otherwise).
                unsafe { panel.setReleasedWhenClosed(false) };
                panel.setOpaque(false);
                panel.setBackgroundColor(Some(&NSColor::clearColor()));
                panel.setLevel(NSScreenSaverWindowLevel);
                panel.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::FullScreenAuxiliary,
                );
                panel.setIgnoresMouseEvents(false);
                panel.setAcceptsMouseMovedEvents(true);
                panel.setHasShadow(false);
                // Deactivation ends the pick (observer below), it never hides the panels.
                panel.setHidesOnDeactivate(false);
                let view = OverlayView::new(mtm, NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
                panel.setContentView(Some(&view));
                panel.setFrame_display(frame, false);
                native.entries.push(Entry {
                    panel,
                    view,
                    display,
                });
            }
            native.monitor = Some(install_monitor(geometry.clone(), sink.clone())?);
            native.resign = Some(install_resign_observer(sink));
            Ok(())
        })();
        let ok = result.is_ok();
        // Keep whatever exists, so the engine's `cleanup` removes a partial overlay.
        ACTIVE.with(|a| *a.borrow_mut() = Some(native));
        result?;
        if ok {
            // Remember who was in front BEFORE our app takes over, so the cleanup can give
            // focus back (an accessory app has no key window afterwards, typing goes nowhere).
            PREVIOUS.with(|p| p.set(frontmost_other_app()));
            super::activate_app();
            NSCursor::crosshairCursor().set();
            ACTIVE.with(|a| {
                if let Some(native) = a.borrow().as_ref() {
                    for e in &native.entries {
                        e.panel.makeKeyAndOrderFront(None);
                    }
                }
            });
        }
        Ok(())
    }

    fn draw(&self, selection: Option<Rect>) {
        ACTIVE.with(|a| {
            if let Some(native) = a.borrow().as_ref() {
                for e in &native.entries {
                    let local = selection.and_then(|full| {
                        selection_in_display(e.display, full)
                            .map(|clipped| ViewSelection { clipped, full })
                    });
                    e.view.set_selection(local);
                }
            }
        });
    }

    fn cleanup(&self) {
        let Some(native) = ACTIVE.with(|a| a.borrow_mut().take()) else {
            return;
        };
        if let Some(monitor) = native.monitor {
            // SAFETY: the token returned by addLocalMonitorForEventsMatchingMask on this
            // (main) thread, removed exactly once.
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
        if let Some((center, token)) = native.resign {
            // SAFETY: exactly the token this center's registration returned.
            unsafe { center.removeObserver((*token).as_ref()) };
        }
        for e in &native.entries {
            e.panel.orderOut(None);
            e.panel.close();
        }
        NSCursor::arrowCursor().set();
    }

    fn restore_previous_app(&self) {
        let Some(previous) = PREVIOUS.with(|p| p.take()) else {
            return;
        };
        // The pick ended because LingoPop resigned active (the user switched to another app on
        // purpose): pulling focus back to the app they left would undo that switch. Selection,
        // Escape and cancel end with LingoPop still active (create activated it).
        let own_active = MainThreadMarker::new()
            .is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive());
        autoreleasepool(|_| {
            let Some(app) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(previous.pid)
            else {
                return;
            };
            let Some(current) = identity(&app) else {
                return;
            };
            if !should_restore(
                own_active,
                app.isTerminated(),
                previous.pid as u32 == std::process::id(),
                &previous,
                &current,
            ) {
                return;
            }
            // Swift `ScreenTranslateFlow` uses `.activateIgnoringOtherApps` here as well.
            #[allow(deprecated)]
            let options = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
            let _ = app.activateWithOptions(options);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::should_restore;
    use crate::macos_targets::AppIdentity;

    fn app(pid: i32, start: Option<i64>, launch: Option<i64>) -> AppIdentity {
        AppIdentity {
            pid,
            process_start: start,
            bundle_id: None,
            launch,
        }
    }

    #[test]
    fn restores_only_while_lingopop_is_still_active_and_the_app_is_the_same_live_process() {
        let a = app(7, Some(100), Some(5));
        assert!(should_restore(true, false, false, &a, &a));
        // The user switched away (LingoPop resigned active): leave their choice alone.
        assert!(!should_restore(false, false, false, &a, &a));
        assert!(!should_restore(true, true, false, &a, &a), "exited");
        assert!(!should_restore(true, false, true, &a, &a), "ourselves");
        let reused = app(7, Some(200), Some(5));
        assert!(
            !should_restore(true, false, false, &a, &reused),
            "pid reused"
        );
    }

    #[test]
    fn an_app_without_a_launch_date_is_never_refocused() {
        // launchDate None == None used to count as the same process.
        let a = app(7, Some(100), None);
        assert!(!should_restore(true, false, false, &a, &a));
        // An unreadable start time is not the same process either.
        let b = app(7, None, Some(5));
        assert!(!should_restore(true, false, false, &b, &b));
    }

    #[test]
    fn a_launch_date_that_disappeared_or_changed_is_not_the_same_app() {
        let remembered = app(7, Some(100), Some(5));
        // Some -> None: same_process alone would accept it (missing metadata is compatible).
        let lost = app(7, Some(100), None);
        assert!(!should_restore(true, false, false, &remembered, &lost));
        // The reverse asymmetry and a different date are refused too.
        assert!(!should_restore(true, false, false, &lost, &remembered));
        let other = app(7, Some(100), Some(6));
        assert!(!should_restore(true, false, false, &remembered, &other));
    }
}
