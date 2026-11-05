//! Raw ApplicationServices (Accessibility) calls used for source targeting.
//!
//! Every symbol below was checked against the macOS SDK header
//! `HIServices.framework/Headers/AXUIElement.h` (`AXIsProcessTrusted`,
//! `AXUIElementCreateApplication`, `AXUIElementCopyAttributeValue`,
//! `AXUIElementSetAttributeValue`, `AXUIElementPerformAction`, `AXUIElementSetMessagingTimeout`,
//! `AXUIElementGetTypeID`). The attribute and action names (`kAXFocusedWindowAttribute`,
//! `kAXRaiseAction`) are `CFSTR` macros, not linkable symbols, so they are built at run time.
//! `AXManualAccessibility` is not in the headers; it is the attribute Chromium and Electron
//! document for switching their accessibility tree on.
//!
//! Everything here is best effort and silent: a failing call means "no AX identity", never an
//! error, and nothing is logged (a window title or role could be user data).

use std::ffi::c_void;

use core_foundation::base::{CFGetTypeID, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::{CFString, CFStringRef};

type AXUIElementRef = *const c_void;
type AXError = i32;
const AX_SUCCESS: AXError = 0;

/// How long an unresponsive application may stall one call (the system default is 6 s, which
/// would outlast the platform gate's 3 s job budget).
const MESSAGING_TIMEOUT_SECONDS: f32 = 0.5;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> AXError;
}

/// `AXIsProcessTrusted()`: whether this process may use the Accessibility API. Never prompts.
pub(super) fn process_trusted() -> bool {
    // SAFETY: AXIsProcessTrusted takes no arguments and only reads the process's TCC state.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// An owned `AXUIElementRef` of a window. Compared with CFEqual.
#[derive(Clone, PartialEq)]
pub(crate) struct AxWindow(CFType);

// SAFETY: the wrapped value is a retained, immutable CF object; CF reference counting is atomic
// and the AX functions that take it are documented as callable from any thread (they message the
// target process). It is never mutated through a shared reference.
unsafe impl Send for AxWindow {}
// SAFETY: see the `Send` impl; `&AxWindow` only exposes CFEqual and read-only AX calls.
unsafe impl Sync for AxWindow {}

impl std::fmt::Debug for AxWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AxWindow")
    }
}

/// The AX element of the application with this pid, with a short messaging timeout.
fn application(pid: i32) -> Option<CFType> {
    // SAFETY: plain call; follows the create rule, so a non-null result is owned by us.
    let raw = unsafe { AXUIElementCreateApplication(pid) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a non-null owned CF object returned by a `Create` function.
    let app = unsafe { CFType::wrap_under_create_rule(raw as CFTypeRef) };
    // SAFETY: `app` is a valid AXUIElementRef for the duration of the call.
    unsafe { AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), MESSAGING_TIMEOUT_SECONDS) };
    Some(app)
}

fn element_ref(element: &CFType) -> AXUIElementRef {
    element.as_CFTypeRef()
}

fn focused_window_of(app: &CFType) -> Option<AxWindow> {
    let attribute = CFString::new("AXFocusedWindow");
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `app` is a valid AXUIElementRef, `attribute` a valid CFString, and `value` a
    // writable out-pointer; on success the value is returned retained (copy rule).
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element_ref(app),
            attribute.as_concrete_TypeRef(),
            &mut value,
        )
    };
    if status != AX_SUCCESS || value.is_null() {
        return None;
    }
    // SAFETY: success and non-null: `value` is an owned CF object (copy rule).
    let window = unsafe { CFType::wrap_under_create_rule(value) };
    // SAFETY: `window` is a valid CF object; GetTypeID calls have no side effects.
    let is_element = unsafe { CFGetTypeID(window.as_CFTypeRef()) == AXUIElementGetTypeID() };
    is_element.then_some(AxWindow(window))
}

/// Asks the application to build its accessibility tree (Chromium and Electron do not until a
/// client says so). The result is ignored: applications without the attribute just refuse.
fn enable_manual_accessibility(app: &CFType) {
    let attribute = CFString::new("AXManualAccessibility");
    let on = CFBoolean::true_value();
    // SAFETY: valid element, CFString and CFBoolean; the call does not retain or mutate them.
    unsafe {
        AXUIElementSetAttributeValue(
            element_ref(app),
            attribute.as_concrete_TypeRef(),
            on.as_CFTypeRef(),
        )
    };
}

/// The focused window of the application, if Accessibility will tell. A first failure is
/// retried once after `AXManualAccessibility` was attempted (the Chromium / Electron case).
pub(super) fn focused_window(pid: i32) -> Option<AxWindow> {
    let app = application(pid)?;
    if let Some(window) = focused_window_of(&app) {
        return Some(window);
    }
    // At most once per process id: an application that simply has no window (Finder, a
    // menu-bar app) must not get the attribute written on every sample.
    if first_manual_attempt(pid) {
        enable_manual_accessibility(&app);
        return focused_window_of(&app);
    }
    None
}

/// Pids `AXManualAccessibility` was already attempted for (bounded; the oldest is forgotten).
fn first_manual_attempt(pid: i32) -> bool {
    use std::sync::Mutex;
    static ATTEMPTED: Mutex<Vec<i32>> = Mutex::new(Vec::new());
    let mut attempted = ATTEMPTED.lock().unwrap_or_else(|p| p.into_inner());
    if attempted.contains(&pid) {
        return false;
    }
    if attempted.len() >= 64 {
        attempted.remove(0);
    }
    attempted.push(pid);
    true
}

/// `AXRaise` on a captured window, best effort. `true` = the application accepted the action.
pub(super) fn raise(window: &AxWindow) -> bool {
    let action = CFString::new("AXRaise");
    // SAFETY: valid element and CFString; the timeout call only sets a per-element value.
    unsafe {
        AXUIElementSetMessagingTimeout(element_ref(&window.0), MESSAGING_TIMEOUT_SECONDS);
        AXUIElementPerformAction(element_ref(&window.0), action.as_concrete_TypeRef()) == AX_SUCCESS
    }
}
