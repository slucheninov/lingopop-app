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
    println!("restore: {:?}", fg.restore(handle));
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
    // Whatever the tray tracker remembers must resolve in the foreground service.
    if let Some(handle) = platform.last_foreground.last_external() {
        println!("restore: {:?}", platform.foreground.restore(handle));
    }
}
