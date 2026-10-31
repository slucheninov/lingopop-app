//! User-visible messages of the flow as catalog keys (the mac English source strings) plus
//! arguments. Arguments are only names, counts and platform error `Display` texts, never the
//! user's text.

use lingopop_core::errors::TranslationError;
use lingopop_core::provider::ProviderType;
use lingopop_platform::PlatformError;

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
/// NEW (Windows only).
pub const KEY_FOREGROUND_LOST: &str = "Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with Ctrl+V.";
/// NEW (Windows only).
pub const KEY_OCR_UNAVAILABLE: &str = "No OCR language is installed. Add a language with Optical character recognition in Settings → Time & language → Language & region, or use Retry to send the image to AI.";
/// NEW (Windows only).
pub const KEY_PLATFORM_GENERIC: &str = "Something went wrong on this PC (%@). Try again.";
/// NEW (Windows only).
pub const KEY_SCREEN_TIMEOUT: &str = "Screen capture or text recognition took too long. Try again.";

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

/// `Unsupported`, `Os`, `HotkeyConflict` and `InvalidHotkey` use the generic key with the
/// error's `Display` (no payload by construction); `InvalidRegion` means the selection is too small.
pub fn platform_message(e: &PlatformError) -> UserMessageView {
    match e {
        PlatformError::ClipboardBusy => plain(KEY_CLIPBOARD_TIMEOUT),
        PlatformError::InputBlocked => plain(KEY_INPUT_BLOCKED),
        PlatformError::OcrUnavailable => plain(KEY_OCR_UNAVAILABLE),
        PlatformError::InvalidRegion => selection_too_small(),
        PlatformError::Unsupported(_)
        | PlatformError::HotkeyConflict
        | PlatformError::InvalidHotkey
        | PlatformError::Os { .. } => {
            UserMessageView::new(KEY_PLATFORM_GENERIC, vec![e.to_string()])
        }
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
    fn platform_message_foreground_lost() {
        let m = plain(KEY_FOREGROUND_LOST);
        assert!(m.key.contains("Ctrl+V") && m.args.is_empty());
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
