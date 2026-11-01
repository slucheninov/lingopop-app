//! UI text catalogs. English catalog keys are their own text; Polish, Ukrainian and German come
//! from `i18n/generated/*.json` (generated from the mac `Localizable.xcstrings` by
//! `scripts/gen-localizations.py`) plus the Windows-only entries of `i18n/windows.json`.
//! The catalogs are embedded and parsed once per language into owned strings (a JSON string with
//! escapes such as `\n` or `\"` cannot be borrowed from the source text).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::Language;

const GENERATED_PL: &str = include_str!("../../../i18n/generated/pl.json");
const GENERATED_UK: &str = include_str!("../../../i18n/generated/uk.json");
const GENERATED_DE: &str = include_str!("../../../i18n/generated/de.json");
const WINDOWS: &str = include_str!("../../../i18n/windows.json");

fn build(language: Language, generated: &str) -> HashMap<String, String> {
    let mut merged: HashMap<String, String> = serde_json::from_str(generated).unwrap_or_default();
    let windows: HashMap<String, HashMap<String, String>> =
        serde_json::from_str(WINDOWS).unwrap_or_default();
    for (key, mut values) in windows {
        if let Some(value) = values.remove(language.code()) {
            merged.entry(key).or_insert(value);
        }
    }
    merged
}

/// The merged generated + Windows-only catalog of a language; empty for English (the key is the
/// text). The shell ships it to the webview.
pub fn catalog(language: Language) -> &'static HashMap<String, String> {
    static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
    static PL: OnceLock<HashMap<String, String>> = OnceLock::new();
    static UK: OnceLock<HashMap<String, String>> = OnceLock::new();
    static DE: OnceLock<HashMap<String, String>> = OnceLock::new();
    match language {
        Language::En => EMPTY.get_or_init(HashMap::new),
        Language::Pl => PL.get_or_init(|| build(Language::Pl, GENERATED_PL)),
        Language::Uk => UK.get_or_init(|| build(Language::Uk, GENERATED_UK)),
        Language::De => DE.get_or_init(|| build(Language::De, GENERATED_DE)),
    }
}

/// The template for `key` in `language`: the catalog entry, else the key itself.
pub fn lookup<'a>(language: Language, key: &'a str) -> Cow<'a, str> {
    match catalog(language).get(key) {
        Some(value) => Cow::Borrowed(value.as_str()),
        None => Cow::Borrowed(key),
    }
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

/// `lookup` followed by `format`.
pub fn t(language: Language, key: &str, args: &[&str]) -> String {
    format(&lookup(language, key), args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Language;
    use std::borrow::Cow;
    use std::collections::HashMap;

    const PL: &str = include_str!("../../../i18n/generated/pl.json");

    const INPUT_BLOCKED: &str = "LingoPop can't reach an app running as administrator. Run LingoPop as administrator, or copy and paste the text manually.";
    const ESCAPED_KEY: &str = "Line one\nLine \"two\"";

    #[test]
    fn english_is_pass_through() {
        assert_eq!(lookup(Language::En, "Quit LingoPop"), "Quit LingoPop");
        assert!(catalog(Language::En).is_empty());
    }

    #[test]
    fn polish_lookup_from_generated() {
        let generated: HashMap<String, String> = serde_json::from_str(PL).unwrap();
        let expected = &generated["Quit LingoPop"];
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
    fn catalog_merges_generated_and_windows_entries() {
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
}
