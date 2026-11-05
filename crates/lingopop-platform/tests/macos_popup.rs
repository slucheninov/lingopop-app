//! OWNER-RUN real-AppKit check of the non-activating popup panel. Never part of the default suite: this target has `harness = false` so it runs on
//! the process MAIN thread (AppKit requires it), and without `--ignored` it prints a skip line
//! and exits 0 at once, creating no window. Run it from a terminal while another app (e.g.
//! TextEdit) is frontmost:
//!
//! ```text
//! cargo test -p lingopop-platform --test macos_popup --locked -- --ignored
//! ```
//!
//! It shows a 360x100 panel at (100, 100) for an instant, checks that LingoPop did not become
//! active, the frontmost app is unchanged and the panel is not key, then hides the panel. It never
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
#[allow(unsafe_code)]
mod owner_run {
    use lingopop_platform::{
        configure_popup_panel, MacMonitors, MacPopupWindow, NoActivateWindow, Rect,
    };
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSWindow, NSWindowStyleMask, NSWorkspace,
    };

    fn frontmost_pid() -> Option<i32> {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier())
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        // SAFETY: `new` is NSObject's `+new` on a main-thread-only class, called on the main
        // thread; the window is kept alive (leaked by `configure_popup_panel`).
        let window: Retained<NSWindow> = unsafe { NSWindow::new(mtm) };
        let pointer = Retained::as_ptr(&window) as *mut std::ffi::c_void;
        let handle = configure_popup_panel(pointer).expect("panel configuration");
        assert!(
            window
                .styleMask()
                .contains(NSWindowStyleMask::NonactivatingPanel),
            "the class swap made it a non-activating panel"
        );

        let foreground_before_show = frontmost_pid();
        let popup = MacPopupWindow::new(MacMonitors::new());
        popup
            .show_at(handle, Rect::new(100, 100, 360, 100))
            .expect("show");
        assert!(window.isVisible(), "the panel is on screen");
        let foreground_after_show = frontmost_pid();
        assert_eq!(
            foreground_after_show, foreground_before_show,
            "showing the popup must not change the frontmost app"
        );
        assert!(!app.isActive(), "LingoPop must not become active");
        assert!(!window.isKeyWindow(), "the panel does not take key status");

        popup.hide(handle).expect("hide");
        assert!(!window.isVisible(), "the panel is ordered out");
        assert_eq!(frontmost_pid(), foreground_before_show);
    }
}
