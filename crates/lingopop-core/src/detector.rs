//! Source-language detection with `whatlang` (pure Rust, no model files, deterministic).

use crate::languages;

/// The English language name (from the `languages` table) of `text`, or `None` when the
/// detector is not confident (short or ambiguous text, digits only, empty) or the detected
/// language is not in the table. Callers then fall back to `target_language`.
pub fn detect(text: &str) -> Option<&'static str> {
    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }
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
    fn short_ambiguous_text_results_are_pinned() {
        // Observed from whatlang 0.18 at implementation time; `is_reliable()` is false
        // for each, so the caller falls back to `target_language`. A detector upgrade
        // that changes this shows up here.
        assert_eq!(detect("ok"), None);
        assert_eq!(detect("a"), None);
        assert_eq!(detect("?!"), None);
    }
}
