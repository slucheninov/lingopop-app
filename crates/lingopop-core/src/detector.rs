//! Source-language detection with `whatlang` (pure Rust, no model files, deterministic).

use std::sync::OnceLock;

use whatlang::{Detector, Lang};

use crate::languages;

/// A detector restricted to the languages of the `languages::ALL` table (built once).
/// Without the restriction a short text lands on a language the app does not offer.
fn detector() -> &'static Detector {
    static DETECTOR: OnceLock<Detector> = OnceLock::new();
    DETECTOR.get_or_init(|| Detector::with_allowlist(allowlist()))
}

/// whatlang languages for every detect code in the table; a code whatlang does not
/// know (it has no separate Nynorsk) is skipped.
fn allowlist() -> Vec<Lang> {
    languages::ALL
        .iter()
        .flat_map(|l| l.detect_codes.iter())
        .filter_map(|code| Lang::from_code(*code))
        .collect()
}

/// The English language name (from the `languages` table) of whatlang's best guess for
/// `text`, or `None` when whatlang has no guess (empty, whitespace, digits or punctuation
/// only) or the guess is not in the table.
///
/// Only languages of the table are candidates (`whatlang::Detector::with_allowlist`).
///
/// Deliberately not gated on `Info::is_reliable()`: the mac app uses
/// `NLLanguageRecognizer.dominantLanguage`, which always returns its best guess, even for a
/// short text, and the popup relies on that (a one-word selection still gets its language
/// pair). Callers that need certainty must not rely on short inputs.
pub fn detect(text: &str) -> Option<&'static str> {
    let info = detector().detect(text)?;
    languages::name_for_detect_code(info.lang().code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ukrainian() {
        assert_eq!(
            detect("Це досить довгий український текст, написаний для перевірки визначення мови."),
            Some("Ukrainian")
        );
    }

    #[test]
    fn detects_russian() {
        assert_eq!(
            detect(
                "Это достаточно длинный русский текст, написанный для проверки определения языка."
            ),
            Some("Russian")
        );
    }

    #[test]
    fn detects_english() {
        assert_eq!(
            detect(
                "This is a reasonably long English sentence written to check language detection."
            ),
            Some("English")
        );
    }

    #[test]
    fn detects_german() {
        assert_eq!(
            detect("Dies ist ein ausreichend langer deutscher Satz, der zur Prüfung der Spracherkennung dient."),
            Some("German")
        );
    }

    #[test]
    fn detects_polish() {
        assert_eq!(
            detect("To jest wystarczająco długie polskie zdanie napisane w celu sprawdzenia wykrywania języka."),
            Some("Polish")
        );
    }

    #[test]
    fn detects_japanese_kana() {
        assert_eq!(
            detect("これは言語の検出を確認するために書かれた、十分に長い日本語の文章です。ひらがなとカタカナも含まれています。"),
            Some("Japanese")
        );
    }

    #[test]
    fn detects_korean_hangul() {
        assert_eq!(
            detect("이것은 언어 감지를 확인하기 위해 작성된 충분히 긴 한국어 문장입니다. 한글로만 쓰여 있습니다."),
            Some("Korean")
        );
    }

    #[test]
    fn detects_chinese_han() {
        assert_eq!(
            detect("这是一个为了检查语言检测而写的足够长的中文句子，只包含汉字和常见的标点符号。"),
            Some("Chinese")
        );
    }

    #[test]
    fn digits_and_empty_string_return_none() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("12345"), None);
        assert_eq!(detect("   "), None);
    }

    #[test]
    fn short_text_results_are_pinned() {
        // Real whatlang 0.18 outputs with the table allowlist, recorded at implementation
        // time. They are best guesses, not necessarily right: "hello" is Danish, "good
        // morning" is Swedish and a short Ukrainian phrase without i/ї/є is Russian (without
        // the allowlist "good morning" was Uzbek, which is not in the table, so `None`). A
        // detector upgrade that changes them shows up here.
        for (text, expected) in [
            ("ok", Some("Hungarian")),
            ("a", Some("Hungarian")),
            ("?!", None),
            ("Привет", Some("Russian")),
            ("hello", Some("Danish")),
            ("good morning", Some("Swedish")),
            ("Доброго ранку, друже", Some("Russian")),
            ("Дякую вам", Some("Russian")),
        ] {
            assert_eq!(detect(text), expected, "{text:?}");
        }
    }

    #[test]
    fn allowlist_covers_every_table_language_whatlang_knows() {
        let allowed = allowlist();
        for info in languages::ALL {
            let known = info
                .detect_codes
                .iter()
                .any(|code| Lang::from_code(*code).is_some());
            assert!(known, "{} has no whatlang language", info.name);
        }
        // Norwegian maps to Bokmal only: whatlang has no Nynorsk ("nno").
        let table_codes: usize = languages::ALL.iter().map(|l| l.detect_codes.len()).sum();
        assert_eq!(allowed.len(), table_codes - 1);
    }
}
