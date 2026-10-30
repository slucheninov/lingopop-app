#![forbid(unsafe_code)]

pub mod envelope;
pub mod fsio;
pub mod languages;
pub mod operation;
pub mod provider;
pub mod records;
pub mod serde_uuid;
pub mod time;

pub use languages::LanguageInfo;
pub use operation::AIOperation;
pub use provider::{
    default_providers, ProviderCapabilities, ProviderConnectionSettings, ProviderField,
    ProviderType,
};
pub use records::{HistoryEntry, UsageRecord};
pub use time::Timestamp;

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
