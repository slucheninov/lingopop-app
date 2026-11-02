//! Provider commands: the API key path, the Corrupt rule and the guide anchors.
use crate::{
    provider_cmds::*,
    state::*,
    task4_tests::{platform, rig},
};
use lingopop_core::{
    i18n,
    paths::DataPaths,
    provider::ProviderType,
    provider_draft::{ProviderDraft, ProviderFields},
    secrets::{encode_payload, ProtectError, SecretProtector},
    time::SystemClock,
    ui_prefs::Language,
};
use std::sync::{Arc, Mutex};

fn draft(key: &str, base_url: &str) -> ProviderDraft {
    ProviderDraft {
        api_key: key.into(),
        fields: ProviderFields {
            base_url: base_url.into(),
            model: "m".into(),
            ..Default::default()
        },
    }
}

#[test]
fn provider_save_writes_key_and_connection() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap();
    assert!(c.has_api_key(ProviderType::Claude));
    assert_eq!(c.connection(ProviderType::Claude).base_url, "https://x");
    assert_eq!(c.connection(ProviderType::Claude).model, "m");
    // Both survive a restart.
    let (c, _, _) = rig(d.path());
    assert!(c.has_api_key(ProviderType::Claude));
    assert_eq!(c.connection(ProviderType::Claude).base_url, "https://x");
}

#[test]
fn provider_save_without_key_keeps_the_stored_key() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap();
    save_draft(&c, ProviderType::Claude, &draft("  ", "https://y")).unwrap();
    assert!(c.has_api_key(ProviderType::Claude));
    assert_eq!(c.connection(ProviderType::Claude).base_url, "https://y");
}

#[test]
fn provider_save_ignores_fields_the_provider_does_not_show() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let before = c.connection(ProviderType::Claude);
    let mut hostile = draft("", "https://x");
    hostile.fields.referer = "bad\nreferer".into();
    hostile.fields.region = "bad region".into();
    save_draft(&c, ProviderType::Claude, &hostile).unwrap();
    let after = c.connection(ProviderType::Claude);
    assert_eq!(
        (after.referer, after.region),
        (before.referer, before.region)
    );
}

struct FailingProtector;
impl SecretProtector for FailingProtector {
    fn protect(&self, _: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Err(ProtectError::Failed("protect boom".into()))
    }
    fn unprotect(&self, d: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(d.to_vec())
    }
}

/// Lets the first (connection) write through and fails every later one (the key write).
struct FailsAfterFirst(std::sync::atomic::AtomicUsize);
impl SecretProtector for FailsAfterFirst {
    fn protect(&self, d: &[u8]) -> Result<Vec<u8>, ProtectError> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Ok(d.to_vec())
        } else {
            Err(ProtectError::Failed("protect boom".into()))
        }
    }
    fn unprotect(&self, d: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(d.to_vec())
    }
}

#[test]
fn provider_save_key_write_failure_keeps_the_key_message() {
    let mut bundle = platform();
    bundle.protector = Arc::new(FailsAfterFirst(Default::default()));
    let (c, _, _) = load(
        &DataPaths::new(tempfile::tempdir().unwrap().path()),
        &bundle,
        Arc::new(SystemClock),
    );
    let error = save_draft(&c, ProviderType::OpenAI, &draft("sk-1", "https://x")).unwrap_err();
    assert_eq!(error, KEY_KEY_NOT_SAVED);
}

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
struct Capture;
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        LINES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{}", record.args()));
    }
    fn flush(&self) {}
}

#[test]
fn provider_save_response_and_logs_never_contain_key() {
    let _ = log::set_boxed_logger(Box::new(Capture));
    log::set_max_level(log::LevelFilter::Trace);
    let d = tempfile::tempdir().unwrap();
    // The state (the command response) has no key field and no key text.
    let (c, _, _) = rig(d.path());
    save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap();
    let state = provider_state_of(&c, ProviderType::Claude);
    assert!(state.has_key);
    assert!(!serde_json::to_string(&state).unwrap().contains("sk-1"));
    assert!(!format!("{:?}", draft("sk-1", "https://x")).contains("sk-1"));
    // A failing protector: the error value and every log line stay key-free.
    let mut failing = platform();
    failing.protector = Arc::new(FailingProtector);
    let (c, _, _) = load(
        &DataPaths::new(tempfile::tempdir().unwrap().path()),
        &failing,
        Arc::new(SystemClock),
    );
    let error = save_draft(&c, ProviderType::OpenAI, &draft("sk-1", "https://x")).unwrap_err();
    assert!(!error.contains("sk-1"));
    // The connection write failed before the key was reached: the settings message, not the key one.
    assert_eq!(error, KEY_SETTINGS_NOT_SAVED);
    let lines = LINES.lock().unwrap().clone();
    assert!(!lines.is_empty(), "the failure is logged");
    for line in &lines {
        assert!(!line.contains("sk-1"), "{line}");
    }
}

#[test]
fn provider_save_rejects_invalid_draft() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let error = save_draft(&c, ProviderType::Claude, &draft("sk-1", "http://x")).unwrap_err();
    assert_eq!(error, "Invalid provider settings.");
    assert!(!c.has_api_key(ProviderType::Claude));
    assert!(!d.path().join("secrets.dat").exists());
}

fn corrupt_secrets(dir: &std::path::Path) -> Vec<u8> {
    let mut entries = std::collections::BTreeMap::new();
    entries.insert("providerSettings.v1".into(), "garbage".into());
    let bytes = platform()
        .protector
        .protect(&encode_payload(&entries).unwrap())
        .unwrap();
    std::fs::write(dir.join("secrets.dat"), &bytes).unwrap();
    bytes
}

#[test]
fn provider_save_blocked_when_provider_settings_corrupt() {
    let d = tempfile::tempdir().unwrap();
    let bytes = corrupt_secrets(d.path());
    let (c, _, _) = rig(d.path());
    assert!(c.connections_corrupt());
    let error = save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap_err();
    assert_eq!(error, KEY_UNREADABLE);
    assert!(!c.has_api_key(ProviderType::Claude));
    assert_eq!(std::fs::read(d.path().join("secrets.dat")).unwrap(), bytes);
    let state = provider_state_of(&c, ProviderType::Claude);
    assert!(state.connections_corrupt);
}

#[test]
fn provider_save_blocked_when_secrets_unavailable() {
    let d = tempfile::tempdir().unwrap();
    // A directory in place of the file: the store cannot open.
    std::fs::create_dir(d.path().join("secrets.dat")).unwrap();
    let (c, _, _) = rig(d.path());
    assert!(!c.secrets_available());
    let error = save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap_err();
    assert_eq!(error, KEY_UNREADABLE);
    assert!(provider_state_of(&c, ProviderType::Claude).secrets_unavailable);
}

#[test]
fn provider_clear_key_removes_entry() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    save_draft(&c, ProviderType::Claude, &draft("sk-1", "https://x")).unwrap();
    assert!(provider_state_of(&c, ProviderType::Claude).has_key);
    clear_key_of(&c, ProviderType::Claude).unwrap();
    assert!(!provider_state_of(&c, ProviderType::Claude).has_key);
    // The connection stays.
    assert_eq!(c.connection(ProviderType::Claude).base_url, "https://x");
}

#[test]
fn provider_clear_key_keeps_corrupt_provider_settings_bytes_readable() {
    let d = tempfile::tempdir().unwrap();
    corrupt_secrets(d.path());
    let (c, _, _) = rig(d.path());
    clear_key_of(&c, ProviderType::Claude).unwrap();
    // Nothing to clear: the corrupt entry is untouched and still reported.
    assert!(c.connections_corrupt());
}

#[test]
fn provider_key_guide_anchors() {
    let base = "https://github.com/slucheninov/lingopop-app/blob/main/docs/api-keys.md#";
    for (provider, anchor) in [
        (ProviderType::Claude, "claude"),
        (ProviderType::OpenAI, "openai"),
        (ProviderType::Gemini, "gemini"),
        (ProviderType::Grok, "grok"),
        (ProviderType::GoogleFree, "google-free"),
        (ProviderType::GoogleCloud, "google-translate"),
        (ProviderType::DeepL, "deepl"),
        (ProviderType::MicrosoftTranslator, "microsoft-translator"),
    ] {
        let url = key_guide_url(provider);
        assert_eq!(url, format!("{base}{anchor}"));
        assert!(crate::settings_cmds::url_allowed(&url));
    }
}

#[test]
fn provider_state_json_shape() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    let v = serde_json::to_value(provider_state_of(&c, ProviderType::DeepL)).unwrap();
    assert_eq!(v["provider"], "DeepL");
    assert_eq!(v["name"], "DeepL");
    assert_eq!(v["hasKey"], false);
    assert_eq!(v["requiresKey"], true);
    assert_eq!(v["autoBaseUrl"], true);
    assert_eq!(v["defaultBaseUrl"], "https://api.deepl.com");
    assert_eq!(
        v["operations"],
        serde_json::json!(["Translate", "Screen Translate"])
    );
    assert_eq!(v["isActive"], false);
    assert!(v["fallback"].is_null());
    assert!(v["keyGuideUrl"].as_str().unwrap().ends_with("#deepl"));
    let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    for expected in [
        "provider",
        "name",
        "hasKey",
        "requiresKey",
        "baseUrl",
        "model",
        "referer",
        "region",
        "defaultBaseUrl",
        "autoBaseUrl",
        "models",
        "fields",
        "operations",
        "isActive",
        "fallback",
        "secretsUnavailable",
        "connectionsCorrupt",
        "keyGuideUrl",
    ] {
        assert!(keys.iter().any(|k| k == expected), "{expected}");
    }
    let claude = serde_json::to_value(provider_state_of(&c, ProviderType::Claude)).unwrap();
    assert_eq!(
        claude["fields"],
        serde_json::json!(["apiKey", "baseUrl", "model"])
    );
    assert_eq!(claude["isActive"], false);
    let free = serde_json::to_value(provider_state_of(&c, ProviderType::GoogleFree)).unwrap();
    assert_eq!(free["isActive"], true);
    assert_eq!(free["requiresKey"], false);
}

#[test]
fn provider_state_reports_the_fallback() {
    let d = tempfile::tempdir().unwrap();
    let (c, _, _) = rig(d.path());
    c.update_settings(|s| s.set_fallback(Some(ProviderType::OpenAI), ProviderType::Claude))
        .unwrap();
    assert_eq!(
        provider_state_of(&c, ProviderType::Claude).fallback,
        Some(ProviderType::OpenAI)
    );
}

#[test]
fn provider_error_keys_are_translated_in_every_catalog() {
    for key in [
        KEY_INVALID,
        KEY_UNREADABLE,
        KEY_KEY_NOT_SAVED,
        KEY_SETTINGS_NOT_SAVED,
        "Base URL must start with https://",
        "Referer must not contain control characters",
        "Region may only contain letters, digits, '-' and '_'",
    ] {
        for language in [Language::Pl, Language::Uk, Language::De] {
            assert_ne!(i18n::lookup(language, key), key, "{language:?}: {key}");
        }
    }
}
