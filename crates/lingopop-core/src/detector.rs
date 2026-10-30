//! Source-language detection with `whatlang` (pure Rust, no model files, deterministic).

use crate::languages;

/// The English language name (from the `languages` table) of whatlang's best guess for
/// `text`, or `None` when whatlang has no guess (empty, whitespace, digits or punctuation
/// only) or the guess is not in the table.
///
/// Deliberately not gated on `Info::is_reliable()`: the mac app uses
/// `NLLanguageRecognizer.dominantLanguage`, which always returns its best guess, even for a
/// short text, and the popup relies on that (a one-word selection still gets its language
/// pair). Callers that need certainty must not rely on short inputs.
pub fn detect(text: &str) -> Option<&'static str> {
    let info = whatlang::detect(text)?;
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
        // Real whatlang 0.18 outputs, recorded at implementation time (best guesses, not necessarily right: "hello" is Danish). A detector upgrade that
        // changes them shows up here.
        for (text, expected) in [
            ("ok", Some("Hungarian")),
            ("a", Some("Hungarian")),
            ("?!", None),
            ("Привет", Some("Russian")),
            ("hello", Some("Danish")),
        ] {
            assert_eq!(detect(text), expected, "{text:?}");
        }
    }
}
