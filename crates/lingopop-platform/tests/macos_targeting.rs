//! Owner-run checks of the real macOS targeting (NSWorkspace + Accessibility).
//!
//! Ignored by default: they read the real frontmost application, and with Accessibility granted
//! they query (and, for Chromium / Electron, may enable the accessibility tree of) the frontmost
//! app. The decision logic itself is covered on fakes in `tests/macos_targets.rs`.
//!
//! Not covered here and recorded as manual checks before a release: multiple windows of one app
//! (the check is process level, so a same-app different-window switch is a documented limit),
//! full-screen apps and Spaces, Chromium / Electron targets with and without
//! `AXManualAccessibility`, and a real Replace into each of them.
#![cfg(target_os = "macos")]

use lingopop_platform::{
    native, ForegroundWindow, MacForeground, PermissionKind, PlatformError, WindowHandle,
};

/// `Ok(true)` with Accessibility granted, `PermissionDenied` without it; never a bare `false`
/// for the application that is in front right now.
#[test]
#[ignore = "owner-run: touches TCC"]
fn real_frontmost_app_roundtrips_through_the_registry() {
    let fg = MacForeground::new();
    let handle = fg.current().expect("some application is in front");
    assert_eq!(fg.current(), Some(handle), "stable between polls");
    match fg.is_current_target(handle) {
        Ok(still) => assert!(still),
        Err(PlatformError::PermissionDenied(PermissionKind::Accessibility)) => {}
        Err(other) => panic!("unexpected: {other:?}"),
    }
    // Re-activating the application that is in front right now must succeed (it is the live
    // process the handle was captured from); an Err here means the identity check is wrong.
    assert_eq!(fg.restore(handle), Ok(()));
}

#[test]
#[ignore = "owner-run: touches TCC"]
fn handles_that_were_never_issued_are_rejected() {
    let fg = MacForeground::new();
    for raw in [0u64, 1, u64::MAX, 0x0100, 0xFFFF_FF00] {
        match fg.is_current_target(WindowHandle(raw)) {
            Ok(false) | Err(PlatformError::PermissionDenied(_)) => {}
            other => panic!("{raw:#x}: {other:?}"),
        }
        assert!(fg.restore(WindowHandle(raw)).is_err(), "{raw:#x}");
    }
}

#[test]
#[ignore = "owner-run: touches TCC"]
fn native_bundle_shares_one_registry_between_foreground_and_tray_tracker() {
    let platform = native();
    // Precondition: the tray tracker has remembered an external (not LingoPop) target. Run this
    // with another application in front when starting the test (e.g. from a terminal).
    let Some(handle) = platform.last_foreground.last_external() else {
        println!("skipped: the tray tracker remembered no external application");
        return;
    };
    // Whatever the tray tracker remembers must resolve in the foreground service: same registry.
    match platform.foreground.is_current_target(handle) {
        // Resolved: true when that app is still in front, false when the user has moved on;
        // either way the handle is known to the registry.
        Ok(_) => {}
        Err(PlatformError::PermissionDenied(PermissionKind::Accessibility)) => {
            println!("skipped: Accessibility is not granted, the check cannot resolve the handle");
            return;
        }
        Err(other) => panic!("unexpected: {other:?}"),
    }
    assert_eq!(
        platform.foreground.restore(handle),
        Ok(()),
        "a handle issued by the tray tracker must restore through the foreground service"
    );
}
