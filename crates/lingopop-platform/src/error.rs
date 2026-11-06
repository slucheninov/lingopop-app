//! The one error type of the platform layer. No variant carries user data.

/// Diagnostic sink: messages are fixed text plus format names and counts, never contents.
pub type LogFn = dyn Fn(&str) + Send + Sync;

static PLATFORM_LOG: std::sync::OnceLock<std::sync::Arc<LogFn>> = std::sync::OnceLock::new();

/// Routes the platform layer's own fixed-text diagnostics (hook installation, tracker startup:
/// API names, codes and error `Display` only, never user data) to `log`. The shell sets it once,
/// BEFORE `native()`; later calls are ignored and without it the lines are dropped.
pub fn set_platform_log(log: std::sync::Arc<LogFn>) {
    let _ = PLATFORM_LOG.set(log);
}

pub(crate) fn platform_log(line: &str) {
    if let Some(log) = PLATFORM_LOG.get() {
        log(line);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("{0} is not supported on this platform")]
    Unsupported(&'static str),
    #[error("input was blocked: the target app runs with higher privileges (UIPI)")]
    InputBlocked,
    #[error("the clipboard is busy")]
    ClipboardBusy,
    #[error("OCR is unavailable: no recognizer language pack is installed")]
    OcrUnavailable,
    #[error("the hotkey is already registered by another application")]
    HotkeyConflict,
    #[error("the hotkey is not valid")]
    InvalidHotkey,
    #[error("the capture region is empty or outside the screen")]
    InvalidRegion,
    #[error("{} permission was denied", .0.display_name())]
    PermissionDenied(crate::permissions::PermissionKind),
    #[error("launch at login needs approval in System Settings")]
    LoginItemRequiresApproval,
    #[error("{api} failed (code {code})")]
    Os { api: &'static str, code: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionKind;

    #[test]
    fn platform_error_display_and_debug_carry_no_payload() {
        let all = [
            PlatformError::Unsupported("ocr"),
            PlatformError::InputBlocked,
            PlatformError::ClipboardBusy,
            PlatformError::OcrUnavailable,
            PlatformError::HotkeyConflict,
            PlatformError::InvalidHotkey,
            PlatformError::InvalidRegion,
            PlatformError::PermissionDenied(PermissionKind::Accessibility),
            PlatformError::PermissionDenied(PermissionKind::ScreenRecording),
            PlatformError::LoginItemRequiresApproval,
            PlatformError::Os {
                api: "CryptProtectData",
                code: 5,
            },
        ];
        // Adding a variant without listing it above fails to compile here.
        for e in &all {
            match e {
                PlatformError::Unsupported(_)
                | PlatformError::InputBlocked
                | PlatformError::ClipboardBusy
                | PlatformError::OcrUnavailable
                | PlatformError::HotkeyConflict
                | PlatformError::InvalidHotkey
                | PlatformError::InvalidRegion
                | PlatformError::PermissionDenied(_)
                | PlatformError::LoginItemRequiresApproval
                | PlatformError::Os { .. } => {}
            }
        }
        for e in &all {
            let shown = e.to_string();
            assert!(!shown.is_empty());
            assert!(
                !shown.contains('{') && !shown.contains('"') && !shown.contains('\''),
                "{shown}"
            );
        }
        assert_eq!(
            PlatformError::Os {
                api: "CryptProtectData",
                code: 5
            }
            .to_string(),
            "CryptProtectData failed (code 5)"
        );
        assert_eq!(
            PlatformError::InputBlocked.to_string(),
            "input was blocked: the target app runs with higher privileges (UIPI)"
        );
    }

    #[test]
    fn platform_error_is_send_sync_clone_eq() {
        fn check<T: Send + Sync + Clone + PartialEq + Eq + std::fmt::Debug>() {}
        check::<PlatformError>();
        let e = PlatformError::Os { api: "X", code: 1 };
        assert_eq!(e.clone(), e);
    }
}
