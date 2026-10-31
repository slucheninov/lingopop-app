//! The shell's `ConfigSource` and `Persist` implementations over the core stores.
//!
//! Startup never crashes on a bad store:
//! - settings: a corrupt file is quarantined by `Settings::load` and defaults are used; an I/O
//!   error also falls back to defaults (nothing is written in Phase 4a);
//! - secrets: when `SecretStore::open` fails (I/O or protector error) the store is `None` in
//!   memory: the snapshot is built with no keys and `ProviderSettingsState::Absent`, no second
//!   file is opened, the real file is untouched and nothing can panic; Phase 4a never writes
//!   secrets;
//! - history / usage: on an open error the store is `None` and persistence is skipped (the real
//!   file is left alone rather than overwritten by a fresh envelope).
//!
//! Log lines are fixed text plus the error's `Display` (I/O errors, never user text).

use std::sync::{Arc, Mutex, MutexGuard};

use lingopop_core::fsio::LoadOutcome;
use lingopop_core::history_store::HistoryStore;
use lingopop_core::paths::DataPaths;
use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::runner::ConfigSnapshot;
use lingopop_core::secrets::SecretStore;
use lingopop_core::settings::Settings;
use lingopop_core::time::Clock;
use lingopop_core::usage_store::UsageStore;
use lingopop_orchestrator::{ConfigSource, Persist};
use lingopop_platform::Platform;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn describe(outcome: &LoadOutcome) -> &'static str {
    match outcome {
        LoadOutcome::Missing => "missing",
        LoadOutcome::Loaded => "loaded",
        LoadOutcome::Legacy => "legacy format, upgraded on next write",
        LoadOutcome::CorruptBackedUp(_) => "corrupt, moved aside",
    }
}

pub struct ShellConfig {
    settings: Mutex<Settings>,
    secrets: Option<Mutex<SecretStore>>,
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

pub struct ShellStores {
    history: Mutex<Option<HistoryStore>>,
    usage: Mutex<Option<UsageStore>>,
}

impl Persist for ShellStores {
    fn add_history(&self, entry: HistoryEntry, max_count: u32) {
        if let Some(store) = lock(&self.history).as_mut() {
            if let Err(e) = store.add(entry, max_count) {
                log::warn!("history write failed: {e}");
            }
        }
    }

    fn append_usage(&self, record: UsageRecord) {
        if let Some(store) = lock(&self.usage).as_mut() {
            if let Err(e) = store.append(record) {
                log::warn!("usage write failed: {e}");
            }
        }
    }
}

/// Loads every store from `paths` (the directory exists). See the module docs for fallbacks.
pub fn load(
    paths: &DataPaths,
    platform: &Platform,
    clock: Arc<dyn Clock>,
) -> (Arc<ShellConfig>, Arc<ShellStores>) {
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
        Err(e) => {
            log::error!("secrets could not be opened, starting without API keys: {e}");
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
            secrets: secrets.map(Mutex::new),
        }),
        Arc::new(ShellStores {
            history: Mutex::new(history),
            usage: Mutex::new(usage),
        }),
    )
}
