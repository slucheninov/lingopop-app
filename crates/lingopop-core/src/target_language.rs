//! Target-language resolution (port of `SettingsStore+TargetLanguage.swift` plus the
//! operation gate of `AppDelegate.handleHotkey`).

use crate::detector;
use crate::operation::AIOperation;
use crate::settings::Settings;

/// Picks the target language for `detected_source`: with auto-detect on and a source given,
/// looks the source up in `language_pairs`; falls back to `target_language` otherwise.
/// A pair value is returned as is, even when it is not in the language table.
pub fn resolve_target_language(settings: &Settings, detected_source: Option<&str>) -> String {
    if settings.use_auto_detect {
        if let Some(mapped) = detected_source.and_then(|source| settings.language_pairs.get(source))
        {
            return mapped.clone();
        }
    }
    settings.target_language.clone()
}

/// Whether the hotkey flow runs language detection before resolving the target language.
/// Only Translate does. Screen Translate is deliberately not gated here: its flow detects
/// on the recognized OCR text itself (`detector::detect` + `resolve_target_language`).
pub fn operation_uses_auto_detect(operation: AIOperation) -> bool {
    operation == AIOperation::Translate
}

/// The target language for `operation` applied to `text`: detection plus pair lookup for
/// operations that use auto-detect, the plain `target_language` for all others.
pub fn target_language_for(settings: &Settings, operation: AIOperation, text: &str) -> String {
    if operation_uses_auto_detect(operation) {
        resolve_target_language(settings, detector::detect(text))
    } else {
        settings.target_language.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::AIOperation;
    use crate::settings::Settings;

    fn settings(auto_detect: bool) -> Settings {
        Settings {
            use_auto_detect: auto_detect,
            target_language: "German".to_string(),
            ..Settings::default()
        }
    }

    #[test]
    fn auto_detect_off_returns_target_language() {
        assert_eq!(
            resolve_target_language(&settings(false), Some("Russian")),
            "German"
        );
    }

    #[test]
    fn auto_detect_on_without_source_returns_target() {
        assert_eq!(resolve_target_language(&settings(true), None), "German");
    }

    #[test]
    fn known_source_uses_pair() {
        assert_eq!(
            resolve_target_language(&settings(true), Some("Russian")),
            "Ukrainian"
        );
    }

    #[test]
    fn unmapped_source_falls_back_to_target() {
        assert_eq!(
            resolve_target_language(&settings(true), Some("Polish")),
            "German"
        );
    }

    #[test]
    fn language_pair_pointing_at_a_language_outside_the_table_is_returned_as_is() {
        let mut s = settings(true);
        s.language_pairs
            .insert("Polish".to_string(), "Klingon".to_string());
        assert_eq!(resolve_target_language(&s, Some("Polish")), "Klingon");
    }

    #[test]
    fn operation_uses_auto_detect_only_for_translate() {
        assert!(operation_uses_auto_detect(AIOperation::Translate));
        for op in [
            AIOperation::ScreenTranslate,
            AIOperation::GrammarFix,
            AIOperation::Rewrite,
            AIOperation::Explain,
            AIOperation::Magic,
        ] {
            assert!(!operation_uses_auto_detect(op), "{op:?}");
        }
    }

    #[test]
    fn target_language_for_uses_detector_only_for_translate() {
        let s = settings(true);
        let russian =
            "Это достаточно длинный русский текст, написанный для проверки определения языка.";
        assert_eq!(
            target_language_for(&s, AIOperation::Translate, russian),
            "Ukrainian"
        );
        for op in [
            AIOperation::ScreenTranslate,
            AIOperation::GrammarFix,
            AIOperation::Rewrite,
            AIOperation::Explain,
            AIOperation::Magic,
        ] {
            assert_eq!(target_language_for(&s, op, russian), "German", "{op:?}");
        }
    }

    #[test]
    fn target_language_for_translate_with_undetectable_text_uses_target() {
        assert_eq!(
            target_language_for(&settings(true), AIOperation::Translate, "12345"),
            "German"
        );
    }
}
