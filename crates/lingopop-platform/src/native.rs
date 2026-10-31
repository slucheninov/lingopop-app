//! The bundle of native implementations the app wires in at startup.

use std::sync::Arc;

use lingopop_core::secrets::SecretProtector;

use crate::capture::ScreenCapture;
use crate::clipboard::Clipboard;
use crate::foreground::ForegroundWindow;
use crate::keys::KeySynth;
use crate::monitors::Monitors;
use crate::ocr::Ocr;
use crate::sleeper::Sleeper;

/// Every `Send + Sync` platform service. The hotkey service is NOT part of it: it is `!Send` on
/// Windows and must be created on the event-loop thread with `GlobalHotkeyService::new()`.
#[derive(Clone)]
pub struct Platform {
    pub popup_window: Arc<dyn crate::NoActivateWindow>,
    pub dismiss: Arc<dyn crate::DismissWatcher>,
    pub clipboard: Arc<dyn Clipboard>,
    pub keys: Arc<dyn KeySynth>,
    pub foreground: Arc<dyn ForegroundWindow>,
    pub monitors: Arc<dyn Monitors>,
    pub capture: Arc<dyn ScreenCapture>,
    pub ocr: Arc<dyn Ocr>,
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
        popup_window: Arc::new(crate::WindowsPopupWindow),
        dismiss: Arc::new(crate::WindowsDismissWatcher::new()),
        clipboard: Arc::new(WindowsClipboard::new(sleeper.clone())),
        keys: Arc::new(WindowsKeys::new()),
        foreground: Arc::new(WindowsForeground::new()),
        monitors: Arc::new(WindowsMonitors::new()),
        capture: Arc::new(WindowsCapture::new()),
        ocr: Arc::new(WindowsOcr::new()),
        protector: Arc::new(WindowsProtector::new()),
        sleeper,
    }
}

/// The macOS dev implementations: text-only clipboard, no screen capture or OCR, and the
/// PLAINTEXT `DevPlaintextProtector`. Dev only, never shipped.
#[cfg(target_os = "macos")]
pub fn native() -> Platform {
    use crate::dev_protector::DevPlaintextProtector;
    use crate::macos::{MacCapture, MacClipboard, MacForeground, MacKeys, MacMonitors, MacOcr};
    use crate::sleeper::SystemSleeper;

    Platform {
        popup_window: Arc::new(crate::MacPopupWindow),
        dismiss: Arc::new(crate::MacDismissWatcher::default()),
        clipboard: Arc::new(MacClipboard::new()),
        keys: Arc::new(MacKeys::new()),
        foreground: Arc::new(MacForeground::new()),
        monitors: Arc::new(MacMonitors::new()),
        capture: Arc::new(MacCapture::new()),
        ocr: Arc::new(MacOcr::new()),
        protector: Arc::new(DevPlaintextProtector::new()),
        sleeper: Arc::new(SystemSleeper::new()),
    }
}
