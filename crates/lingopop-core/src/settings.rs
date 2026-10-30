//! `settings.json`: the user-facing preferences. Wire names are the persisted
//! `SettingsStore.Persisted` names of the mac app; iCloud fields, provider
//! settings (those live in `secrets.dat`) and the legacy singular `hotkey` are not
//! part of the Windows format. Decoding is lenient per field (RF 2 / RF 3).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::fsio::{atomic_write, is_blank, quarantine, read_optional, strip_bom, LoadOutcome};
use crate::hotkey::Hotkey;
use crate::keyed_map::ProviderKeyedMap;
use crate::operation::AIOperation;
use crate::prompts::DEFAULT_MAGIC_PROMPT;
use crate::provider::ProviderType;
use crate::time::Clock;

/// Maximum length of the Magic prompt, in Unicode scalars.
const MAGIC_PROMPT_MAX_SCALARS: usize = 2000;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("file access failed: {0}")]
    Io(#[from] io::Error),
    #[error("settings are not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings.json does not hold a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub active_provider: ProviderType,
    pub fallbacks: ProviderKeyedMap<ProviderType>,
    pub target_language: String,
    pub hotkeys: BTreeMap<AIOperation, Hotkey>,
    pub launch_at_login: bool,
    /// Longest input sent to a provider, in characters; 0 means no limit.
    pub max_input_chars: u32,
    pub max_history_count: u32,
    pub menu_history_count: u32,
    pub use_auto_detect: bool,
    pub language_pairs: BTreeMap<String, String>,
    pub auto_copy_ocr: bool,
    pub magic_prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_provider: ProviderType::Claude,
            fallbacks: ProviderKeyedMap::default(),
            target_language: "Ukrainian".to_string(),
            // Parity with the mac app, which ships only Translate.
            hotkeys: BTreeMap::from([(AIOperation::Translate, Hotkey::default_translate())]),
            launch_at_login: false,
            max_input_chars: 5000,
            max_history_count: 10,
            menu_history_count: 10,
            use_auto_detect: false,
            language_pairs: [
                ("Russian", "Ukrainian"),
                ("Ukrainian", "Russian"),
                ("English", "Ukrainian"),
            ]
            .into_iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect(),
            auto_copy_ocr: true,
            magic_prompt: DEFAULT_MAGIC_PROMPT.to_string(),
        }
    }
}

/// Serialization mirror with the persisted wire names.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Wire<'a> {
    active_provider: ProviderType,
    fallbacks: &'a ProviderKeyedMap<ProviderType>,
    target_language: &'a str,
    hotkeys: &'a BTreeMap<AIOperation, Hotkey>,
    launch_at_login: bool,
    max_input_chars: u32,
    max_history_count: u32,
    menu_history_count: u32,
    use_auto_detect: bool,
    language_pairs: &'a BTreeMap<String, String>,
    #[serde(rename = "autoCopyOCR")]
    auto_copy_ocr: bool,
    magic_prompt: &'a str,
}

/// Decodes one field; a missing or undecodable one gives `None`.
fn field<T: DeserializeOwned>(object: &Map<String, Value>, name: &str) -> Option<T> {
    object
        .get(name)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Hotkeys by operation raw value; unknown operations and invalid hotkeys are skipped.
fn decode_hotkeys(value: &Value) -> Option<BTreeMap<AIOperation, Hotkey>> {
    let entries = value.as_object()?;
    Some(
        entries
            .iter()
            .filter_map(|(raw, value)| {
                let operation = AIOperation::from_raw_value(raw)?;
                let hotkey: Hotkey = serde_json::from_value(value.clone()).ok()?;
                let hotkey = Hotkey::new(hotkey.key, hotkey.modifiers);
                hotkey.is_valid().then_some((operation, hotkey))
            })
            .collect(),
    )
}

/// Port of `SettingsStore.sanitizeMagicPrompt`: trim Unicode whitespace and
/// newlines, keep `\n` and `\t`, drop every other ASCII character below 0x20
/// (keeps U+007F and all non-ASCII, including the C1 range), cut to 2000 scalars.
pub fn sanitize_magic_prompt(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_ascii() || u32::from(c) >= 0x20)
        .take(MAGIC_PROMPT_MAX_SCALARS)
        .collect()
}

impl Settings {
    /// Lenient decode (a UTF-8 BOM is tolerated): the root must be an object, every
    /// field is decoded on its own and a bad one gets its default.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Settings, SettingsError> {
        let value: Value = serde_json::from_slice(strip_bom(bytes))?;
        let Value::Object(object) = value else {
            return Err(SettingsError::NotAnObject);
        };
        let defaults = Settings::default();
        Ok(Settings {
            active_provider: field(&object, "activeProvider").unwrap_or(defaults.active_provider),
            fallbacks: field(&object, "fallbacks").unwrap_or(defaults.fallbacks),
            target_language: field(&object, "targetLanguage").unwrap_or(defaults.target_language),
            hotkeys: object
                .get("hotkeys")
                .and_then(decode_hotkeys)
                .unwrap_or(defaults.hotkeys),
            launch_at_login: field(&object, "launchAtLogin").unwrap_or(defaults.launch_at_login),
            max_input_chars: field(&object, "maxInputChars").unwrap_or(defaults.max_input_chars),
            max_history_count: field(&object, "maxHistoryCount")
                .unwrap_or(defaults.max_history_count),
            menu_history_count: field(&object, "menuHistoryCount")
                .unwrap_or(defaults.menu_history_count),
            use_auto_detect: field(&object, "useAutoDetect").unwrap_or(defaults.use_auto_detect),
            language_pairs: field(&object, "languagePairs").unwrap_or(defaults.language_pairs),
            auto_copy_ocr: field(&object, "autoCopyOCR").unwrap_or(defaults.auto_copy_ocr),
            magic_prompt: field(&object, "magicPrompt").unwrap_or(defaults.magic_prompt),
        })
    }

    /// Pretty-printed JSON; the Magic prompt is sanitized.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, SettingsError> {
        let magic_prompt = sanitize_magic_prompt(&self.magic_prompt);
        let wire = Wire {
            active_provider: self.active_provider,
            fallbacks: &self.fallbacks,
            target_language: &self.target_language,
            hotkeys: &self.hotkeys,
            launch_at_login: self.launch_at_login,
            max_input_chars: self.max_input_chars,
            max_history_count: self.max_history_count,
            menu_history_count: self.menu_history_count,
            use_auto_detect: self.use_auto_detect,
            language_pairs: &self.language_pairs,
            auto_copy_ocr: self.auto_copy_ocr,
            magic_prompt: &magic_prompt,
        };
        Ok(serde_json::to_vec_pretty(&wire)?)
    }

    /// Reads `path`; never writes. Missing or blank = defaults; an unparsable or
    /// non-object file is moved aside with `quarantine` and defaults are used.
    pub fn load(path: &Path, clock: &dyn Clock) -> Result<(Settings, LoadOutcome), SettingsError> {
        let Some(bytes) = read_optional(path)? else {
            return Ok((Settings::default(), LoadOutcome::Missing));
        };
        if is_blank(&bytes) {
            return Ok((Settings::default(), LoadOutcome::Missing));
        }
        match Settings::from_json_bytes(&bytes) {
            Ok(settings) => Ok((settings, LoadOutcome::Loaded)),
            Err(SettingsError::Json(_) | SettingsError::NotAnObject) => {
                let backup = quarantine(path, clock.now())?;
                Ok((Settings::default(), LoadOutcome::CorruptBackedUp(backup)))
            }
            Err(other) => Err(other),
        }
    }

    /// Atomic write (temp file + rename); creates a missing directory.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let bytes = self.to_json_bytes()?;
        atomic_write(path, &bytes, false)?;
        Ok(())
    }

    pub fn fallback(&self, provider: ProviderType) -> Option<ProviderType> {
        self.fallbacks.0.get(&provider).copied()
    }

    /// Sets the fallback of `provider`; `None`, or the provider itself, removes the entry.
    pub fn set_fallback(&mut self, fallback: Option<ProviderType>, provider: ProviderType) {
        match fallback {
            Some(fallback) if fallback != provider => {
                self.fallbacks.0.insert(provider, fallback);
            }
            _ => {
                self.fallbacks.0.remove(&provider);
            }
        }
    }

    pub fn hotkey(&self, operation: AIOperation) -> Option<&Hotkey> {
        self.hotkeys.get(&operation)
    }

    pub fn set_hotkey(&mut self, operation: AIOperation, hotkey: Option<Hotkey>) {
        match hotkey {
            Some(hotkey) => {
                self.hotkeys.insert(operation, hotkey);
            }
            None => {
                self.hotkeys.remove(&operation);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::Modifier;
    use crate::time::{ManualClock, Timestamp};
    use std::fs;

    fn clock() -> ManualClock {
        ManualClock::new(Timestamp::from_apple_seconds(700_000_000.0))
    }

    fn decode(json: &str) -> Settings {
        Settings::from_json_bytes(json.as_bytes()).unwrap()
    }

    fn key_set(settings: &Settings) -> Vec<String> {
        let bytes = settings.to_json_bytes().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    #[test]
    fn defaults_match_the_values_above() {
        let s = Settings::default();
        assert_eq!(s.active_provider, ProviderType::Claude);
        assert!(s.fallbacks.0.is_empty());
        assert_eq!(s.target_language, "Ukrainian");
        assert_eq!(
            s.hotkeys,
            [(AIOperation::Translate, Hotkey::default_translate())]
                .into_iter()
                .collect::<BTreeMap<_, _>>()
        );
        assert!(!s.launch_at_login);
        assert_eq!(s.max_input_chars, 5000);
        assert_eq!(s.max_history_count, 10);
        assert_eq!(s.menu_history_count, 10);
        assert!(!s.use_auto_detect);
        assert_eq!(
            s.language_pairs,
            [
                ("Russian", "Ukrainian"),
                ("Ukrainian", "Russian"),
                ("English", "Ukrainian")
            ]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<BTreeMap<_, _>>()
        );
        assert!(s.auto_copy_ocr);
        assert_eq!(s.magic_prompt, crate::prompts::DEFAULT_MAGIC_PROMPT);
    }

    #[test]
    fn serialized_keys_are_exactly_the_persisted_swift_names() {
        let mut expected = vec![
            "activeProvider",
            "fallbacks",
            "targetLanguage",
            "hotkeys",
            "launchAtLogin",
            "maxInputChars",
            "maxHistoryCount",
            "menuHistoryCount",
            "useAutoDetect",
            "languagePairs",
            "autoCopyOCR",
            "magicPrompt",
        ];
        expected.sort_unstable();
        assert_eq!(key_set(&Settings::default()), expected);
        for forbidden in ["iCloudSync", "providers", "hotkey", "appLanguage"] {
            assert!(!expected.contains(&forbidden));
        }
    }

    #[test]
    fn round_trip_equality() {
        let mut s = Settings {
            active_provider: ProviderType::DeepL,
            use_auto_detect: true,
            launch_at_login: true,
            auto_copy_ocr: false,
            max_history_count: 50,
            magic_prompt: "Be brief in {{targetLanguage}}.".into(),
            ..Settings::default()
        };
        s.set_fallback(Some(ProviderType::Claude), ProviderType::DeepL);
        s.set_fallback(Some(ProviderType::Gemini), ProviderType::OpenAI);
        s.set_hotkey(
            AIOperation::Rewrite,
            Some(Hotkey::new("Digit1", [Modifier::Shift, Modifier::Ctrl])),
        );
        s.language_pairs.insert("German".into(), "English".into());
        let bytes = s.to_json_bytes().unwrap();
        assert_eq!(Settings::from_json_bytes(&bytes).unwrap(), s);
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_fields_default() {
        let s = decode(r#"{"iCloudSync":true,"providers":[],"futureField":1}"#);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn bad_field_falls_back_per_field_not_per_file() {
        let s = decode(
            r#"{"activeProvider":"Bogus","maxHistoryCount":-5,"targetLanguage":"German","useAutoDetect":"yes"}"#,
        );
        assert_eq!(s.active_provider, ProviderType::Claude);
        assert_eq!(s.max_history_count, 10);
        assert_eq!(s.target_language, "German");
        assert!(!s.use_auto_detect);
    }

    #[test]
    fn not_an_object_is_an_error() {
        for json in ["[1]", "42", "null", "\"x\""] {
            assert!(matches!(
                Settings::from_json_bytes(json.as_bytes()),
                Err(SettingsError::NotAnObject)
            ));
        }
        assert!(matches!(
            Settings::from_json_bytes(b"{\"a\":"),
            Err(SettingsError::Json(_))
        ));
    }

    #[test]
    fn fallbacks_object_legacy_array_and_unknown_key() {
        let s = decode(r#"{"fallbacks":{"Claude":"DeepL","OpenAI":"Gemini"}}"#);
        assert_eq!(s.fallback(ProviderType::Claude), Some(ProviderType::DeepL));
        assert_eq!(s.fallback(ProviderType::OpenAI), Some(ProviderType::Gemini));
        let s = decode(r#"{"fallbacks":["Claude","DeepL"]}"#);
        assert_eq!(s.fallback(ProviderType::Claude), Some(ProviderType::DeepL));
        let s = decode(r#"{"fallbacks":{"Bogus":"Claude","Gemini":"Bogus"}}"#);
        assert!(s.fallbacks.0.is_empty());
        let s = decode(r#"{"fallbacks":"nope"}"#);
        assert!(s.fallbacks.0.is_empty());
    }

    #[test]
    fn hotkeys_skip_unknown_operation_and_invalid_hotkey() {
        let s = decode(
            r#"{"hotkeys":{
                "translate":{"key":"KeyT","modifiers":["ctrl","alt"]},
                "bogusOp":{"key":"KeyT","modifiers":["ctrl","alt"]},
                "rewrite":{"key":"t","modifiers":[]}
            }}"#,
        );
        assert_eq!(s.hotkeys.len(), 1);
        assert_eq!(
            s.hotkey(AIOperation::Translate),
            Some(&Hotkey::new("KeyT", [Modifier::Ctrl, Modifier::Alt]))
        );
        assert_eq!(s.hotkey(AIOperation::Rewrite), None);
    }

    #[test]
    fn hotkeys_object_decides_between_empty_map_and_default() {
        // A hotkeys object is authoritative: an empty one, or one whose entries are all
        // skipped, means "no hotkeys" (the user removed them), not "use the default".
        for json in [
            r#"{"hotkeys":{}}"#,
            r#"{"hotkeys":{"rewrite":{"key":"t","modifiers":[]}}}"#,
        ] {
            assert!(decode(json).hotkeys.is_empty(), "{json}");
        }
        // Anything that is not an object is a bad value: fall back to the default.
        let defaults = BTreeMap::from([(AIOperation::Translate, Hotkey::default_translate())]);
        assert_eq!(decode(r#"{"hotkeys":"x"}"#).hotkeys, defaults);
        assert_eq!(
            decode(r#"{"hotkeys":"x"}"#).hotkey(AIOperation::Translate),
            Some(&Hotkey::new("KeyT", [Modifier::Ctrl, Modifier::Alt]))
        );
    }

    #[test]
    fn hotkeys_normalize_modifiers_on_decode() {
        let s =
            decode(r#"{"hotkeys":{"explain":{"key":"KeyQ","modifiers":["alt","alt","ctrl"]}}}"#);
        assert_eq!(
            s.hotkey(AIOperation::Explain).unwrap().modifiers,
            vec![Modifier::Ctrl, Modifier::Alt]
        );
    }

    #[test]
    fn set_fallback_ignores_self_and_none_removes() {
        let mut s = Settings::default();
        s.set_fallback(Some(ProviderType::DeepL), ProviderType::Claude);
        assert_eq!(s.fallback(ProviderType::Claude), Some(ProviderType::DeepL));
        s.set_fallback(Some(ProviderType::Claude), ProviderType::Claude);
        assert_eq!(s.fallback(ProviderType::Claude), None);
        s.set_fallback(Some(ProviderType::DeepL), ProviderType::Claude);
        s.set_fallback(None, ProviderType::Claude);
        assert_eq!(s.fallback(ProviderType::Claude), None);
    }

    #[test]
    fn hotkey_set_hotkey_round_trip() {
        let mut s = Settings::default();
        let hk = Hotkey::new("F5", [Modifier::Shift]);
        s.set_hotkey(AIOperation::Magic, Some(hk.clone()));
        assert_eq!(s.hotkey(AIOperation::Magic), Some(&hk));
        s.set_hotkey(AIOperation::Magic, None);
        assert_eq!(s.hotkey(AIOperation::Magic), None);
        s.set_hotkey(AIOperation::Translate, None);
        assert!(s.hotkeys.is_empty());
    }

    #[test]
    fn sanitize_magic_prompt_trims_and_keeps_newline_and_tab() {
        assert_eq!(
            sanitize_magic_prompt("  \n\t hi\tthere\nfriend \u{a0}\u{3000}\n"),
            "hi\tthere\nfriend"
        );
        assert_eq!(sanitize_magic_prompt(""), "");
        assert_eq!(sanitize_magic_prompt(" \n\t "), "");
    }

    #[test]
    fn sanitize_magic_prompt_drops_ascii_controls() {
        assert_eq!(
            sanitize_magic_prompt("a\u{0}b\u{1b}c\u{8}d\u{1f}e"),
            "abcde"
        );
        assert_eq!(sanitize_magic_prompt("a\r\nb"), "a\nb");
        assert_eq!(sanitize_magic_prompt("a\rb"), "ab");
    }

    #[test]
    fn sanitize_magic_prompt_keeps_del_and_non_ascii() {
        assert_eq!(sanitize_magic_prompt("a\u{7f}b"), "a\u{7f}b");
        assert_eq!(sanitize_magic_prompt("a\u{85}b"), "a\u{85}b");
        assert_eq!(
            sanitize_magic_prompt("a\u{9f}b\u{200b}c"),
            "a\u{9f}b\u{200b}c"
        );
        assert_eq!(
            sanitize_magic_prompt("Привіт, світе 😀"),
            "Привіт, світе 😀"
        );
    }

    #[test]
    fn sanitize_magic_prompt_caps_by_scalars_not_bytes() {
        let emoji = "😀".repeat(2500);
        let out = sanitize_magic_prompt(&emoji);
        assert_eq!(out.chars().count(), 2000);
        assert_eq!(out, "😀".repeat(2000));
        let exact = "a".repeat(2000);
        assert_eq!(sanitize_magic_prompt(&exact), exact);
        let over = "a".repeat(2001);
        assert_eq!(sanitize_magic_prompt(&over).len(), 2000);
    }

    #[test]
    fn to_json_sanitizes_magic_prompt() {
        let s = Settings {
            magic_prompt: "  hello\u{0}\r\nworld  ".into(),
            ..Settings::default()
        };
        let back = Settings::from_json_bytes(&s.to_json_bytes().unwrap()).unwrap();
        assert_eq!(back.magic_prompt, "hello\nworld");
    }

    #[test]
    fn zero_max_input_chars_round_trips_unchanged() {
        let s = Settings {
            max_input_chars: 0,
            ..Settings::default()
        };
        let back = Settings::from_json_bytes(&s.to_json_bytes().unwrap()).unwrap();
        assert_eq!(back.max_input_chars, 0);
        assert_eq!(decode(r#"{"maxInputChars":0}"#).max_input_chars, 0);
    }

    #[test]
    fn load_missing_file_gives_defaults_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let (s, outcome) = Settings::load(&path, &clock()).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(outcome, LoadOutcome::Missing);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn load_blank_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for blank in [&b""[..], b"  \r\n\t", b"\xEF\xBB\xBF", b"\xEF\xBB\xBF \n"] {
            fs::write(&path, blank).unwrap();
            let (s, outcome) = Settings::load(&path, &clock()).unwrap();
            assert_eq!(s, Settings::default());
            assert_eq!(outcome, LoadOutcome::Missing);
            assert_eq!(fs::read(&path).unwrap(), blank, "blank file left alone");
        }
    }

    #[test]
    fn load_bom_prefixed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"targetLanguage":"German"}"#);
        fs::write(&path, bytes).unwrap();
        let (s, outcome) = Settings::load(&path, &clock()).unwrap();
        assert_eq!(s.target_language, "German");
        assert_eq!(outcome, LoadOutcome::Loaded);
    }

    fn assert_quarantined(content: &[u8]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, content).unwrap();
        let (s, outcome) = Settings::load(&path, &clock()).unwrap();
        assert_eq!(s, Settings::default());
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("settings.json.bak-"));
        assert_eq!(fs::read(&backup).unwrap(), content);
        assert!(
            !path.exists(),
            "settings.json must be absent after quarantine"
        );
    }

    #[test]
    fn load_truncated_file_is_quarantined() {
        assert_quarantined(br#"{"activeProvider":"#);
    }

    #[test]
    fn load_top_level_array_is_quarantined() {
        assert_quarantined(b"[1]");
    }

    #[test]
    fn load_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{broken").unwrap();
        let (settings, outcome) = Settings::load(&path, &clock()).unwrap();
        assert!(matches!(outcome, LoadOutcome::CorruptBackedUp(_)));
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "only the backup"
        );
        settings.save(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_is_atomic_and_reloads_equal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut s = Settings {
            target_language: "Polish".into(),
            ..Settings::default()
        };
        s.set_fallback(Some(ProviderType::OpenAI), ProviderType::Claude);
        s.save(&path).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["settings.json".to_string()],
            "no temp files left"
        );
        let (back, outcome) = Settings::load(&path, &clock()).unwrap();
        assert_eq!(back, s);
        assert_eq!(outcome, LoadOutcome::Loaded);
    }

    #[test]
    fn save_creates_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("settings.json");
        Settings::default().save(&path).unwrap();
        assert!(path.exists());
    }
}
