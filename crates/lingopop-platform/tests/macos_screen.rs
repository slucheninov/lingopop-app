//! OWNER-RUN real-screen capture check. Never part of the default suite and never run by CI:
//! this target has `harness = false` (the display topology is read on the process main thread)
//! and without `--ignored` it prints a skip line and exits 0, touching nothing.
//!
//! With `--ignored` it only READS the Screen Recording state (never prompts, never requests):
//!
//! * not granted: asserts the capture service answers `PermissionDenied` and captures nothing;
//! * granted: it STILL captures nothing unless the owner explicitly authorizes a real capture of
//!   the top-left 100x50 points of the main display with `LINGOPOP_OWNER_SCREEN_CAPTURE=1`; the
//!   pixels are never printed or stored, only the size and the opaque alpha are asserted.
//!
//! ```text
//! cargo test -p lingopop-platform --test macos_screen --locked -- --ignored
//! LINGOPOP_OWNER_SCREEN_CAPTURE=1 cargo test -p lingopop-platform --test macos_screen --locked -- --ignored
//! ```

#[cfg(target_os = "macos")]
fn main() {
    if !std::env::args().any(|arg| arg == "--ignored") {
        println!("skipped: owner-run (pass --ignored; needs the macOS desktop)");
        return;
    }
    owner_run::run();
    println!("ok: macos_screen");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod owner_run {
    use lingopop_platform::{
        MacCapture, MacMonitors, MacPermissions, PermissionKind, PermissionState, Permissions,
        PlatformError, Rect, ScreenCapture,
    };
    use std::sync::Arc;

    pub fn run() {
        let monitors = MacMonitors::new();
        monitors
            .initialize_on_main_thread()
            .expect("topology setup on the main thread");
        let geometry = monitors.geometry();
        let permissions = Arc::new(MacPermissions::new());
        let capture = MacCapture::with_services(monitors, permissions.clone());
        let region = Rect::new(0, 0, 100, 50);
        let state = permissions.state(PermissionKind::ScreenRecording);
        if state != PermissionState::Granted {
            assert_eq!(
                capture.capture_region(region),
                Err(PlatformError::PermissionDenied(
                    PermissionKind::ScreenRecording
                ))
            );
            println!("screen recording not granted: denied without capturing, as required");
            return;
        }
        if std::env::var_os("LINGOPOP_OWNER_SCREEN_CAPTURE").is_none() {
            println!("granted, but LINGOPOP_OWNER_SCREEN_CAPTURE is not set: nothing captured");
            return;
        }
        let scale = geometry
            .displays()
            .iter()
            .find(|d| d.primary)
            .map(|d| d.backing_scale)
            .expect("a primary display");
        let image = capture.capture_region(region).expect("authorized capture");
        assert_eq!(f64::from(image.width), 100.0 * scale);
        assert_eq!(f64::from(image.height), 50.0 * scale);
        assert_eq!(
            image.rgba.len(),
            image.width as usize * image.height as usize * 4
        );
        assert!(image.rgba.chunks(4).all(|px| px[3] == 255));
    }
}
