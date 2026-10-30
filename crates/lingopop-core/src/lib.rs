#![forbid(unsafe_code)]

pub mod operation;
pub mod provider;

pub use operation::AIOperation;
pub use provider::{
    default_providers, ProviderCapabilities, ProviderConnectionSettings, ProviderField,
    ProviderType,
};

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
