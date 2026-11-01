//! Editable provider fields; keys are only sent to the secret store by the shell.
use crate::{errors::debug_url, ProviderConnectionSettings, ProviderField, ProviderType};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFields {
    pub base_url: String,
    pub model: String,
    pub referer: String,
    pub region: String,
}

impl fmt::Debug for ProviderFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderFields")
            .field("base_url", &debug_url(&self.base_url))
            .field("model", &self.model)
            .field("referer", &debug_url(&self.referer))
            .field("region", &self.region)
            .finish()
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraft {
    pub api_key: String,
    #[serde(flatten)]
    pub fields: ProviderFields,
}

impl fmt::Debug for ProviderDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderDraft")
            .field("api_key", &"<redacted>")
            .field("fields", &self.fields)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftErrors {
    pub base_url: Option<&'static str>,
    pub referer: Option<&'static str>,
    pub region: Option<&'static str>,
}

impl DraftErrors {
    pub fn is_empty(&self) -> bool {
        self.base_url.is_none() && self.referer.is_none() && self.region.is_none()
    }
}

impl ProviderFields {
    pub fn validate(&self, provider: ProviderType) -> DraftErrors {
        let fields = provider.capabilities().settings_fields;
        let base_url = self.base_url.trim();
        let referer = self.referer.trim();
        let region = self.region.trim();
        DraftErrors {
            base_url: (fields.contains(&ProviderField::BaseUrl)
                && !base_url.is_empty()
                && !base_url.to_ascii_lowercase().starts_with("https://"))
            .then_some("Base URL must start with https://"),
            referer: (fields.contains(&ProviderField::Referer)
                && referer.chars().any(char::is_control))
            .then_some("Referer must not contain control characters"),
            region: (fields.contains(&ProviderField::Region)
                && !region
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .then_some("Region may only contain letters, digits, '-' and '_'"),
        }
    }

    /// Like the mac draft, applies all fields; validation is the caller's responsibility.
    pub fn apply(&self, _provider: ProviderType, into: &mut ProviderConnectionSettings) {
        fn optional(value: &str) -> Option<String> {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        into.base_url = self.base_url.trim().to_owned();
        into.model = self.model.trim().to_owned();
        into.referer = optional(&self.referer);
        into.region = optional(&self.region);
    }
}

impl ProviderDraft {
    pub fn validate(&self, provider: ProviderType) -> DraftErrors {
        self.fields.validate(provider)
    }

    pub fn apply(&self, provider: ProviderType, into: &mut ProviderConnectionSettings) {
        self.fields.apply(provider, into);
    }

    pub fn trimmed_api_key(&self) -> Option<&str> {
        let key = self.api_key.trim();
        (!key.is_empty()).then_some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_must_be_https() {
        for (url, error) in [
            ("http://x", Some("Base URL must start with https://")),
            ("  HTTPS://x ", None),
            ("", None),
        ] {
            let fields = ProviderFields {
                base_url: url.into(),
                ..Default::default()
            };
            assert_eq!(fields.validate(ProviderType::Claude).base_url, error);
        }
    }

    #[test]
    fn referer_rejects_control_chars() {
        let mut fields = ProviderFields {
            referer: "https://a/\nb".into(),
            ..Default::default()
        };
        assert_eq!(
            fields.validate(ProviderType::GoogleCloud).referer,
            Some("Referer must not contain control characters")
        );
        assert!(fields.validate(ProviderType::Claude).is_empty());
        fields.referer = "https://a/\n".into();
        assert_eq!(fields.validate(ProviderType::GoogleCloud).referer, None);
    }

    #[test]
    fn region_charset() {
        for (region, error) in [
            ("east us", true),
            ("eastus-2_a", false),
            ("ä", true),
            ("я", true),
            (" ", false),
        ] {
            let fields = ProviderFields {
                region: region.into(),
                ..Default::default()
            };
            assert_eq!(
                fields.validate(ProviderType::MicrosoftTranslator).region,
                error.then_some("Region may only contain letters, digits, '-' and '_'")
            );
            assert!(fields.validate(ProviderType::Claude).is_empty());
        }
    }

    #[test]
    fn apply_trims_and_nones_empty() {
        let mut settings = crate::default_providers()[&ProviderType::GoogleCloud].clone();
        let draft = ProviderDraft {
            api_key: " k ".into(),
            fields: ProviderFields {
                base_url: " https://x ".into(),
                model: " m ".into(),
                referer: "  ".into(),
                region: " eastus ".into(),
            },
        };
        draft.apply(ProviderType::GoogleCloud, &mut settings);
        assert_eq!(settings.base_url, "https://x");
        assert_eq!(settings.model, "m");
        assert_eq!(settings.referer, None);
        assert_eq!(settings.region, Some("eastus".into()));
        assert_eq!(draft.trimmed_api_key(), Some("k"));
        assert_eq!(ProviderDraft::default().trimmed_api_key(), None);
        assert!(draft.validate(ProviderType::GoogleCloud).is_empty());
    }

    #[test]
    fn provider_draft_debug_redacts_key() {
        let draft = ProviderDraft {
            api_key: "KEY_SENTINEL".into(),
            fields: ProviderFields {
                base_url: "https://u:URL_SENTINEL@x/?q=URL_SENTINEL#URL_SENTINEL".into(),
                referer: "user:REF_SENTINEL".into(),
                ..Default::default()
            },
        };
        let debug = format!("{draft:?}");
        assert!(debug.contains("<redacted>"));
        for secret in ["KEY_SENTINEL", "URL_SENTINEL", "REF_SENTINEL"] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn provider_draft_json_flattens_fields() {
        let text = r#"{"apiKey":"k","baseUrl":"https://x","model":"m","referer":"","region":""}"#;
        let draft: ProviderDraft = serde_json::from_str(text).unwrap();
        assert_eq!(draft.api_key, "k");
        assert_eq!(draft.fields.base_url, "https://x");
        let fields: ProviderFields =
            serde_json::from_str(r#"{"baseUrl":"https://x","model":"m","referer":"","region":""}"#)
                .unwrap();
        assert_eq!(draft.fields, fields);
        assert_eq!(
            serde_json::to_value(DraftErrors::default()).unwrap(),
            serde_json::json!({"baseUrl":null,"referer":null,"region":null})
        );
    }
}
