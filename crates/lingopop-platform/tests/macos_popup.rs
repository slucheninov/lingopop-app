//! OWNER-RUN real-AppKit check of the non-activating popup panel and the passive dismiss
//! monitors. Never part of the default suite: this target has `harness = false` so it runs on
//! the process MAIN thread (AppKit requires it), and without `--ignored` it prints a skip line
//! and exits 0 at once, creating no window. Run it from a terminal while another app (e.g.
//! TextEdit) is frontmost:
//!
//! ```text
//! cargo test -p lingopop-platform --test macos_popup --locked -- --ignored
//! ```
//!
//! It shows a 360x100 panel at (100, 100) for an instant, checks that LingoPop did not become
//! active, the frontmost app is unchanged and the panel is not key, then installs and removes
//! the dismiss monitors through the real `AppKitMonitors` and hides the panel. It never
//! prompts for or resets TCC permissions and never touches the Keychain.

#[cfg(target_os = "macos")]
fn main() {
    if !std::env::args().any(|arg| arg == "--ignored") {
        println!("skipped: owner-run (pass --ignored; needs the macOS desktop)");
        return;
    }
    owner_run::run();
    println!("ok: macos_popup");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod owner_run {
    use lingopop_platform::{
        configure_popup_panel, AppKitMonitors, DismissWatcher, MacDismissWatcher, MacMonitors,
        MacPopupWindow, MainThread, NoActivateWindow, OwnerRunWindow, PlatformError, Rect,
    };
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
    use std::sync::Arc;

    /// This test IS the main thread: run every job inline.
    struct Inline;
    impl MainThread for Inline {
        fn is_main_thread(&self) -> bool {
            true
        }
        fn post(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), PlatformError> {
            job();
            Ok(())
        }
    }

    fn frontmost_pid() -> Option<i32> {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier())
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        // A plain NSWindow from the main-thread-checked platform helper, converted through
        // the same safe `HasWindowHandle` entry the shell uses for the Tauri window.
        let window = OwnerRunWindow::new().expect("owner-run window");
        let handle = configure_popup_panel(&window).expect("panel configuration");
        assert!(
            window.is_nonactivating_panel(),
            "the class swap made it a non-activating panel"
        );

        let foreground_before_show = frontmost_pid();
        let popup = MacPopupWindow::new(MacMonitors::new());
        popup
            .show_at(handle, Rect::new(100, 100, 360, 100))
            .expect("show");
        assert!(window.is_visible(), "the panel is on screen");
        let foreground_after_show = frontmost_pid();
        assert_eq!(
            foreground_after_show, foreground_before_show,
            "showing the popup must not change the frontmost app"
        );
        assert!(!app.isActive(), "LingoPop must not become active");
        assert!(!window.is_key(), "the panel does not take key status");

        let watcher = MacDismissWatcher::new(Arc::new(AppKitMonitors));
        watcher.bind_main_thread(Arc::new(Inline));
        println!("escape available: {}", watcher.escape_available());
        watcher
            .start(Rect::new(100, 100, 360, 100))
            .expect("monitors");
        watcher.stop();
        watcher.shutdown();
        assert!(watcher.start(Rect::new(0, 0, 1, 1)).is_err());

        popup.hide(handle).expect("hide");
        assert!(!window.is_visible(), "the panel is ordered out");
        assert_eq!(frontmost_pid(), foreground_before_show);
    }
}
