#![forbid(unsafe_code)]

pub mod action_row;
pub mod detector;
pub mod envelope;
pub mod errors;
pub mod fsio;
pub mod history_store;
pub mod hotkey;
pub mod http;
pub mod keyed_map;
pub mod languages;
pub mod operation;
pub mod paths;
pub mod prompts;
pub mod provider;
pub mod providers;
pub mod records;
pub mod secrets;
pub mod serde_uuid;
pub mod settings;
mod store_common;
pub mod target_language;
pub mod time;
pub mod usage_store;

pub use fsio::{LoadOutcome, StoreError};
pub use history_store::HistoryStore;
pub use hotkey::{Hotkey, Modifier};
pub use keyed_map::ProviderKeyedMap;
pub use languages::LanguageInfo;
pub use operation::AIOperation;
pub use paths::DataPaths;
pub use prompts::{
    sanitize_output, system_prompt, user_content, vision_prompt, DEFAULT_MAGIC_PROMPT,
};
pub use provider::{
    default_providers, ProviderCapabilities, ProviderConnectionSettings, ProviderField,
    ProviderType,
};
pub use providers::{AIProvider, ProviderConfig, ProviderRegistry, ProviderResult, TokenUsage};
pub use records::{HistoryEntry, UsageRecord};
pub use secrets::{
    ProtectError, ProviderSettingsState, SecretProtector, SecretStore, SecretsError,
};
pub use settings::{sanitize_magic_prompt, Settings, SettingsError};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
pub use usage_store::UsageStore;

/// Crate version, taken from the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semver_shaped() {
        let parts: Vec<&str> = crate::VERSION.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "expected MAJOR.MINOR.PATCH, got {}",
            crate::VERSION
        );
        for part in parts {
            assert!(
                !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
                "non-numeric version component {part:?} in {}",
                crate::VERSION
            );
        }
    }
}
