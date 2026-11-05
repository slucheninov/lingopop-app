//! `AppKitMonitors`: the real `MonitorBackend` over passive `NSEvent` monitors (see
//! `macos/dismiss.rs` for the decision logic, which this file only feeds).
//!
//! Global monitors (`addGlobalMonitorForEventsMatchingMask:handler:`, macOS 10.6) observe
//! events sent to OTHER apps and can never consume them; local monitors
//! (`addLocalMonitorForEventsMatchingMask:handler:`, macOS 10.6) observe our own app's events
//! and here always return the event unchanged, so nothing is swallowed. Global keyDown events
//! are delivered only with Accessibility trust. Tokens live in a main-thread `thread_local!`
//! and are removed with `removeMonitor:` (macOS 10.6) on that thread. Callbacks read only the
//! key code, modifier flags, location and the `kCGEventSourceUserData` marker, never characters.
use crate::macos_dismiss::{DismissSink, MonitorBackend, MonitorSet, ObservedEvent};
use crate::{PlatformError, Point};
use block2::RcBlock;
use core_graphics::display::CGDisplay;
use core_graphics::geometry::CGPoint;
use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

/// `kCGEventSourceUserData`.
const EVENT_SOURCE_USER_DATA: u32 = 42;

/// `CGEventRef` as returned by `-[NSEvent CGEvent]` (autoreleased, borrowed for the callback).
#[repr(transparent)]
#[derive(Clone, Copy)]
struct CgEventRef(*mut c_void);

// SAFETY: a CGEventRef is a pointer to the opaque struct `__CGEvent`; this is exactly the
// Objective-C type encoding of the `CGEvent` property's return type (`^{__CGEvent=}`).
unsafe impl Encode for CgEventRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
}

thread_local! {
    /// Installed monitor tokens; touched only on the main thread.
    static TOKENS: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
}

fn install_error(api: &'static str) -> PlatformError {
    PlatformError::Os { api, code: 0 }
}

/// Marker and Quartz location of `event`, when AppKit exposes its CGEvent.
fn cg_fields(event: &NSEvent) -> Option<(i64, CGPoint)> {
    // SAFETY: `CGEvent` is a read-only NSEvent property (macOS 10.5) returning a CGEventRef
    // or NULL, encoded as `^{__CGEvent=}` (see `CgEventRef`).
    let cg: CgEventRef = unsafe { msg_send![event, CGEvent] };
    if cg.0.is_null() {
        return None;
    }
    // SAFETY: `cg.0` is a non-null CGEventRef owned by `event`, which outlives this call;
    // both functions only read it.
    unsafe {
        Some((
            CGEventGetIntegerValueField(cg.0, EVENT_SOURCE_USER_DATA),
            CGEventGetLocation(cg.0),
        ))
    }
}

/// The event's own Quartz location (what a CGEvent carries, top-left origin), floored like the
/// cursor reads; `None` when AppKit exposes no CGEvent. Used by the region overlay so a press
/// is placed where it happened, not where the mouse is by the time the handler runs.
pub(super) fn quartz_point(event: &NSEvent) -> Option<Point> {
    cg_fields(event).map(|(_, location)| Point {
        x: location.x.floor() as i32,
        y: location.y.floor() as i32,
    })
}

/// Current mouse location in Quartz points (AppKit's is bottom-left, Y-up).
fn fallback_location() -> CGPoint {
    let appkit = NSEvent::mouseLocation();
    let primary_height = CGDisplay::main().bounds().size.height;
    CGPoint::new(appkit.x, primary_height - appkit.y)
}

fn mouse_event(event: &NSEvent) -> ObservedEvent {
    let (marker, location) = cg_fields(event).unwrap_or_else(|| (0, fallback_location()));
    ObservedEvent::MouseDown {
        location: Point {
            x: location.x.floor() as i32,
            y: location.y.floor() as i32,
        },
        marker,
    }
}

fn key_event(event: &NSEvent) -> ObservedEvent {
    let marker = cg_fields(event).map_or(0, |(marker, _)| marker);
    ObservedEvent::KeyDown {
        // Only keyDown monitors call this, so `keyCode` is valid for the event type.
        key_code: event.keyCode(),
        modified: event.modifierFlags().intersects(
            NSEventModifierFlags::Shift
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Command,
        ),
        marker,
    }
}

fn add_global(
    mask: NSEventMask,
    sink: Arc<DismissSink>,
    read: fn(&NSEvent) -> ObservedEvent,
) -> Result<(), PlatformError> {
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        // SAFETY: AppKit passes a valid NSEvent for the duration of the handler call.
        sink.observe(read(unsafe { event.as_ref() }));
    });
    let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &handler)
        .ok_or_else(|| install_error("addGlobalMonitorForEventsMatchingMask"))?;
    TOKENS.with(|tokens| tokens.borrow_mut().push(token));
    Ok(())
}

fn add_local(
    mask: NSEventMask,
    sink: Arc<DismissSink>,
    read: fn(&NSEvent) -> ObservedEvent,
) -> Result<(), PlatformError> {
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit passes a valid NSEvent for the duration of the handler call.
        sink.observe(read(unsafe { event.as_ref() }));
        event.as_ptr()
    });
    // SAFETY: the handler returns the event it was given, unchanged and still owned by AppKit,
    // which is the documented "pass it on" result; nothing is swallowed or retained.
    let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
        .ok_or_else(|| install_error("addLocalMonitorForEventsMatchingMask"))?;
    TOKENS.with(|tokens| tokens.borrow_mut().push(token));
    Ok(())
}

/// The production monitor backend. Install / remove only from the main thread (the
/// `MacDismissWatcher` reconcile job guarantees it).
#[derive(Debug, Default, Clone, Copy)]
pub struct AppKitMonitors;

impl MonitorBackend for AppKitMonitors {
    fn trusted(&self) -> bool {
        super::ax::process_trusted()
    }

    fn install(&self, set: MonitorSet, sink: Arc<DismissSink>) -> Result<(), PlatformError> {
        if objc2::MainThreadMarker::new().is_none() {
            return Err(install_error("dismiss monitors off main thread"));
        }
        let mouse =
            NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
        if set.global_mouse {
            add_global(mouse, sink.clone(), mouse_event)?;
        }
        if set.local_mouse {
            add_local(mouse, sink.clone(), mouse_event)?;
        }
        if set.global_key {
            add_global(NSEventMask::KeyDown, sink.clone(), key_event)?;
        }
        if set.local_key {
            add_local(NSEventMask::KeyDown, sink, key_event)?;
        }
        Ok(())
    }

    fn remove_all(&self) {
        let tokens = TOKENS.with(|tokens| std::mem::take(&mut *tokens.borrow_mut()));
        for token in tokens {
            // SAFETY: `token` was returned by addGlobal/LocalMonitorForEventsMatchingMask on
            // this (main) thread and is removed exactly once.
            unsafe { NSEvent::removeMonitor(&token) };
        }
    }
}
