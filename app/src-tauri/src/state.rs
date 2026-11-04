//! Shell configuration, persistence notifications and process UI preferences.
use std::sync::{Arc, Mutex, MutexGuard};

use lingopop_core::history_store::HistoryStore;
use lingopop_core::paths::DataPaths;
use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::runner::ConfigSnapshot;
use lingopop_core::secrets::SecretStore;
use lingopop_core::settings::Settings;
use lingopop_core::time::Clock;
use lingopop_core::usage_store::UsageStore;
use lingopop_core::{
    fsio::{LoadOutcome, StoreError},
    provider::{default_providers, ProviderConnectionSettings, ProviderType},
    secrets::{ProviderSettingsState, SecretsError},
    settings::SettingsError,
    ui_prefs::{Language, UiPrefs},
};
use lingopop_orchestrator::{ConfigSource, Persist};
use lingopop_platform::Platform;
use std::{io, path::PathBuf};
use uuid::Uuid;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn describe(outcome: &LoadOutcome) -> &'static str {
    match outcome {
        LoadOutcome::Locked => "locked",
        LoadOutcome::Missing => "missing",
        LoadOutcome::Loaded => "loaded",
        LoadOutcome::Legacy => "legacy format, upgraded on next write",
        LoadOutcome::CorruptBackedUp(_) => "corrupt, moved aside",
    }
}

pub struct ShellConfig {
    settings: Mutex<Settings>,
    path: PathBuf,
    secrets: Option<Mutex<SecretStore>>,
    secrets_metadata: Mutex<SecretsMetadata>,
}

// Key-free committed metadata. Never held during protector/filesystem calls.
struct SecretsMetadata {
    available: bool,
    locked: bool,
    connections: ProviderSettingsState,
    has_keys: std::collections::BTreeSet<ProviderType>,
}
impl SecretsMetadata {
    fn from_store(store: Option<&SecretStore>) -> Self {
        Self {
            available: store.is_some_and(|s| !s.is_locked()),
            locked: store.is_some_and(SecretStore::is_locked),
            connections: store.map_or(
                ProviderSettingsState::Absent,
                SecretStore::provider_settings,
            ),
            has_keys: ProviderType::ALL
                .into_iter()
                .filter(|p| store.is_some_and(|s| s.has_api_key(*p)))
                .collect(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretsErrorKind {
    #[error("secrets could not be saved")]
    Io(io::ErrorKind),
    #[error("secrets could not be protected")]
    Protect,
    #[error("secrets could not be encoded")]
    Encode,
}
#[derive(Debug, thiserror::Error)]
pub enum SecretsWriteError {
    #[error("secrets are unavailable")]
    Unavailable,
    #[error("provider settings are corrupt")]
    Corrupt,
    #[error(transparent)]
    Store(SecretsErrorKind),
}
impl From<SecretsError> for SecretsWriteError {
    fn from(e: SecretsError) -> Self {
        Self::Store(match e {
            SecretsError::Io(e) => SecretsErrorKind::Io(e.kind()),
            SecretsError::Protect(_) => SecretsErrorKind::Protect,
            SecretsError::Encode(_) => SecretsErrorKind::Encode,
        })
    }
}
impl ShellConfig {
    pub fn reset_secrets(&self) -> Result<(), SecretsWriteError> {
        let secrets = self
            .secrets
            .as_ref()
            .ok_or(SecretsWriteError::Unavailable)?;
        let mut secrets = lock(secrets);
        if !secrets.is_locked() {
            return Err(SecretsWriteError::Unavailable);
        }
        let result = secrets.reset(&lingopop_core::time::SystemClock);
        *lock(&self.secrets_metadata) = SecretsMetadata::from_store(Some(&secrets));
        result.map_err(Into::into)
    }
    pub fn update_settings(
        &self,
        f: impl FnOnce(&mut Settings),
    ) -> Result<Settings, SettingsError> {
        let mut settings = lock(&self.settings);
        let before = settings.clone();
        f(&mut settings);
        if let Err(e) = settings.save(&self.path) {
            *settings = before;
            return Err(e);
        }
        Ok(settings.clone())
    }
    pub fn set_api_key(&self, provider: ProviderType, key: &str) -> Result<(), SecretsWriteError> {
        let mut secrets = lock(
            self.secrets
                .as_ref()
                .ok_or(SecretsWriteError::Unavailable)?,
        );
        let result = secrets.set_api_key(provider, key);
        *lock(&self.secrets_metadata) = SecretsMetadata::from_store(Some(&secrets));
        result.map_err(Into::into)
    }
    pub fn set_connection(
        &self,
        provider: ProviderType,
        settings: ProviderConnectionSettings,
    ) -> Result<(), SecretsWriteError> {
        let mut secrets = lock(
            self.secrets
                .as_ref()
                .ok_or(SecretsWriteError::Unavailable)?,
        );
        let mut map = match secrets.provider_settings() {
            ProviderSettingsState::Loaded(map) => map,
            ProviderSettingsState::Absent => default_providers(),
            ProviderSettingsState::Corrupt => return Err(SecretsWriteError::Corrupt),
        };
        map.insert(provider, settings);
        let result = secrets.set_provider_settings(&map);
        *lock(&self.secrets_metadata) = SecretsMetadata::from_store(Some(&secrets));
        result.map_err(Into::into)
    }
    pub fn has_api_key(&self, provider: ProviderType) -> bool {
        lock(&self.secrets_metadata).has_keys.contains(&provider)
    }
    pub fn connection(&self, provider: ProviderType) -> ProviderConnectionSettings {
        let mut map = default_providers();
        if let ProviderSettingsState::Loaded(m) = &lock(&self.secrets_metadata).connections {
            map.extend(m.clone());
        }
        map.remove(&provider)
            .unwrap_or_else(|| ProviderConnectionSettings {
                base_url: String::new(),
                model: String::new(),
                referer: None,
                region: None,
            })
    }
    pub fn secrets_locked(&self) -> bool {
        lock(&self.secrets_metadata).locked
    }
    pub fn secrets_available(&self) -> bool {
        lock(&self.secrets_metadata).available
    }
    pub fn connections_corrupt(&self) -> bool {
        matches!(
            lock(&self.secrets_metadata).connections,
            ProviderSettingsState::Corrupt
        )
    }
}

impl ConfigSource for ShellConfig {
    fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    fn snapshot(&self) -> ConfigSnapshot {
        let settings = lock(&self.settings);
        match &self.secrets {
            Some(secrets) => {
                let secrets = lock(secrets);
                ConfigSnapshot::from_parts(&settings, &secrets.provider_settings(), &secrets)
            }
            None => ConfigSnapshot::without_keys(&settings),
        }
    }
}

/// Why a command-facing history mutation failed. Carries no user data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HistoryWriteError {
    #[error("history is unavailable")]
    Unavailable,
    #[error("history could not be saved")]
    Write,
}

#[derive(Clone, Copy)]
pub enum StoreKind {
    History,
    Usage,
}
type ChangeHook = Arc<dyn Fn(StoreKind) + Send + Sync>;
pub struct ShellStores {
    on_change: Mutex<Option<ChangeHook>>,
    history: Mutex<Option<HistoryStore>>,
    usage: Mutex<Option<UsageStore>>,
}

impl ShellStores {
    pub fn set_on_change(&self, f: ChangeHook) {
        *lock(&self.on_change) = Some(f);
    }
    /// Entries for best-effort readers (tray menu): an unavailable store reads as empty.
    pub fn history_entries(&self) -> Vec<HistoryEntry> {
        self.history_entries_checked().unwrap_or_default()
    }
    /// `None` when `history.json` could not be opened, so callers can tell "unavailable" from
    /// "empty" (`Some(vec![])`).
    pub fn history_entries_checked(&self) -> Option<Vec<HistoryEntry>> {
        lock(&self.history).as_ref().map(|s| s.entries().to_vec())
    }
    /// Clone only the selected entry; the store lock is released before clipboard work.
    pub fn history_entry(&self, id: uuid::Uuid) -> Option<HistoryEntry> {
        lock(&self.history)
            .as_ref()
            .and_then(|s| s.entries().iter().find(|entry| entry.id == id).cloned())
    }
    pub fn usage_records(&self) -> Vec<UsageRecord> {
        lock(&self.usage)
            .as_ref()
            .map(|s| s.records().to_vec())
            .unwrap_or_default()
    }
    /// False when `usage.json` could not be opened. Callers must not create the file.
    pub fn usage_available(&self) -> bool {
        lock(&self.usage).is_some()
    }
    fn mutate_history(&self, f: impl FnOnce(&mut HistoryStore) -> Result<(), StoreError>) {
        let hook = {
            let mut guard = lock(&self.history);
            let Some(s) = guard.as_mut() else { return };
            if f(s).is_err() {
                log::warn!("history write failed");
                return;
            }
            lock(&self.on_change).clone()
        };
        if let Some(h) = hook {
            h(StoreKind::History);
        }
    }
    fn mutate_usage(&self, f: impl FnOnce(&mut UsageStore) -> Result<(), StoreError>) {
        let hook = {
            let mut guard = lock(&self.usage);
            let Some(s) = guard.as_mut() else { return };
            if f(s).is_err() {
                log::warn!("usage write failed");
                return;
            }
            lock(&self.on_change).clone()
        };
        if let Some(h) = hook {
            h(StoreKind::Usage);
        }
    }
    /// Like `mutate_history`, but reports the failure; the change hook fires only after a save.
    fn mutate_history_checked(
        &self,
        f: impl FnOnce(&mut HistoryStore) -> Result<(), StoreError>,
    ) -> Result<(), HistoryWriteError> {
        let hook = {
            let mut guard = lock(&self.history);
            let s = guard.as_mut().ok_or(HistoryWriteError::Unavailable)?;
            if f(s).is_err() {
                log::warn!("history write failed");
                return Err(HistoryWriteError::Write);
            }
            lock(&self.on_change).clone()
        };
        if let Some(h) = hook {
            h(StoreKind::History);
        }
        Ok(())
    }
    pub fn remove_history(&self, id: Uuid) -> Result<(), HistoryWriteError> {
        self.mutate_history_checked(|s| s.remove(id))
    }
    pub fn clear_history(&self) -> Result<(), HistoryWriteError> {
        self.mutate_history_checked(HistoryStore::clear)
    }
    pub fn trim_history(&self, max: u32) {
        self.mutate_history(|s| s.trim(max));
    }
    /// `Ok(())` when the usage store is unavailable (nothing is created).
    pub fn reset_usage(&self) -> Result<(), StoreError> {
        let hook = {
            let mut guard = lock(&self.usage);
            let Some(s) = guard.as_mut() else {
                return Ok(());
            };
            s.reset()?;
            lock(&self.on_change).clone()
        };
        if let Some(h) = hook {
            h(StoreKind::Usage);
        }
        Ok(())
    }
}
impl Persist for ShellStores {
    fn add_history(&self, entry: HistoryEntry, max_count: u32) {
        self.mutate_history(|s| s.add(entry, max_count));
    }
    fn append_usage(&self, record: UsageRecord) {
        self.mutate_usage(|s| s.append(record));
    }
}
pub struct UiState {
    prefs: Mutex<UiPrefs>,
    path: PathBuf,
    pub launched: Language,
    pending_tab: Mutex<Option<String>>,
}
impl UiState {
    pub fn prefs(&self) -> UiPrefs {
        lock(&self.prefs).clone()
    }
    pub fn update(&self, f: impl FnOnce(&mut UiPrefs)) -> Result<UiPrefs, StoreError> {
        let mut prefs = lock(&self.prefs);
        let before = prefs.clone();
        f(&mut prefs);
        if let Err(e) = prefs.save(&self.path) {
            *prefs = before;
            return Err(e);
        }
        Ok(prefs.clone())
    }
    pub fn language(&self) -> Language {
        self.launched
    }
    pub fn set_pending_tab(&self, tab: Option<String>) {
        *lock(&self.pending_tab) = tab;
    }
    pub fn take_pending_tab(&self) -> Option<String> {
        lock(&self.pending_tab).take()
    }
}

/// Loads every store from `paths` (the directory exists). See the module docs for fallbacks.
pub fn load(
    paths: &DataPaths,
    platform: &Platform,
    clock: Arc<dyn Clock>,
) -> (Arc<ShellConfig>, Arc<ShellStores>, Arc<UiState>) {
    let prefs = UiPrefs::load(&paths.ui_json(), clock.as_ref())
        .map(|(p, _)| p)
        .unwrap_or_else(|_| {
            log::warn!("UI preferences could not be read");
            UiPrefs::default()
        });
    let launched = prefs
        .app_language
        .effective(&platform.system.ui_languages());
    let settings = match Settings::load(&paths.settings_json(), clock.as_ref()) {
        Ok((settings, outcome)) => {
            log::info!("settings: {}", describe(&outcome));
            settings
        }
        Err(e) => {
            log::error!("settings could not be read, using defaults: {e}");
            Settings::default()
        }
    };

    let secrets = match SecretStore::open(
        paths.secrets_dat(),
        platform.protector.clone(),
        clock.clone(),
    ) {
        Ok((store, outcome)) => {
            log::info!("secrets: {}", describe(&outcome));
            Some(store)
        }
        Err(_) => {
            log::error!("secrets could not be opened, starting without API keys");
            None
        }
    };

    let history = match HistoryStore::open(paths.history_json(), clock.clone()) {
        Ok((store, outcome)) => {
            log::info!("history: {}", describe(&outcome));
            Some(store)
        }
        Err(e) => {
            log::error!("history could not be opened, it will not be saved: {e}");
            None
        }
    };
    let usage = match UsageStore::open(paths.usage_json(), clock) {
        Ok((store, outcome)) => {
            log::info!("usage: {}", describe(&outcome));
            Some(store)
        }
        Err(e) => {
            log::error!("usage could not be opened, it will not be saved: {e}");
            None
        }
    };

    (
        Arc::new(ShellConfig {
            settings: Mutex::new(settings),
            path: paths.settings_json(),
            secrets_metadata: Mutex::new(SecretsMetadata::from_store(secrets.as_ref())),
            secrets: secrets.map(Mutex::new),
        }),
        Arc::new(ShellStores {
            history: Mutex::new(history),
            on_change: Mutex::new(None),
            usage: Mutex::new(usage),
        }),
        Arc::new(UiState {
            prefs: Mutex::new(prefs),
            path: paths.ui_json(),
            launched,
            pending_tab: Mutex::new(None),
        }),
    )
}
