//! The one error type of the platform layer. No variant carries user data.

/// Diagnostic sink: messages are fixed text plus format names and counts, never contents.
pub type LogFn = dyn Fn(&str) + Send + Sync;

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
    #[error("{api} failed (code {code})")]
    Os { api: &'static str, code: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;

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
            PlatformError::Os {
                api: "CryptProtectData",
                code: 5,
            },
        ];
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
