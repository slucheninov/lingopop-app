//! Native launch-at-login checks (macOS only). The decision logic is covered on fakes by
//! `macos_login_service.rs`; this file touches the real ServiceManagement.
//!
//! * `unbundled_test_binary_touches_nothing` runs by default: a cargo test binary is not an
//!   installed `.app`, so enabling is `Unsupported` and disabling is a no-op WITHOUT any
//!   `SMAppService` call. It can never change the user's login configuration.
//! * `installed_bundle_roundtrip` is OWNER-RUN (`#[ignore]`): it registers the main app of the
//!   bundle it runs from. Never run it from the real LingoPop bundle. Build a THROWAWAY bundle
//!   with its own identifier, e.g.
//!   `mkdir -p /tmp/LingoPopLoginTest.app/Contents/MacOS`, copy the built test executable there
//!   as `login`, add an `Info.plist` with `CFBundleIdentifier` `app.lingopop.logintest` and
//!   `CFBundleExecutable` `login`, `codesign -s - --force /tmp/LingoPopLoginTest.app`, then run
//!   `/tmp/LingoPopLoginTest.app/Contents/MacOS/login --ignored installed_bundle_roundtrip`.
//!   The RAII guard unregisters it on every path; remove the bundle afterwards and check System
//!   Settings > General > Login Items for leftovers.
#![cfg(target_os = "macos")]

use std::path::Path;

use lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM;
use lingopop_platform::macos_login::MainAppService;
use lingopop_platform::{LoginItem, MacLoginItem, PlatformError, SmMainApp};

#[test]
fn unbundled_test_binary_touches_nothing() {
    assert!(
        !SmMainApp.is_installed_bundle(),
        "a test binary is not an app bundle"
    );
    let item = MacLoginItem::new(SmMainApp);
    assert_eq!(
        item.set_enabled(true, Path::new("/ignored")),
        Err(PlatformError::Unsupported(UNSUPPORTED_LOGIN_ITEM))
    );
    assert_eq!(item.set_enabled(false, Path::new("/ignored")), Ok(()));
}

/// Unregisters the throwaway bundle's login item on drop, whatever happened.
struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = MacLoginItem::new(SmMainApp).set_enabled(false, Path::new(""));
    }
}

#[test]
#[ignore = "owner-run: registers the login item of the throwaway bundle it runs from"]
fn installed_bundle_roundtrip() {
    if !SmMainApp.is_installed_bundle() {
        println!("skipped: not running from an installed .app bundle (see the file header)");
        return;
    }
    let _cleanup = Cleanup;
    let item = MacLoginItem::new(SmMainApp);
    // Either enabled, or registered and waiting for the user's approval: never a silent success.
    match item.set_enabled(true, Path::new("")) {
        Ok(()) => println!("enabled"),
        Err(PlatformError::LoginItemRequiresApproval) => println!("requires approval"),
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert_eq!(item.set_enabled(false, Path::new("")), Ok(()));
}
