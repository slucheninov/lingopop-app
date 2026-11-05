//! User-visible messages of the flow as catalog keys (the mac English source strings) plus
//! arguments. Arguments are only names, counts and platform error `Display` texts, never the
//! user's text.

use lingopop_core::errors::TranslationError;
use lingopop_core::provider::ProviderType;
use lingopop_platform::pasteboard::UNSUPPORTED_SNAPSHOT;
use lingopop_platform::{PermissionKind, PlatformError};

use crate::gate::GateError;
use crate::view_model::UserMessageView;

pub const KEY_NOTHING_SELECTED: &str =
    "Nothing selected. Select some text first, then press the hotkey.";
pub const KEY_CLIPBOARD_TIMEOUT: &str = "Clipboard didn't update in time. Try again.";
pub const KEY_MAX_INPUT: &str = "Text is %@ characters, max is %@. Adjust in Settings → Main.";
pub const KEY_MISSING_KEY: &str = "API key missing for %@. Open Settings → Providers to add one.";
pub const KEY_NO_TEXT_FOUND_RETRY: &str =
    "No text found in the selected area. Try a larger selection or use Retry to send the image to AI.";
pub const KEY_NO_TEXT_FOUND: &str = "No text found in the selected area. Try a larger selection.";
pub const KEY_SELECTION_TOO_SMALL: &str = "Selection is too small. Try selecting a larger area.";
/// NEW (Windows only).
pub const KEY_INPUT_BLOCKED: &str = "LingoPop can't reach an app running as administrator. Run LingoPop as administrator, or copy and paste the text manually.";
/// Windows catalog key (`i18n/windows.json`).
pub const KEY_FOREGROUND_LOST_WINDOWS: &str = "Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with Ctrl+V.";
/// macOS catalog key (`i18n/macos.json`), the same sentence with the Command shortcut.
pub const KEY_FOREGROUND_LOST_MACOS: &str = "Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with ⌘V.";
/// Replace did not paste because the original app could not be brought back or is no longer in
/// front (a lost, stale or evicted target): the text is on the clipboard for a manual paste.
/// Chosen by the build target so each platform shows its own paste shortcut.
#[cfg(target_os = "macos")]
pub const KEY_FOREGROUND_LOST: &str = KEY_FOREGROUND_LOST_MACOS;
#[cfg(not(target_os = "macos"))]
pub const KEY_FOREGROUND_LOST: &str = KEY_FOREGROUND_LOST_WINDOWS;
/// NEW (Windows only).
pub const KEY_OCR_UNAVAILABLE: &str = "No OCR language is installed. Add a language with Optical character recognition in Settings → Time & language → Language & region, or use Retry to send the image to AI.";
/// NEW (Windows only).
pub const KEY_PLATFORM_GENERIC: &str = "Something went wrong on this PC (%@). Try again.";
/// NEW (Windows only).
pub const KEY_SCREEN_TIMEOUT: &str = "Screen capture or text recognition took too long. Try again.";

/// NEW (macOS only): Accessibility is missing, so the selection cannot be read or pasted.
pub const KEY_PERMISSION_ACCESSIBILITY: &str = "LingoPop needs Accessibility access to read selected text. Grant it in Settings → Main → Permissions.";
/// NEW (macOS only): Replace needs Accessibility to look at the foreground and send the paste.
/// Shown as the notice of the kept result; nothing was pasted.
pub const KEY_REPLACE_NEEDS_ACCESSIBILITY: &str =
    "Replace needs Accessibility access. Allow LingoPop in System Settings, then try again.";
/// NEW (macOS only): without Accessibility the popup cannot observe Escape (a global keyDown
/// monitor needs it); click-outside still closes it. Added by the shell as the popup notice.
pub const KEY_ESCAPE_UNAVAILABLE: &str =
    "Escape can't close this popup until LingoPop has Accessibility access. Click outside it to close.";
/// NEW (macOS only): Screen Recording is missing, so Screen Translate cannot capture.
pub const KEY_PERMISSION_SCREEN_RECORDING: &str = "LingoPop needs Screen Recording access to translate the screen. Grant it in Settings → Main → Permissions.";
/// NEW (macOS only): the previous clipboard holds content that cannot be preserved (promised,
/// lazy, `dyn.*`, oversized or unreadable types), so the operation stops before it touches the
/// clipboard.
pub const KEY_PASTEBOARD_UNSUPPORTED: &str =
    "Clipboard content can't be preserved. Nothing was changed.";
/// Mac catalog key (`MainSettingsTab.swift`). Conditional wording, never a claim that access was
/// granted: macOS reads a new Screen Recording grant only after a relaunch, so this process cannot
/// tell. Detail line of the denied message on a retry after `request` already ran, and the line
/// under the Screen Recording row in Settings.
pub const KEY_SCREEN_RECORDING_RELAUNCH: &str =
    "After granting access, fully quit and relaunch LingoPop.";

fn plain(key: &str) -> UserMessageView {
    UserMessageView::new(key, Vec::new())
}

pub fn nothing_selected() -> UserMessageView {
    plain(KEY_NOTHING_SELECTED)
}

/// Both arguments are decimal strings (the mac catalog uses `%@` for both).
pub fn max_input(chars: usize, limit: u32) -> UserMessageView {
    UserMessageView::new(KEY_MAX_INPUT, vec![chars.to_string(), limit.to_string()])
}

pub fn missing_key(provider: ProviderType) -> UserMessageView {
    UserMessageView::new(KEY_MISSING_KEY, vec![provider.display_name().to_string()])
}

/// `vision_available`: some vision provider could take the image (Retry is offered).
pub fn no_text_found(vision_available: bool) -> UserMessageView {
    plain(if vision_available {
        KEY_NO_TEXT_FOUND_RETRY
    } else {
        KEY_NO_TEXT_FOUND
    })
}

pub fn selection_too_small() -> UserMessageView {
    plain(KEY_SELECTION_TOO_SMALL)
}

/// `PermissionDenied` maps to the per-kind macOS key (never the Windows administrator text).
/// `Unsupported`, `Os`, `HotkeyConflict` and `InvalidHotkey` use the generic key with the
/// error's `Display` (no payload by construction); `InvalidRegion` means the selection is too small.
pub fn platform_message(e: &PlatformError) -> UserMessageView {
    match e {
        PlatformError::ClipboardBusy => plain(KEY_CLIPBOARD_TIMEOUT),
        PlatformError::InputBlocked => plain(KEY_INPUT_BLOCKED),
        PlatformError::OcrUnavailable => plain(KEY_OCR_UNAVAILABLE),
        PlatformError::PermissionDenied(PermissionKind::Accessibility) => {
            plain(KEY_PERMISSION_ACCESSIBILITY)
        }
        PlatformError::PermissionDenied(PermissionKind::ScreenRecording) => {
            plain(KEY_PERMISSION_SCREEN_RECORDING)
        }
        PlatformError::InvalidRegion => selection_too_small(),
        PlatformError::Unsupported(what) if *what == UNSUPPORTED_SNAPSHOT => {
            plain(KEY_PASTEBOARD_UNSUPPORTED)
        }
        PlatformError::Unsupported(_)
        | PlatformError::HotkeyConflict
        | PlatformError::InvalidHotkey
        | PlatformError::Os { .. } => {
            UserMessageView::new(KEY_PLATFORM_GENERIC, vec![e.to_string()])
        }
    }
}

/// Like `gate_message` for a failed Replace: a missing Accessibility permission has its own
/// sentence (it is about replacing, not about reading the selection).
pub fn replace_message(e: &GateError) -> UserMessageView {
    match e {
        GateError::Platform(PlatformError::PermissionDenied(PermissionKind::Accessibility)) => {
            plain(KEY_REPLACE_NEEDS_ACCESSIBILITY)
        }
        other => gate_message(other),
    }
}

/// `Busy` / `TimedOut` mean the clipboard flow did not finish in time; a panic is generic.
pub fn gate_message(e: &GateError) -> UserMessageView {
    match e {
        GateError::Busy | GateError::TimedOut => plain(KEY_CLIPBOARD_TIMEOUT),
        GateError::Panicked => UserMessageView::new(KEY_PLATFORM_GENERIC, vec![e.to_string()]),
        GateError::Platform(p) => platform_message(p),
    }
}

/// Like `gate_message` for Screen Translate work (capture, OCR, PNG encoding): `Busy` /
/// `TimedOut` mean the screen job was too slow, not that the clipboard did not update.
pub fn screen_gate_message(e: &GateError) -> UserMessageView {
    match e {
        GateError::Busy | GateError::TimedOut => plain(KEY_SCREEN_TIMEOUT),
        other => gate_message(other),
    }
}

/// Summary and detail lines of a provider error.
pub fn translation_error_view(e: &TranslationError) -> (UserMessageView, Vec<UserMessageView>) {
    (
        e.user_summary().into(),
        e.user_detail()
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_message_input_blocked() {
        assert_eq!(
            platform_message(&PlatformError::InputBlocked),
            plain(KEY_INPUT_BLOCKED)
        );
    }

    #[test]
    fn platform_message_unpreservable_pasteboard_has_its_own_key() {
        assert_eq!(
            platform_message(&PlatformError::Unsupported(UNSUPPORTED_SNAPSHOT)),
            plain(KEY_PASTEBOARD_UNSUPPORTED)
        );
        // Any other unsupported feature keeps the generic message.
        assert_eq!(
            platform_message(&PlatformError::Unsupported("ocr")).key,
            KEY_PLATFORM_GENERIC
        );
        assert_eq!(
            gate_message(&GateError::Platform(PlatformError::Unsupported(
                UNSUPPORTED_SNAPSHOT
            ))),
            plain(KEY_PASTEBOARD_UNSUPPORTED)
        );
    }

    #[test]
    fn platform_message_permission_denied_uses_the_per_kind_mac_key() {
        assert_eq!(
            platform_message(&PlatformError::PermissionDenied(
                PermissionKind::Accessibility
            )),
            plain(KEY_PERMISSION_ACCESSIBILITY)
        );
        assert_eq!(
            platform_message(&PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording
            )),
            plain(KEY_PERMISSION_SCREEN_RECORDING)
        );
        // Mac never shows the Windows UIPI / administrator text for a missing permission.
        for kind in PermissionKind::ALL {
            let m = platform_message(&PlatformError::PermissionDenied(kind));
            assert_ne!(m.key, KEY_INPUT_BLOCKED);
            assert!(!m.key.to_lowercase().contains("administrator"));
            assert!(m.args.is_empty());
        }
        assert!(KEY_INPUT_BLOCKED.contains("running as administrator"));
        assert_eq!(
            gate_message(&GateError::Platform(PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording
            ))),
            plain(KEY_PERMISSION_SCREEN_RECORDING)
        );
        assert_eq!(
            screen_gate_message(&GateError::Platform(PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording
            ))),
            plain(KEY_PERMISSION_SCREEN_RECORDING)
        );
    }

    #[test]
    fn mac_permission_keys_are_translated_on_macos_only() {
        use lingopop_core::i18n::{lookup_for, CatalogPlatform};
        use lingopop_core::Language;
        for key in [
            KEY_PERMISSION_ACCESSIBILITY,
            KEY_PERMISSION_SCREEN_RECORDING,
            KEY_SCREEN_RECORDING_RELAUNCH,
        ] {
            for language in [Language::Pl, Language::Uk, Language::De] {
                assert_ne!(lookup_for(CatalogPlatform::Macos, language, key), key);
                assert_eq!(lookup_for(CatalogPlatform::Windows, language, key), key);
            }
        }
        assert_eq!(
            KEY_SCREEN_RECORDING_RELAUNCH,
            "After granting access, fully quit and relaunch LingoPop."
        );
    }

    #[test]
    fn platform_message_foreground_lost() {
        let m = plain(KEY_FOREGROUND_LOST);
        assert!(m.args.is_empty());
        assert!(KEY_FOREGROUND_LOST_WINDOWS.ends_with("Ctrl+V."));
        assert!(KEY_FOREGROUND_LOST_MACOS.ends_with("⌘V."));
        // Each build shows its own paste shortcut, never the other platform's.
        if cfg!(target_os = "macos") {
            assert_eq!(m.key, KEY_FOREGROUND_LOST_MACOS);
            assert!(!m.key.contains("Ctrl+V"));
        } else {
            assert_eq!(m.key, KEY_FOREGROUND_LOST_WINDOWS);
            assert!(!m.key.contains('⌘'));
        }
    }

    #[test]
    fn replace_message_names_accessibility_only_for_a_missing_permission() {
        let denied = GateError::Platform(PlatformError::PermissionDenied(
            PermissionKind::Accessibility,
        ));
        assert_eq!(
            replace_message(&denied),
            plain(KEY_REPLACE_NEEDS_ACCESSIBILITY)
        );
        // Reading the selection keeps its own wording.
        assert_eq!(gate_message(&denied), plain(KEY_PERMISSION_ACCESSIBILITY));
        assert_ne!(
            KEY_REPLACE_NEEDS_ACCESSIBILITY,
            KEY_PERMISSION_ACCESSIBILITY
        );
        // Everything else is the ordinary gate message.
        for e in [
            GateError::Busy,
            GateError::TimedOut,
            GateError::Panicked,
            GateError::Platform(PlatformError::InputBlocked),
            GateError::Platform(PlatformError::PermissionDenied(
                PermissionKind::ScreenRecording,
            )),
        ] {
            assert_eq!(replace_message(&e), gate_message(&e));
        }
    }

    #[test]
    fn platform_message_ocr_unavailable() {
        assert_eq!(
            platform_message(&PlatformError::OcrUnavailable),
            plain(KEY_OCR_UNAVAILABLE)
        );
    }

    #[test]
    fn platform_message_other() {
        let e = PlatformError::Os {
            api: "SendInput",
            code: 5,
        };
        let m = platform_message(&e);
        assert_eq!(m.key, KEY_PLATFORM_GENERIC);
        assert_eq!(m.args, vec!["SendInput failed (code 5)".to_string()]);
        assert_eq!(
            platform_message(&PlatformError::ClipboardBusy),
            plain(KEY_CLIPBOARD_TIMEOUT)
        );
        assert_eq!(
            platform_message(&PlatformError::InvalidRegion),
            selection_too_small()
        );
        assert_eq!(
            platform_message(&PlatformError::HotkeyConflict).key,
            KEY_PLATFORM_GENERIC
        );
    }

    #[test]
    fn screen_gate_message_uses_the_screen_timeout() {
        assert_eq!(
            screen_gate_message(&GateError::Busy),
            plain(KEY_SCREEN_TIMEOUT)
        );
        assert_eq!(
            screen_gate_message(&GateError::TimedOut),
            plain(KEY_SCREEN_TIMEOUT)
        );
        assert_eq!(
            screen_gate_message(&GateError::Platform(PlatformError::InputBlocked)),
            plain(KEY_INPUT_BLOCKED)
        );
        assert_eq!(
            screen_gate_message(&GateError::Panicked).key,
            KEY_PLATFORM_GENERIC
        );
    }

    #[test]
    fn gate_message_busy_and_timed_out() {
        assert_eq!(gate_message(&GateError::Busy), plain(KEY_CLIPBOARD_TIMEOUT));
        assert_eq!(
            gate_message(&GateError::TimedOut),
            plain(KEY_CLIPBOARD_TIMEOUT)
        );
        assert_eq!(
            gate_message(&GateError::Platform(PlatformError::InputBlocked)),
            plain(KEY_INPUT_BLOCKED)
        );
        assert_eq!(gate_message(&GateError::Panicked).key, KEY_PLATFORM_GENERIC);
    }

    #[test]
    fn constructors_carry_only_names_and_counts() {
        assert_eq!(max_input(5000, 4000).args, vec!["5000", "4000"]);
        assert_eq!(
            missing_key(ProviderType::GoogleFree).args,
            vec!["Google (Free)"]
        );
        assert_eq!(missing_key(ProviderType::Claude).key, KEY_MISSING_KEY);
        assert_eq!(no_text_found(true).key, KEY_NO_TEXT_FOUND_RETRY);
        assert_eq!(no_text_found(false).key, KEY_NO_TEXT_FOUND);
        assert!(nothing_selected().args.is_empty());
    }

    #[test]
    fn messages_never_contain_selected_text() {
        // Every message is built from a closed set of keys and names, counts or error Display
        // texts: none of the constructors takes user text.
        let secret = "TOP-SECRET-SELECTION";
        let all = [
            nothing_selected(),
            max_input(secret.len(), 10),
            missing_key(ProviderType::Claude),
            no_text_found(true),
            selection_too_small(),
            platform_message(&PlatformError::Os { api: "X", code: 1 }),
            gate_message(&GateError::Busy),
        ];
        for m in all {
            assert!(!m.key.contains(secret));
            assert!(m.args.iter().all(|a| !a.contains(secret)));
        }
    }

    #[test]
    fn translation_error_view_maps_summary_and_detail() {
        let (s, d) = translation_error_view(&TranslationError::MissingApiKey);
        assert_eq!(
            s.key,
            "API key is not set. Open Settings → Providers and add one."
        );
        assert!(d.is_empty());
    }
}
