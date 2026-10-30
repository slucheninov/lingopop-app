//! `secrets.dat`: API keys and per-provider connection settings, sealed by an
//! injected [`SecretProtector`] (DPAPI on Windows, a fake in tests).
//!
//! Port of `SecretsPayload` (`Models/ProviderSettingsStorage.swift`). The plaintext
//! is a flat JSON object `{String: String}`: API keys are entries named
//! `apiKey.<ProviderType raw value>`, provider settings one extra entry
//! `providerSettings.v1` holding a JSON string
//! `{"version":1,"providers":{"<raw>":{"baseURL":...,"model":...,...}}}`.
//! Unknown entries survive a rewrite. The macOS quarantine marker and migration
//! machinery is not ported: a file that cannot be read is moved aside and the
//! store starts empty, so the user re-enters keys.
//!
//! Security: secret values are never put into error text or `Debug` output.
//! (`decode_payload` returns the plain map, so do not `{:?}` its result.)

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fsio::{atomic_write, quarantine, read_optional, LoadOutcome};
use crate::keyed_map::ProviderKeyedMap;
use crate::provider::{ProviderConnectionSettings, ProviderType};
use crate::time::Clock;

/// Name of the entry holding the provider settings JSON.
pub const PROVIDER_SETTINGS_ENTRY: &str = "providerSettings.v1";

const CURRENT_VERSION: u32 = 1;

/// Name of the entry holding the API key of `provider`.
pub fn api_key_entry(provider: ProviderType) -> String {
    format!("apiKey.{}", provider.raw_value())
}

/// What the `providerSettings.v1` entry of a payload holds.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderSettingsState {
    /// No entry (a key-only file from an older build, or nothing saved yet).
    Absent,
    /// The entry is not `{"version":N,"providers":{...}}`.
    Corrupt,
    Loaded(BTreeMap<ProviderType, ProviderConnectionSettings>),
}

// `providers` before `version` = the sorted-keys order Swift writes.
#[derive(Serialize, Deserialize)]
struct Envelope {
    providers: ProviderKeyedMap<ProviderConnectionSettings>,
    version: u32,
}

/// Decodes the plaintext of `secrets.dat`; `None` if it is not a flat string map.
/// No BOM handling: the bytes come from our own `protect`, not from a user-edited file.
pub fn decode_payload(plaintext: &[u8]) -> Option<BTreeMap<String, String>> {
    serde_json::from_slice(plaintext).ok()
}

/// Compact JSON with sorted keys (`BTreeMap` order).
pub fn encode_payload(entries: &BTreeMap<String, String>) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(entries)
}

/// Reads the provider settings out of decoded entries. Unknown providers and bad
/// values inside the entry are skipped.
pub fn provider_settings_in(entries: &BTreeMap<String, String>) -> ProviderSettingsState {
    let Some(raw) = entries.get(PROVIDER_SETTINGS_ENTRY) else {
        return ProviderSettingsState::Absent;
    };
    match serde_json::from_str::<Envelope>(raw) {
        Ok(envelope) => ProviderSettingsState::Loaded(envelope.providers.0),
        Err(_) => ProviderSettingsState::Corrupt,
    }
}

/// Stores provider settings into `entries`; every other entry is left untouched.
pub fn set_provider_settings_in(
    settings: &BTreeMap<ProviderType, ProviderConnectionSettings>,
    entries: &mut BTreeMap<String, String>,
) -> Result<(), serde_json::Error> {
    let envelope = Envelope {
        providers: ProviderKeyedMap(settings.clone()),
        version: CURRENT_VERSION,
    };
    entries.insert(
        PROVIDER_SETTINGS_ENTRY.to_owned(),
        serde_json::to_string(&envelope)?,
    );
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ProtectError {
    #[error("protecting or unprotecting the secrets failed: {0}")]
    Failed(String),
}

/// Seals and opens the plaintext payload (Windows DPAPI in Phase 3).
pub trait SecretProtector: Send + Sync {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError>;
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("file access failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Protect(#[from] ProtectError),
    #[error("could not encode the secrets payload: {0}")]
    Encode(#[from] serde_json::Error),
}

/// `secrets.dat` in memory. Every write is encode, `protect`, atomic write
/// (private), and only then the in-memory swap: a failure changes neither.
pub struct SecretStore {
    path: PathBuf,
    protector: Arc<dyn SecretProtector>,
    entries: BTreeMap<String, String>,
}

impl std::fmt::Debug for SecretStore {
    // Never prints entries: they hold API keys.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStore")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl SecretStore {
    /// Loads the file without writing. No file or a zero-byte file is an empty
    /// store (`Missing`). A file that `unprotect` rejects, or whose plaintext is
    /// not a flat string map (any size), is moved aside with `quarantine` and the
    /// store starts empty (`CorruptBackedUp`).
    pub fn open(
        path: PathBuf,
        protector: Arc<dyn SecretProtector>,
        clock: Arc<dyn Clock>,
    ) -> Result<(SecretStore, LoadOutcome), SecretsError> {
        let (entries, outcome) = match read_optional(&path)? {
            None => (BTreeMap::new(), LoadOutcome::Missing),
            Some(bytes) if bytes.is_empty() => (BTreeMap::new(), LoadOutcome::Missing),
            Some(bytes) => {
                let decoded = protector
                    .unprotect(&bytes)
                    .ok()
                    .and_then(|plain| decode_payload(&plain));
                match decoded {
                    Some(entries) => (entries, LoadOutcome::Loaded),
                    None => {
                        let backup = quarantine(&path, clock.now())?;
                        (BTreeMap::new(), LoadOutcome::CorruptBackedUp(backup))
                    }
                }
            }
        };
        Ok((
            SecretStore {
                path,
                protector,
                entries,
            },
            outcome,
        ))
    }

    pub fn api_key(&self, provider: ProviderType) -> Option<&str> {
        self.entries
            .get(&api_key_entry(provider))
            .map(String::as_str)
    }

    pub fn has_api_key(&self, provider: ProviderType) -> bool {
        self.api_key(provider).is_some()
    }

    /// Stores the trimmed key; a key that is empty after trimming deletes the entry.
    pub fn set_api_key(&mut self, provider: ProviderType, key: &str) -> Result<(), SecretsError> {
        let name = api_key_entry(provider);
        let key = key.trim();
        let mut next = self.entries.clone();
        if key.is_empty() {
            if next.remove(&name).is_none() {
                return Ok(());
            }
        } else {
            next.insert(name, key.to_owned());
        }
        self.persist(next)
    }

    pub fn provider_settings(&self) -> ProviderSettingsState {
        provider_settings_in(&self.entries)
    }

    /// Replaces the provider settings entry (also repairing a corrupt one).
    pub fn set_provider_settings(
        &mut self,
        settings: &BTreeMap<ProviderType, ProviderConnectionSettings>,
    ) -> Result<(), SecretsError> {
        let mut next = self.entries.clone();
        set_provider_settings_in(settings, &mut next)?;
        self.persist(next)
    }

    fn persist(&mut self, next: BTreeMap<String, String>) -> Result<(), SecretsError> {
        let sealed = self.protector.protect(&encode_payload(&next)?)?;
        atomic_write(&self.path, &sealed, true)?;
        self.entries = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::fsio::LoadOutcome;
    use crate::provider::{ProviderConnectionSettings, ProviderType};
    use crate::time::{ManualClock, Timestamp};

    const PREFIX: &[u8] = b"FAKE:";

    /// Prefix + XOR 0x5A; `unprotect` rejects data without the prefix.
    struct XorProtector;

    impl SecretProtector for XorProtector {
        fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
            let mut out = PREFIX.to_vec();
            out.extend(plain.iter().map(|b| b ^ 0x5A));
            Ok(out)
        }

        fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
            let body = data
                .strip_prefix(PREFIX)
                .ok_or_else(|| ProtectError::Failed("missing prefix".into()))?;
            Ok(body.iter().map(|b| b ^ 0x5A).collect())
        }
    }

    struct FailingProtector;

    impl SecretProtector for FailingProtector {
        fn protect(&self, _: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Err(ProtectError::Failed("protect refused".into()))
        }

        fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Err(ProtectError::Failed("unprotect refused".into()))
        }
    }

    /// Works like `XorProtector` until `fail_protect` is switched on.
    struct FlakyProtector {
        fail_protect: AtomicBool,
    }

    impl SecretProtector for FlakyProtector {
        fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
            if self.fail_protect.load(Ordering::SeqCst) {
                return Err(ProtectError::Failed("protect refused".into()));
            }
            XorProtector.protect(plain)
        }

        fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
            XorProtector.unprotect(data)
        }
    }

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn conn(base: &str, model: &str) -> ProviderConnectionSettings {
        ProviderConnectionSettings {
            base_url: base.into(),
            model: model.into(),
            referer: None,
            region: None,
        }
    }

    fn sample_settings() -> BTreeMap<ProviderType, ProviderConnectionSettings> {
        BTreeMap::from([
            (
                ProviderType::Claude,
                conn("https://api.anthropic.com", "claude-haiku-5-5"),
            ),
            (
                ProviderType::MicrosoftTranslator,
                ProviderConnectionSettings {
                    region: Some("westeurope".into()),
                    ..conn("https://api.cognitive.microsofttranslator.com", "")
                },
            ),
            (
                ProviderType::GoogleCloud,
                ProviderConnectionSettings {
                    referer: Some("https://example.com/app".into()),
                    ..conn("https://translation.googleapis.com", "nmt")
                },
            ),
        ])
    }

    fn loaded(state: ProviderSettingsState) -> BTreeMap<ProviderType, ProviderConnectionSettings> {
        match state {
            ProviderSettingsState::Loaded(map) => map,
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    // ---- payload ----

    #[test]
    fn entry_names() {
        assert_eq!(PROVIDER_SETTINGS_ENTRY, "providerSettings.v1");
        assert_eq!(api_key_entry(ProviderType::Claude), "apiKey.Claude");
        assert_eq!(
            api_key_entry(ProviderType::GoogleFree),
            "apiKey.Google (Free)"
        );
    }

    #[test]
    fn decodes_old_key_only_format() {
        let map =
            decode_payload(br#"{"apiKey.Claude":"sk-1","apiKey.Google (Free)":"x"}"#).unwrap();
        assert_eq!(
            map,
            entries(&[("apiKey.Claude", "sk-1"), ("apiKey.Google (Free)", "x")])
        );
        assert_eq!(provider_settings_in(&map), ProviderSettingsState::Absent);
    }

    #[test]
    fn rejects_non_flat_string_map() {
        assert_eq!(decode_payload(br#"{"version":2,"secrets":{}}"#), None);
        assert_eq!(decode_payload(b"[1,2]"), None);
        assert_eq!(decode_payload(b"not json"), None);
        assert_eq!(decode_payload(b""), None);
    }

    #[test]
    fn provider_settings_round_trip_keeps_api_keys() {
        let mut map = entries(&[("apiKey.Claude", "sk-1")]);
        set_provider_settings_in(&sample_settings(), &mut map).unwrap();
        let bytes = encode_payload(&map).unwrap();
        let back = decode_payload(&bytes).unwrap();
        assert_eq!(back, map);
        assert_eq!(back["apiKey.Claude"], "sk-1");
        assert_eq!(loaded(provider_settings_in(&back)), sample_settings());
    }

    #[test]
    fn new_payload_is_still_a_flat_string_map() {
        let mut map = BTreeMap::new();
        set_provider_settings_in(&sample_settings(), &mut map).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&encode_payload(&map).unwrap()).unwrap();
        assert!(value[PROVIDER_SETTINGS_ENTRY].is_string());
        let nested: serde_json::Value =
            serde_json::from_str(value[PROVIDER_SETTINGS_ENTRY].as_str().unwrap()).unwrap();
        assert_eq!(nested["version"], 1);
        assert!(nested["providers"]["Claude"].is_object());
    }

    #[test]
    fn encode_is_sorted_and_compact() {
        let map = entries(&[("b", "2"), ("a", "1")]);
        assert_eq!(encode_payload(&map).unwrap(), br#"{"a":"1","b":"2"}"#);
    }

    #[test]
    fn set_provider_settings_preserves_unknown_entries() {
        let mut map = entries(&[("apiKey.Claude", "sk-1"), ("futureEntry.v9", r#"{"x":1}"#)]);
        set_provider_settings_in(&sample_settings(), &mut map).unwrap();
        assert_eq!(map["futureEntry.v9"], r#"{"x":1}"#);
        assert_eq!(map["apiKey.Claude"], "sk-1");
        assert!(map.contains_key(PROVIDER_SETTINGS_ENTRY));
    }

    #[test]
    fn provider_settings_skip_unknown_provider_and_bad_entry() {
        let nested = r#"{"version":1,"providers":{
            "Claude":{"baseURL":"https://a","model":"m"},
            "Bogus":{"baseURL":"https://b","model":"m"},
            "DeepL":"x"}}"#;
        let map = entries(&[(PROVIDER_SETTINGS_ENTRY, nested)]);
        assert_eq!(
            loaded(provider_settings_in(&map)),
            BTreeMap::from([(ProviderType::Claude, conn("https://a", "m"))])
        );
    }

    #[test]
    fn corrupt_provider_settings_entry_is_corrupt_and_api_keys_still_readable() {
        for bad in ["not json", r#"{"providers":{}}"#, r#"{"version":1}"#, "[1]"] {
            let map = entries(&[("apiKey.Claude", "sk-1"), (PROVIDER_SETTINGS_ENTRY, bad)]);
            assert_eq!(
                provider_settings_in(&map),
                ProviderSettingsState::Corrupt,
                "{bad}"
            );
            assert_eq!(map["apiKey.Claude"], "sk-1");
        }
    }

    #[test]
    fn swift_fixture_payload_decodes() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mac/secrets.payload.json");
        let map = decode_payload(&fs::read(path).unwrap()).unwrap();
        assert_eq!(map["apiKey.Claude"], "sk-ant-FIXTURE");
        assert_eq!(map["futureEntry.v9"], r#"{"x":1}"#);
        let settings = loaded(provider_settings_in(&map));
        assert_eq!(
            settings.keys().copied().collect::<Vec<_>>(),
            [
                ProviderType::Claude,
                ProviderType::GoogleCloud,
                ProviderType::MicrosoftTranslator
            ]
        );
        assert_eq!(
            settings[&ProviderType::GoogleCloud].referer.as_deref(),
            Some("https://example.com/app")
        );
        assert_eq!(
            settings[&ProviderType::MicrosoftTranslator]
                .region
                .as_deref(),
            Some("westeurope")
        );

        let mut rewritten = map.clone();
        set_provider_settings_in(&settings, &mut rewritten).unwrap();
        assert_eq!(loaded(provider_settings_in(&rewritten)), settings);
    }

    // ---- store ----

    struct Fixture {
        _dir: tempfile::TempDir,
        path: PathBuf,
        clock: Arc<ManualClock>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.dat");
        Fixture {
            _dir: dir,
            path,
            clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(
                1_793_366_956.0,
            ))),
        }
    }

    impl Fixture {
        fn open_with(
            &self,
            protector: Arc<dyn SecretProtector>,
        ) -> Result<(SecretStore, LoadOutcome), SecretsError> {
            SecretStore::open(self.path.clone(), protector, self.clock.clone())
        }

        fn open(&self) -> (SecretStore, LoadOutcome) {
            self.open_with(Arc::new(XorProtector)).unwrap()
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.path.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    #[test]
    fn open_missing_is_empty_and_writes_nothing() {
        let fx = fixture();
        let (store, outcome) = fx.open();
        assert_eq!(outcome, LoadOutcome::Missing);
        assert_eq!(store.api_key(ProviderType::Claude), None);
        assert_eq!(store.provider_settings(), ProviderSettingsState::Absent);
        assert!(fx.names().is_empty());
    }

    #[test]
    fn set_api_key_trims_persists_and_reopens() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store
            .set_api_key(ProviderType::Claude, "  sk-1 \n")
            .unwrap();
        assert_eq!(store.api_key(ProviderType::Claude), Some("sk-1"));

        let (reopened, outcome) = fx.open();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(reopened.api_key(ProviderType::Claude), Some("sk-1"));
        assert_eq!(reopened.api_key(ProviderType::OpenAI), None);
    }

    #[test]
    fn empty_key_deletes_entry() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        store.set_api_key(ProviderType::DeepL, "d-1").unwrap();
        store.set_api_key(ProviderType::Claude, "  ").unwrap();
        assert_eq!(store.api_key(ProviderType::Claude), None);

        let (reopened, _) = fx.open();
        assert_eq!(reopened.api_key(ProviderType::Claude), None);
        assert_eq!(reopened.api_key(ProviderType::DeepL), Some("d-1"));
    }

    #[test]
    fn deleting_a_missing_key_writes_nothing() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store.set_api_key(ProviderType::Claude, "").unwrap();
        assert!(fx.names().is_empty());
    }

    #[test]
    fn has_api_key() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        assert!(!store.has_api_key(ProviderType::Claude));
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        assert!(store.has_api_key(ProviderType::Claude));
        assert!(!store.has_api_key(ProviderType::Gemini));
        store.set_api_key(ProviderType::Claude, "").unwrap();
        assert!(!store.has_api_key(ProviderType::Claude));
    }

    #[test]
    fn file_bytes_do_not_contain_the_plaintext_key() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store
            .set_api_key(ProviderType::Claude, "sk-ant-SECRET-VALUE")
            .unwrap();
        let bytes = fs::read(&fx.path).unwrap();
        assert!(bytes.starts_with(PREFIX));
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("sk-ant-SECRET-VALUE"));
        assert!(!text.contains("apiKey.Claude"));
    }

    #[test]
    fn unknown_entries_preserved_on_rewrite() {
        let fx = fixture();
        let plain = encode_payload(&entries(&[
            ("apiKey.Claude", "sk-1"),
            ("futureEntry.v9", r#"{"x":1}"#),
        ]))
        .unwrap();
        fs::write(&fx.path, XorProtector.protect(&plain).unwrap()).unwrap();

        let (mut store, outcome) = fx.open();
        assert_eq!(outcome, LoadOutcome::Loaded);
        store.set_api_key(ProviderType::DeepL, "d-1").unwrap();
        store.set_provider_settings(&sample_settings()).unwrap();

        let raw = XorProtector
            .unprotect(&fs::read(&fx.path).unwrap())
            .unwrap();
        let map = decode_payload(&raw).unwrap();
        assert_eq!(map["futureEntry.v9"], r#"{"x":1}"#);
        assert_eq!(map["apiKey.Claude"], "sk-1");
        assert_eq!(map["apiKey.DeepL"], "d-1");
    }

    #[test]
    fn provider_settings_persist_with_referer_and_region() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        store.set_provider_settings(&sample_settings()).unwrap();
        assert_eq!(loaded(store.provider_settings()), sample_settings());

        let (reopened, _) = fx.open();
        assert_eq!(loaded(reopened.provider_settings()), sample_settings());
        assert_eq!(reopened.api_key(ProviderType::Claude), Some("sk-1"));
    }

    #[test]
    fn undecryptable_file_is_quarantined() {
        let fx = fixture();
        let garbage = b"\x00\x01garbage that is not ours, long enough to be anything".to_vec();
        fs::write(&fx.path, &garbage).unwrap();

        let (mut store, outcome) = fx.open();
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert_eq!(backup, fx.path.with_file_name("secrets.dat.bak-1793366956"));
        assert_eq!(fs::read(&backup).unwrap(), garbage);
        assert!(!fx.path.exists());
        assert!(!store.has_api_key(ProviderType::Claude));
        assert_eq!(store.provider_settings(), ProviderSettingsState::Absent);

        store.set_api_key(ProviderType::Claude, "sk-new").unwrap();
        assert_eq!(fs::read(&backup).unwrap(), garbage);
        assert_eq!(fx.open().0.api_key(ProviderType::Claude), Some("sk-new"));
    }

    #[test]
    fn truncated_and_non_map_plaintext_are_quarantined() {
        // 5 bytes, truncated.
        let fx = fixture();
        fs::write(&fx.path, b"abcde").unwrap();
        let (_, outcome) = fx.open();
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert_eq!(fs::read(backup).unwrap(), b"abcde");

        // Decrypts fine, but the plaintext is not a flat string map.
        let fx = fixture();
        let sealed = XorProtector.protect(b"[1,2]").unwrap();
        fs::write(&fx.path, &sealed).unwrap();
        let (store, outcome) = fx.open();
        let LoadOutcome::CorruptBackedUp(backup) = outcome else {
            panic!("expected CorruptBackedUp, got {outcome:?}");
        };
        assert_eq!(fs::read(backup).unwrap(), sealed);
        assert!(!store.has_api_key(ProviderType::Claude));
    }

    #[test]
    fn zero_byte_file_is_missing_without_backup() {
        let fx = fixture();
        fs::write(&fx.path, b"").unwrap();
        let (mut store, outcome) = fx.open();
        assert_eq!(outcome, LoadOutcome::Missing);
        assert_eq!(fx.names(), ["secrets.dat"]);
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        assert_eq!(fx.names(), ["secrets.dat"]);
        assert_eq!(fx.open().0.api_key(ProviderType::Claude), Some("sk-1"));
    }

    #[test]
    fn failing_protector_write_leaves_memory_and_file_unchanged() {
        // Empty store: nothing is written, memory stays empty.
        let fx = fixture();
        let (mut store, outcome) = fx.open_with(Arc::new(FailingProtector)).unwrap();
        assert_eq!(outcome, LoadOutcome::Missing);
        let err = store.set_api_key(ProviderType::Claude, "sk-1").unwrap_err();
        assert!(matches!(err, SecretsError::Protect(_)), "{err}");
        assert_eq!(store.api_key(ProviderType::Claude), None);
        assert!(matches!(
            store.set_provider_settings(&sample_settings()),
            Err(SecretsError::Protect(_))
        ));
        assert_eq!(store.provider_settings(), ProviderSettingsState::Absent);
        assert!(fx.names().is_empty());

        // Existing store: the old file and the old memory survive a failed write.
        let fx = fixture();
        let flaky = Arc::new(FlakyProtector {
            fail_protect: AtomicBool::new(false),
        });
        let (mut store, _) = fx.open_with(flaky.clone()).unwrap();
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        let before = fs::read(&fx.path).unwrap();
        flaky.fail_protect.store(true, Ordering::SeqCst);
        assert!(store.set_api_key(ProviderType::Claude, "sk-2").is_err());
        assert!(store.set_api_key(ProviderType::DeepL, "d-1").is_err());
        assert_eq!(store.api_key(ProviderType::Claude), Some("sk-1"));
        assert_eq!(store.api_key(ProviderType::DeepL), None);
        assert_eq!(fs::read(&fx.path).unwrap(), before);
        assert_eq!(fx.names(), ["secrets.dat"]);
    }

    #[test]
    fn io_failure_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("data");
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(
            1_793_366_956.0,
        )));
        let (mut store, _) =
            SecretStore::open(parent.join("secrets.dat"), Arc::new(XorProtector), clock).unwrap();
        // The data directory cannot be created: a regular file sits in its place.
        fs::write(&parent, b"i am a file").unwrap();
        let err = store.set_api_key(ProviderType::Claude, "sk-1").unwrap_err();
        assert!(matches!(err, SecretsError::Io(_)), "{err}");
        assert_eq!(store.api_key(ProviderType::Claude), None);
    }

    #[test]
    fn debug_output_never_contains_secret_values() {
        let fx = fixture();
        let (mut store, _) = fx.open();
        store
            .set_api_key(ProviderType::Claude, "sk-ant-SECRET-VALUE")
            .unwrap();
        let text = format!("{store:?}");
        assert!(!text.contains("sk-ant-SECRET-VALUE"), "{text}");
        let state = format!("{:?}", store.provider_settings());
        assert!(!state.contains("SECRET"), "{state}");
    }

    #[cfg(unix)]
    #[test]
    fn written_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        let (mut store, _) = fx.open();
        store.set_api_key(ProviderType::Claude, "sk-1").unwrap();
        let mode = fs::metadata(&fx.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
