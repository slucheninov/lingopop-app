//! UI text catalogs. English catalog keys are their own text; Polish, Ukrainian and German come
//! from the catalogs this repository owns: the shared `i18n/catalogs/{pl,uk,de}.json` (keys shown
//! on both platforms) plus the platform file of the running OS, `i18n/windows.json` or
//! `i18n/macos.json` (`scripts/gen-localizations.py check` validates all of them).
//! The catalogs are embedded and parsed once per platform and language into owned strings (a JSON
//! string with escapes such as `\n` or `\"` cannot be borrowed from the source text).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::Language;

const SHARED_PL: &str = include_str!("../../../i18n/catalogs/pl.json");
const SHARED_UK: &str = include_str!("../../../i18n/catalogs/uk.json");
const SHARED_DE: &str = include_str!("../../../i18n/catalogs/de.json");
const WINDOWS: &str = include_str!("../../../i18n/windows.json");
const MACOS: &str = include_str!("../../../i18n/macos.json");

/// Which platform file is layered over the shared catalogs. The caller injects it (the shell
/// picks it with `cfg(target_os)`), the core never inspects the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogPlatform {
    Windows,
    Macos,
}

impl CatalogPlatform {
    fn source(self) -> &'static str {
        match self {
            CatalogPlatform::Windows => WINDOWS,
            CatalogPlatform::Macos => MACOS,
        }
    }
}

fn build(platform: CatalogPlatform, language: Language, shared: &str) -> HashMap<String, String> {
    let mut merged: HashMap<String, String> = serde_json::from_str(shared).unwrap_or_default();
    let entries: HashMap<String, HashMap<String, String>> =
        serde_json::from_str(platform.source()).unwrap_or_default();
    for (key, mut values) in entries {
        if let Some(value) = values.remove(language.code()) {
            merged.entry(key).or_insert(value);
        }
    }
    merged
}

/// The merged shared + platform catalog of a language; empty for English (the key is the text).
/// The shell ships it to the webview.
pub fn catalog_for(
    platform: CatalogPlatform,
    language: Language,
) -> &'static HashMap<String, String> {
    type Cell = OnceLock<HashMap<String, String>>;
    static EMPTY: Cell = OnceLock::new();
    static WINDOWS_PL: Cell = OnceLock::new();
    static WINDOWS_UK: Cell = OnceLock::new();
    static WINDOWS_DE: Cell = OnceLock::new();
    static MACOS_PL: Cell = OnceLock::new();
    static MACOS_UK: Cell = OnceLock::new();
    static MACOS_DE: Cell = OnceLock::new();
    let (cell, shared) = match (platform, language) {
        (_, Language::En) => return EMPTY.get_or_init(HashMap::new),
        (CatalogPlatform::Windows, Language::Pl) => (&WINDOWS_PL, SHARED_PL),
        (CatalogPlatform::Windows, Language::Uk) => (&WINDOWS_UK, SHARED_UK),
        (CatalogPlatform::Windows, Language::De) => (&WINDOWS_DE, SHARED_DE),
        (CatalogPlatform::Macos, Language::Pl) => (&MACOS_PL, SHARED_PL),
        (CatalogPlatform::Macos, Language::Uk) => (&MACOS_UK, SHARED_UK),
        (CatalogPlatform::Macos, Language::De) => (&MACOS_DE, SHARED_DE),
    };
    cell.get_or_init(|| build(platform, language, shared))
}

/// The template for `key` in `language` on `platform`: the catalog entry, else the key itself.
pub fn lookup_for<'a>(platform: CatalogPlatform, language: Language, key: &'a str) -> Cow<'a, str> {
    match catalog_for(platform, language).get(key) {
        Some(value) => Cow::Borrowed(value.as_str()),
        None => Cow::Borrowed(key),
    }
}

/// `catalog_for` on Windows (kept for callers that do not inject a platform yet).
pub fn catalog(language: Language) -> &'static HashMap<String, String> {
    catalog_for(CatalogPlatform::Windows, language)
}

/// `lookup_for` on Windows.
pub fn lookup<'a>(language: Language, key: &'a str) -> Cow<'a, str> {
    lookup_for(CatalogPlatform::Windows, language, key)
}

/// Substitutes `args` into a template: ordered `%@` / `%lld` / `%ld` / `%d`, positional
/// `%1$@`, and `%%` for a literal percent. A placeholder without an argument stays as written.
pub fn format(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut ordered = 0usize;
    let mut rest = template;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match placeholder(tail) {
            Some((Placeholder::Percent, len)) => {
                out.push('%');
                rest = &tail[len..];
            }
            Some((Placeholder::Arg(position), len)) => {
                let index = position.unwrap_or_else(|| {
                    ordered += 1;
                    ordered
                });
                match args.get(index - 1) {
                    Some(arg) => out.push_str(arg),
                    None => out.push_str(&tail[..len]),
                }
                rest = &tail[len..];
            }
            None => {
                out.push('%');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

enum Placeholder {
    Percent,
    /// `Some(n)` = 1-based positional (`%n$@`), `None` = the next ordered argument.
    Arg(Option<usize>),
}

/// Parses the placeholder at the start of `s` (which starts with `%`): the variant and its length.
fn placeholder(s: &str) -> Option<(Placeholder, usize)> {
    let body = &s[1..];
    if body.starts_with('%') {
        return Some((Placeholder::Percent, 2));
    }
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let (position, after) = if digits > 0 && body[digits..].starts_with('$') {
        // `%0$@` or an overflowing number is not a placeholder.
        let n = body[..digits].parse::<usize>().ok().filter(|n| *n > 0)?;
        (Some(n), 1 + digits + 1)
    } else {
        (None, 1)
    };
    let spec = &s[after..];
    ["@", "lld", "ld", "d"]
        .iter()
        .find(|candidate| spec.starts_with(**candidate))
        .map(|candidate| (Placeholder::Arg(position), after + candidate.len()))
}

/// `lookup_for` followed by `format`.
pub fn t_for(platform: CatalogPlatform, language: Language, key: &str, args: &[&str]) -> String {
    format(&lookup_for(platform, language, key), args)
}

/// `t_for` on Windows.
pub fn t(language: Language, key: &str, args: &[&str]) -> String {
    t_for(CatalogPlatform::Windows, language, key, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Language;
    use std::borrow::Cow;
    use std::collections::HashMap;

    const PL: &str = include_str!("../../../i18n/catalogs/pl.json");
    const BEFORE_PL: &str = include_str!("../tests/fixtures/i18n/windows_merged_pl.json");
    const BEFORE_UK: &str = include_str!("../tests/fixtures/i18n/windows_merged_uk.json");
    const BEFORE_DE: &str = include_str!("../tests/fixtures/i18n/windows_merged_de.json");

    const LANGUAGES: [Language; 3] = [Language::Pl, Language::Uk, Language::De];
    const ACCESSIBILITY: &str = "Accessibility access required";
    const COMMAND_V_HINT: &str = "Couldn't switch back to the original app, so nothing was replaced. The text is on your clipboard; paste it manually with ⌘V.";

    const INPUT_BLOCKED: &str = "LingoPop can't reach an app running as administrator. Run LingoPop as administrator, or copy and paste the text manually.";
    const ESCAPED_KEY: &str = "Line one\nLine \"two\"";

    #[test]
    fn english_is_pass_through() {
        assert_eq!(lookup(Language::En, "Quit LingoPop"), "Quit LingoPop");
        assert!(catalog(Language::En).is_empty());
    }

    #[test]
    fn polish_lookup_from_shared() {
        let shared: HashMap<String, String> = serde_json::from_str(PL).unwrap();
        let expected = &shared["Quit LingoPop"];
        assert_ne!(expected, "Quit LingoPop");
        assert_eq!(lookup(Language::Pl, "Quit LingoPop"), expected.as_str());
    }

    #[test]
    fn windows_only_key_is_looked_up() {
        for language in [Language::Pl, Language::Uk, Language::De] {
            assert_ne!(
                lookup(language, INPUT_BLOCKED),
                INPUT_BLOCKED,
                "{language:?}"
            );
        }
    }

    #[test]
    fn escaped_key_and_value_round_trip() {
        let pl = lookup(Language::Pl, ESCAPED_KEY);
        assert_ne!(pl, ESCAPED_KEY);
        assert!(pl.contains('\n'), "{pl:?}");
        assert!(pl.contains('"'), "{pl:?}");
        for language in [Language::Uk, Language::De] {
            let value = lookup(language, ESCAPED_KEY);
            assert!(value.contains('\n') && value.contains('"'), "{value:?}");
        }
    }

    #[test]
    fn unknown_key_falls_back_to_key() {
        let key = String::from("No such key anywhere");
        let value = lookup(Language::De, &key);
        assert_eq!(value, "No such key anywhere");
        assert!(matches!(value, Cow::Borrowed(borrowed) if std::ptr::eq(borrowed, key.as_str())));
    }

    #[test]
    fn catalog_merges_shared_and_windows_entries() {
        let pl = catalog(Language::Pl);
        assert!(pl.contains_key("Quit LingoPop"));
        assert!(pl.contains_key(INPUT_BLOCKED));
        assert!(!pl.contains_key("Accessibility"));
    }

    #[test]
    fn format_ordered_positional_and_percent() {
        assert_eq!(
            format("Text is %@ characters, max is %@.", &["3", "2"]),
            "Text is 3 characters, max is 2."
        );
        assert_eq!(format("%2$@ %1$@", &["a", "b"]), "b a");
        assert_eq!(format("100%%", &[]), "100%");
        assert_eq!(format("Used %lld times", &["4"]), "Used 4 times");
        assert_eq!(format("Missing %@ and %3$@", &["x"]), "Missing x and %3$@");
        assert_eq!(format("Dangling %", &[]), "Dangling %");
        assert_eq!(
            format("%0$@ %99999999999999999999$@ %5", &["x"]),
            "%0$@ %99999999999999999999$@ %5"
        );
    }

    #[test]
    fn t_looks_up_then_formats() {
        assert_eq!(
            t(Language::En, "Retry with %@", &["Claude"]),
            "Retry with Claude"
        );
        let pl = t(Language::Pl, "Retry with %@", &["Claude"]);
        assert!(pl.contains("Claude") && !pl.contains("%@"), "{pl}");
    }

    #[test]
    fn windows_resolves_shared_and_windows_but_not_mac_only_keys() {
        for language in LANGUAGES {
            let platform = CatalogPlatform::Windows;
            assert_ne!(
                lookup_for(platform, language, "Quit LingoPop"),
                "Quit LingoPop"
            );
            assert_ne!(lookup_for(platform, language, INPUT_BLOCKED), INPUT_BLOCKED);
            assert_eq!(lookup_for(platform, language, ACCESSIBILITY), ACCESSIBILITY);
            assert_eq!(
                lookup_for(platform, language, COMMAND_V_HINT),
                COMMAND_V_HINT
            );
            assert!(!catalog_for(platform, language).contains_key(ACCESSIBILITY));
        }
    }

    #[test]
    fn macos_resolves_shared_and_macos_but_not_windows_only_keys() {
        for language in LANGUAGES {
            let platform = CatalogPlatform::Macos;
            assert_ne!(
                lookup_for(platform, language, "Quit LingoPop"),
                "Quit LingoPop"
            );
            assert_ne!(lookup_for(platform, language, ACCESSIBILITY), ACCESSIBILITY);
            let hint = lookup_for(platform, language, COMMAND_V_HINT);
            assert_ne!(hint, COMMAND_V_HINT);
            assert!(hint.contains('⌘'), "{language:?}: {hint}");
            assert_eq!(lookup_for(platform, language, INPUT_BLOCKED), INPUT_BLOCKED);
            assert!(!catalog_for(platform, language).contains_key(INPUT_BLOCKED));
        }
    }

    #[test]
    fn english_returns_the_key_on_every_platform() {
        for platform in [CatalogPlatform::Windows, CatalogPlatform::Macos] {
            assert_eq!(
                lookup_for(platform, Language::En, ACCESSIBILITY),
                ACCESSIBILITY
            );
            assert!(catalog_for(platform, Language::En).is_empty());
            assert_eq!(
                t_for(platform, Language::En, "Retry with %@", &["Claude"]),
                "Retry with Claude"
            );
        }
    }

    #[test]
    fn t_for_formats_the_platform_template() {
        let steps = t_for(
            CatalogPlatform::Macos,
            Language::Pl,
            "Remove %@",
            &["DeepL"],
        );
        assert!(steps.contains("DeepL") && !steps.contains("%@"), "{steps}");
    }

    #[test]
    fn windows_wrappers_equal_the_windows_platform() {
        for language in LANGUAGES {
            assert_eq!(
                catalog(language),
                catalog_for(CatalogPlatform::Windows, language)
            );
            assert_eq!(
                lookup(language, INPUT_BLOCKED),
                lookup_for(CatalogPlatform::Windows, language, INPUT_BLOCKED)
            );
            assert_eq!(
                t(language, "Remove %@", &["x"]),
                t_for(CatalogPlatform::Windows, language, "Remove %@", &["x"])
            );
        }
    }

    #[test]
    fn windows_merged_catalog_is_semantically_unchanged_by_the_migration() {
        // The snapshots are the merged generated + windows catalogs captured before the migration.
        for (language, before) in [
            (Language::Pl, BEFORE_PL),
            (Language::Uk, BEFORE_UK),
            (Language::De, BEFORE_DE),
        ] {
            let before: HashMap<String, String> = serde_json::from_str(before).unwrap();
            let after = catalog_for(CatalogPlatform::Windows, language);
            for (key, value) in &before {
                assert_eq!(after.get(key), Some(value), "{language:?}: {key}");
            }
        }
    }

    #[test]
    fn no_windows_value_contains_a_command_symbol() {
        for language in LANGUAGES {
            for (key, value) in catalog_for(CatalogPlatform::Windows, language) {
                assert!(!value.contains('⌘'), "{language:?}: {key}");
            }
        }
    }
}
