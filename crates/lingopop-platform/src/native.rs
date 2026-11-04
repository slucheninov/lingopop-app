//! The bundle of native implementations the app wires in at startup.

use std::sync::Arc;

use lingopop_core::secrets::SecretProtector;

use crate::capture::ScreenCapture;
use crate::clipboard::Clipboard;
use crate::foreground::ForegroundWindow;
use crate::keys::KeySynth;
use crate::monitors::Monitors;
use crate::ocr::Ocr;
use crate::permissions::Permissions;
use crate::sleeper::Sleeper;

/// Every `Send + Sync` platform service. The hotkey service is NOT part of it: it is `!Send` on
/// Windows and must be created on the event-loop thread with `GlobalHotkeyService::new()`.
#[derive(Clone)]
pub struct Platform {
    pub overlay: Arc<dyn crate::RegionOverlay>,
    pub system: Arc<dyn crate::SystemInfo>,
    pub login_item: Arc<dyn crate::LoginItem>,
    pub popup_window: Arc<dyn crate::NoActivateWindow>,
    pub dismiss: Arc<dyn crate::DismissWatcher>,
    pub clipboard: Arc<dyn Clipboard>,
    pub keys: Arc<dyn KeySynth>,
    pub foreground: Arc<dyn ForegroundWindow>,
    pub last_foreground: Arc<dyn crate::LastForeground>,
    pub monitors: Arc<dyn Monitors>,
    pub capture: Arc<dyn ScreenCapture>,
    pub ocr: Arc<dyn Ocr>,
    /// Privacy permissions (Accessibility, Screen Recording). `AlwaysGranted` on Windows.
    pub permissions: Arc<dyn Permissions>,
    pub protector: Arc<dyn SecretProtector>,
    pub sleeper: Arc<dyn Sleeper>,
}

/// The Windows implementations (DPAPI protector).
#[cfg(windows)]
pub fn native() -> Platform {
    use crate::sleeper::SystemSleeper;
    use crate::windows::{
        WindowsCapture, WindowsClipboard, WindowsForeground, WindowsKeys, WindowsMonitors,
        WindowsOcr, WindowsProtector,
    };

    let sleeper: Arc<dyn Sleeper> = Arc::new(SystemSleeper::new());
    Platform {
        overlay: Arc::new(crate::WindowsOverlay::new()),
        system: native_system(),
        login_item: Arc::new(crate::windows::WindowsLoginItem::new()),
        popup_window: Arc::new(crate::WindowsPopupWindow),
        dismiss: Arc::new(crate::WindowsDismissWatcher::new()),
        clipboard: Arc::new(WindowsClipboard::new(sleeper.clone())),
        keys: Arc::new(WindowsKeys::new()),
        foreground: Arc::new(WindowsForeground::new()),
        // A failed hook only loses tray-triggered operations; the hotkey flow never needs it.
        last_foreground: match crate::windows::WindowsLastForeground::new() {
            Ok(tracker) => Arc::new(tracker),
            Err(e) => {
                crate::error::platform_log(&format!("last-foreground tracker unavailable: {e}"));
                Arc::new(crate::last_foreground::NoLastForeground)
            }
        },
        monitors: Arc::new(WindowsMonitors::new()),
        capture: Arc::new(WindowsCapture::new()),
        ocr: Arc::new(WindowsOcr::new()),
        permissions: Arc::new(crate::permissions::AlwaysGranted),
        protector: Arc::new(WindowsProtector::new()),
        sleeper,
    }
}

/// The macOS dev implementations: text-only clipboard, no screen capture or OCR, and the
/// Keychain-backed secret protection.
#[cfg(target_os = "macos")]
pub fn native() -> Platform {
    use crate::macos::{MacCapture, MacClipboard, MacForeground, MacKeys, MacMonitors, MacOcr};
    use crate::sleeper::SystemSleeper;
    use crate::MacProtector;

    // This is the one shared monitor/cache instance. Shell setup initializes it through
    // Monitors; Task 15 must also give MacCapture a clone of this instance.
    let monitors = Arc::new(MacMonitors::new());
    // One target registry: handles from the last-foreground tracker must resolve in `foreground`.
    let foreground = MacForeground::new();
    let last_foreground = crate::macos::MacLastForeground::new(&foreground);
    Platform {
        overlay: Arc::new(crate::MacOverlay),
        system: native_system(),
        login_item: Arc::new(crate::macos::MacLoginItem),
        popup_window: Arc::new(crate::MacPopupWindow),
        dismiss: Arc::new(crate::MacDismissWatcher::default()),
        clipboard: Arc::new(MacClipboard::new()),
        keys: Arc::new(MacKeys::new()),
        foreground: Arc::new(foreground),
        last_foreground: Arc::new(last_foreground),
        monitors,
        capture: Arc::new(MacCapture::new()),
        ocr: Arc::new(MacOcr::new()),
        permissions: Arc::new(crate::macos::MacPermissions::new()),
        protector: Arc::new(MacProtector::new()),
        sleeper: Arc::new(SystemSleeper::new()),
    }
}

/// Construct only the system service, without starting trackers or installing hooks.
#[cfg(windows)]
pub fn native_system() -> Arc<dyn crate::SystemInfo> {
    Arc::new(crate::windows::WindowsSystem)
}

/// The system-only macOS dev service for the relaunch wait path.
#[cfg(target_os = "macos")]
pub fn native_system() -> Arc<dyn crate::SystemInfo> {
    Arc::new(crate::macos::MacSystem)
}
