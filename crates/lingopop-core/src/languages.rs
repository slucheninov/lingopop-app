//! The language table: the single source of truth for the quick-pick list, every
//! provider's API code and the detector mapping. Ported from the macOS app
//! (`Models/Languages.swift`, `Models/LanguageCodes.swift`); names, codes and the
//! quick-list order are persisted or sent to APIs, so do not "fix" them.

/// One supported language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageInfo {
    /// English display name; also the value stored in settings and sent in prompts.
    pub name: &'static str,
    /// ISO 639-1 code used by Google Free / Google Cloud (and the base for the others).
    pub code: &'static str,
    /// DeepL code when it differs from `code.to_uppercase()`.
    pub deepl_code: Option<&'static str>,
    /// Microsoft Translator code when it differs from `code`.
    pub microsoft_code: Option<&'static str>,
    /// ISO 639-3 codes the detector maps to this language (replaces Swift's `nlLanguages`).
    pub detect_codes: &'static [&'static str],
    /// Whether the language appears in the quick-pick lists.
    pub in_quick_list: bool,
}

const fn lang(
    name: &'static str,
    code: &'static str,
    deepl_code: Option<&'static str>,
    microsoft_code: Option<&'static str>,
    detect_codes: &'static [&'static str],
    in_quick_list: bool,
) -> LanguageInfo {
    LanguageInfo {
        name,
        code,
        deepl_code,
        microsoft_code,
        detect_codes,
        in_quick_list,
    }
}

/// The language table. The order of the `in_quick_list` entries is the UI order.
pub const ALL: &[LanguageInfo] = &[
    lang("Ukrainian", "uk", None, None, &["ukr"], true),
    lang("English", "en", None, None, &["eng"], true),
    lang("Russian", "ru", None, None, &["rus"], true),
    lang("German", "de", None, None, &["deu"], true),
    lang("Spanish", "es", None, None, &["spa"], true),
    lang("French", "fr", None, None, &["fra"], true),
    lang("Italian", "it", None, None, &["ita"], true),
    lang("Polish", "pl", None, None, &["pol"], true),
    lang("Portuguese", "pt", None, None, &["por"], true),
    lang("Chinese", "zh", None, Some("zh-Hans"), &["cmn"], true),
    lang("Japanese", "ja", None, None, &["jpn"], true),
    lang("Korean", "ko", None, None, &["kor"], true),
    lang("Turkish", "tr", None, None, &["tur"], true),
    lang("Arabic", "ar", None, None, &["ara"], true),
    lang("Dutch", "nl", None, None, &["nld"], false),
    lang("Czech", "cs", None, None, &["ces"], false),
    lang("Danish", "da", None, None, &["dan"], false),
    lang("Finnish", "fi", None, None, &["fin"], false),
    lang("Greek", "el", None, None, &["ell"], false),
    lang("Hebrew", "he", None, None, &["heb"], false),
    lang("Hindi", "hi", None, None, &["hin"], false),
    lang("Hungarian", "hu", None, None, &["hun"], false),
    lang("Indonesian", "id", None, None, &["ind"], false),
    lang(
        "Norwegian",
        "no",
        Some("NB"),
        Some("nb"),
        &["nob", "nno"],
        false,
    ),
    lang("Romanian", "ro", None, None, &["ron"], false),
    lang("Swedish", "sv", None, None, &["swe"], false),
    lang("Thai", "th", None, None, &["tha"], false),
    lang("Vietnamese", "vi", None, None, &["vie"], false),
];

/// The table row for an English language name, or `None` for custom names.
pub fn info(name: &str) -> Option<&'static LanguageInfo> {
    ALL.iter().find(|l| l.name == name)
}

/// Names shown in the language pickers, in UI order.
pub fn quick_list() -> Vec<&'static str> {
    ALL.iter()
        .filter(|l| l.in_quick_list)
        .map(|l| l.name)
        .collect()
}

/// The display name for an ISO 639-3 code returned by the detector, if supported.
pub fn name_for_detect_code(code: &str) -> Option<&'static str> {
    ALL.iter()
        .find(|l| l.detect_codes.contains(&code))
        .map(|l| l.name)
}

/// ISO 639-1 code for a language name; unknown (custom) names fall back to the lowercased name.
pub fn code(name: &str) -> String {
    info(name).map_or_else(|| name.to_lowercase(), |l| l.code.to_string())
}

/// DeepL expects uppercase codes and `NB` for Norwegian.
pub fn deepl_code(name: &str) -> String {
    info(name)
        .and_then(|l| l.deepl_code)
        .map_or_else(|| code(name).to_uppercase(), str::to_string)
}

/// Microsoft Translator expects `nb` for Norwegian and `zh-Hans` for (simplified) Chinese.
pub fn microsoft_code(name: &str) -> String {
    info(name)
        .and_then(|l| l.microsoft_code)
        .map_or_else(|| code(name), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn code_table() {
        for (n, c, d, m) in [
            ("Ukrainian", "uk", "UK", "uk"),
            ("Chinese", "zh", "ZH", "zh-Hans"),
            ("Norwegian", "no", "NB", "nb"),
            ("Klingon", "klingon", "KLINGON", "klingon"),
        ] {
            assert_eq!(code(n), c, "code({n})");
            assert_eq!(deepl_code(n), d, "deepl_code({n})");
            assert_eq!(microsoft_code(n), m, "microsoft_code({n})");
        }
    }

    #[test]
    fn quick_list_has_14_entries_ukrainian_first() {
        assert_eq!(quick_list().len(), 14);
        assert_eq!(
            quick_list(),
            vec![
                "Ukrainian",
                "English",
                "Russian",
                "German",
                "Spanish",
                "French",
                "Italian",
                "Polish",
                "Portuguese",
                "Chinese",
                "Japanese",
                "Korean",
                "Turkish",
                "Arabic",
            ]
        );
    }

    #[test]
    fn table_has_28_languages_with_unique_names_and_codes() {
        assert_eq!(ALL.len(), 28);
        let names: HashSet<_> = ALL.iter().map(|l| l.name).collect();
        let codes: HashSet<_> = ALL.iter().map(|l| l.code).collect();
        assert_eq!(names.len(), 28);
        assert_eq!(codes.len(), 28);
    }

    #[test]
    fn info_finds_known_names_only() {
        assert_eq!(info("Polish").map(|l| l.code), Some("pl"));
        assert!(info("Klingon").is_none());
        assert!(info("polish").is_none());
    }

    #[test]
    fn name_for_detect_code_maps_iso_639_3() {
        assert_eq!(name_for_detect_code("cmn"), Some("Chinese"));
        assert_eq!(name_for_detect_code("ukr"), Some("Ukrainian"));
        assert_eq!(name_for_detect_code("nob"), Some("Norwegian"));
        assert_eq!(name_for_detect_code("nno"), Some("Norwegian"));
        assert_eq!(name_for_detect_code("xxx"), None);
    }

    #[test]
    fn every_detect_code_is_unique_across_the_table() {
        let all: Vec<_> = ALL.iter().flat_map(|l| l.detect_codes.iter()).collect();
        let unique: HashSet<_> = all.iter().collect();
        assert_eq!(all.len(), unique.len());
        assert_eq!(all.len(), 29);
        assert!(ALL.iter().all(|l| !l.detect_codes.is_empty()));
    }
}
