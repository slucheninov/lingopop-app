use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::errors::debug_url;
use crate::operation::AIOperation;

/// A translation / AI provider. The serde names are the frozen persisted raw
/// values (settings keys, history and usage records, `apiKey.<raw>` secrets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProviderType {
    #[serde(rename = "Claude")]
    Claude,
    #[serde(rename = "OpenAI")]
    OpenAI,
    #[serde(rename = "Gemini")]
    Gemini,
    #[serde(rename = "Grok")]
    Grok,
    #[serde(rename = "Google (Free)")]
    GoogleFree,
    #[serde(rename = "Google Translate")]
    GoogleCloud,
    #[serde(rename = "DeepL")]
    DeepL,
    #[serde(rename = "Microsoft Translator")]
    MicrosoftTranslator,
}

/// A field shown in a provider's settings form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderField {
    ApiKey,
    BaseUrl,
    Model,
    Referer,
    Region,
}

/// Static, per-provider facts. Order of every list is significant (UI order).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCapabilities {
    pub default_base_url: &'static str,
    pub available_models: &'static [&'static str],
    pub requires_api_key: bool,
    pub supported_operations: &'static [AIOperation],
    pub supports_vision: bool,
    /// An empty base URL is valid; the endpoint is chosen automatically (DeepL, by key suffix).
    pub auto_base_url: bool,
    pub settings_fields: &'static [ProviderField],
    pub preferred_default_model: Option<&'static str>,
}

/// Per-provider connection settings (the `providerSettings.v1` entry shape). The hand-written
/// `Debug` shows the URL-valued fields through `errors::debug_url` (scheme, host and port only: no user info, path, query or
/// fragment), so a pasted `https://user:secret@host/?key=...` never reaches a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionSettings {
    #[serde(rename = "baseURL")]
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

impl fmt::Debug for ProviderConnectionSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConnectionSettings")
            .field("base_url", &debug_url(&self.base_url))
            .field("model", &self.model)
            .field("referer", &self.referer.as_deref().map(debug_url))
            .field("region", &self.region)
            .finish()
    }
}

const CHAT_FIELDS: &[ProviderField] = &[
    ProviderField::ApiKey,
    ProviderField::BaseUrl,
    ProviderField::Model,
];
const TRANSLATE_OPERATIONS: &[AIOperation] =
    &[AIOperation::Translate, AIOperation::ScreenTranslate];

const CLAUDE: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://api.anthropic.com",
    available_models: &[
        "claude-fable-5-1",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
        "claude-opus-4-7",
    ],
    requires_api_key: true,
    supported_operations: &AIOperation::ALL,
    supports_vision: true,
    auto_base_url: false,
    settings_fields: CHAT_FIELDS,
    preferred_default_model: Some("claude-haiku-5-5"),
};

const OPENAI: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://api.openai.com",
    available_models: &[
        "gpt-6-astra",
        "gpt-6.1-sol",
        "gpt-6-luna",
        "gpt-5.6-luna",
        "gpt-5.6-terra",
        "gpt-5.6-sol",
        "gpt-5.5-pro",
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.4-mini",
    ],
    requires_api_key: true,
    supported_operations: &AIOperation::ALL,
    supports_vision: true,
    auto_base_url: false,
    settings_fields: CHAT_FIELDS,
    preferred_default_model: Some("gpt-6-luna"),
};

const GEMINI: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://generativelanguage.googleapis.com",
    available_models: &[
        "gemini-4-argon",
        "gemini-3.8-flash",
        "gemini-3.6-flash",
        "gemini-3.5-flash-lite",
        "gemini-3.1-pro-preview",
        "gemini-3.1-flash-lite",
        "gemini-2.5-flash-lite",
        "gemini-2.5-flash",
        "gemini-2.5-pro",
    ],
    requires_api_key: true,
    supported_operations: &AIOperation::ALL,
    supports_vision: true,
    auto_base_url: false,
    settings_fields: CHAT_FIELDS,
    preferred_default_model: Some("gemini-3.8-flash"),
};

const GROK: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://api.x.ai",
    available_models: &[
        "grok-4.7",
        "grok-4.6",
        "grok-4.5",
        "grok-4.3",
        "grok-4.20-0309-reasoning",
        "grok-4.20-0309-non-reasoning",
        "grok-build-0.1",
    ],
    requires_api_key: true,
    supported_operations: &AIOperation::ALL,
    supports_vision: true,
    auto_base_url: false,
    settings_fields: CHAT_FIELDS,
    preferred_default_model: Some("grok-4.3"),
};

const GOOGLE_FREE: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://translate.googleapis.com",
    available_models: &[],
    requires_api_key: false,
    supported_operations: TRANSLATE_OPERATIONS,
    supports_vision: false,
    auto_base_url: false,
    settings_fields: &[ProviderField::BaseUrl, ProviderField::Model],
    preferred_default_model: None,
};

const GOOGLE_CLOUD: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://translation.googleapis.com",
    available_models: &["nmt", "base"],
    requires_api_key: true,
    supported_operations: TRANSLATE_OPERATIONS,
    supports_vision: false,
    auto_base_url: false,
    settings_fields: &[
        ProviderField::ApiKey,
        ProviderField::BaseUrl,
        ProviderField::Model,
        ProviderField::Referer,
    ],
    preferred_default_model: None,
};

const DEEPL: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://api.deepl.com",
    available_models: &[],
    requires_api_key: true,
    supported_operations: TRANSLATE_OPERATIONS,
    supports_vision: false,
    auto_base_url: true,
    settings_fields: CHAT_FIELDS,
    preferred_default_model: None,
};

const MICROSOFT: ProviderCapabilities = ProviderCapabilities {
    default_base_url: "https://api.cognitive.microsofttranslator.com",
    available_models: &[],
    requires_api_key: true,
    supported_operations: TRANSLATE_OPERATIONS,
    supports_vision: false,
    auto_base_url: false,
    settings_fields: &[
        ProviderField::ApiKey,
        ProviderField::BaseUrl,
        ProviderField::Region,
    ],
    preferred_default_model: None,
};

impl ProviderType {
    /// Every provider, in declaration (UI) order.
    pub const ALL: [ProviderType; 8] = [
        ProviderType::Claude,
        ProviderType::OpenAI,
        ProviderType::Gemini,
        ProviderType::Grok,
        ProviderType::GoogleFree,
        ProviderType::GoogleCloud,
        ProviderType::DeepL,
        ProviderType::MicrosoftTranslator,
    ];

    /// The frozen persisted identifier. Never show this to the user.
    pub fn raw_value(self) -> &'static str {
        match self {
            ProviderType::Claude => "Claude",
            ProviderType::OpenAI => "OpenAI",
            ProviderType::Gemini => "Gemini",
            ProviderType::Grok => "Grok",
            ProviderType::GoogleFree => "Google (Free)",
            ProviderType::GoogleCloud => "Google Translate",
            ProviderType::DeepL => "DeepL",
            ProviderType::MicrosoftTranslator => "Microsoft Translator",
        }
    }

    /// Parses a persisted identifier; unknown values (from a newer version) give `None`.
    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.raw_value() == raw)
    }

    /// Brand name for UI, menus and error text. Equal to the raw value today,
    /// but only this one may be reworded.
    pub fn display_name(self) -> &'static str {
        match self {
            ProviderType::Claude => "Claude",
            ProviderType::OpenAI => "OpenAI",
            ProviderType::Gemini => "Gemini",
            ProviderType::Grok => "Grok",
            ProviderType::GoogleFree => "Google (Free)",
            ProviderType::GoogleCloud => "Google Translate",
            ProviderType::DeepL => "DeepL",
            ProviderType::MicrosoftTranslator => "Microsoft Translator",
        }
    }

    pub fn capabilities(self) -> &'static ProviderCapabilities {
        match self {
            ProviderType::Claude => &CLAUDE,
            ProviderType::OpenAI => &OPENAI,
            ProviderType::Gemini => &GEMINI,
            ProviderType::Grok => &GROK,
            ProviderType::GoogleFree => &GOOGLE_FREE,
            ProviderType::GoogleCloud => &GOOGLE_CLOUD,
            ProviderType::DeepL => &DEEPL,
            ProviderType::MicrosoftTranslator => &MICROSOFT,
        }
    }

    pub fn supports(self, operation: AIOperation) -> bool {
        self.capabilities()
            .supported_operations
            .contains(&operation)
    }
}

/// First-launch settings for every provider (port of `SettingsStore.defaultProviders`):
/// the default base URL and the preferred model if it is in the list, else the
/// first model, else an empty string. DeepL keeps an empty base URL because its
/// endpoint is chosen by the key suffix.
pub fn default_providers() -> BTreeMap<ProviderType, ProviderConnectionSettings> {
    ProviderType::ALL
        .into_iter()
        .map(|provider| {
            let caps = provider.capabilities();
            let model = caps
                .preferred_default_model
                .filter(|m| caps.available_models.contains(m))
                .or_else(|| caps.available_models.first().copied())
                .unwrap_or("");
            let base_url = if caps.auto_base_url {
                ""
            } else {
                caps.default_base_url
            };
            (
                provider,
                ProviderConnectionSettings {
                    base_url: base_url.to_owned(),
                    model: model.to_owned(),
                    referer: None,
                    region: None,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use AIOperation::*;
    use ProviderField::*;

    const ALL_OPS: [AIOperation; 6] = AIOperation::ALL;
    const TRANSLATE_ONLY: [AIOperation; 2] = [Translate, ScreenTranslate];

    #[test]
    fn provider_raw_values_are_frozen() {
        assert_eq!(
            ProviderType::ALL.map(|p| p.raw_value()),
            [
                "Claude",
                "OpenAI",
                "Gemini",
                "Grok",
                "Google (Free)",
                "Google Translate",
                "DeepL",
                "Microsoft Translator"
            ]
        );
    }

    #[test]
    fn serde_uses_raw_values() {
        for p in ProviderType::ALL {
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(json, format!("\"{}\"", p.raw_value()));
            assert_eq!(serde_json::from_str::<ProviderType>(&json).unwrap(), p);
            assert_eq!(ProviderType::from_raw_value(p.raw_value()), Some(p));
        }
    }

    #[test]
    fn unknown_raw_value_is_none_not_a_panic() {
        assert_eq!(ProviderType::from_raw_value("Bogus"), None);
        assert_eq!(ProviderType::from_raw_value(""), None);
        // containers skip these (Task 8)
        assert!(serde_json::from_str::<ProviderType>("\"Bogus\"").is_err());
    }

    #[test]
    fn display_name_equals_raw_value_today() {
        for p in ProviderType::ALL {
            assert_eq!(p.display_name(), p.raw_value());
        }
    }

    #[test]
    fn claude_capabilities() {
        assert_eq!(
            *ProviderType::Claude.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://api.anthropic.com",
                available_models: &[
                    "claude-fable-5-1",
                    "claude-opus-5-5",
                    "claude-sonnet-5-5",
                    "claude-haiku-5-5",
                    "claude-fable-5",
                    "claude-opus-5",
                    "claude-opus-4-8",
                    "claude-sonnet-5",
                    "claude-sonnet-4-6",
                    "claude-opus-4-7",
                ],
                requires_api_key: true,
                supported_operations: &ALL_OPS,
                supports_vision: true,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Model],
                preferred_default_model: Some("claude-haiku-5-5"),
            }
        );
    }

    #[test]
    fn openai_capabilities() {
        assert_eq!(
            *ProviderType::OpenAI.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://api.openai.com",
                available_models: &[
                    "gpt-6-astra",
                    "gpt-6.1-sol",
                    "gpt-6-luna",
                    "gpt-5.6-luna",
                    "gpt-5.6-terra",
                    "gpt-5.6-sol",
                    "gpt-5.5-pro",
                    "gpt-5.5",
                    "gpt-5.4",
                    "gpt-5.4-mini",
                ],
                requires_api_key: true,
                supported_operations: &ALL_OPS,
                supports_vision: true,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Model],
                preferred_default_model: Some("gpt-6-luna"),
            }
        );
    }

    #[test]
    fn gemini_capabilities() {
        assert_eq!(
            *ProviderType::Gemini.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://generativelanguage.googleapis.com",
                available_models: &[
                    "gemini-4-argon",
                    "gemini-3.8-flash",
                    "gemini-3.6-flash",
                    "gemini-3.5-flash-lite",
                    "gemini-3.1-pro-preview",
                    "gemini-3.1-flash-lite",
                    "gemini-2.5-flash-lite",
                    "gemini-2.5-flash",
                    "gemini-2.5-pro",
                ],
                requires_api_key: true,
                supported_operations: &ALL_OPS,
                supports_vision: true,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Model],
                preferred_default_model: Some("gemini-3.8-flash"),
            }
        );
    }

    #[test]
    fn grok_capabilities() {
        assert_eq!(
            *ProviderType::Grok.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://api.x.ai",
                available_models: &[
                    "grok-4.7",
                    "grok-4.6",
                    "grok-4.5",
                    "grok-4.3",
                    "grok-4.20-0309-reasoning",
                    "grok-4.20-0309-non-reasoning",
                    "grok-build-0.1",
                ],
                requires_api_key: true,
                supported_operations: &ALL_OPS,
                supports_vision: true,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Model],
                preferred_default_model: Some("grok-4.3"),
            }
        );
    }

    #[test]
    fn google_free_capabilities() {
        assert_eq!(
            *ProviderType::GoogleFree.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://translate.googleapis.com",
                available_models: &[],
                requires_api_key: false,
                supported_operations: &TRANSLATE_ONLY,
                supports_vision: false,
                auto_base_url: false,
                settings_fields: &[BaseUrl, Model],
                preferred_default_model: None,
            }
        );
    }

    #[test]
    fn google_cloud_capabilities() {
        assert_eq!(
            *ProviderType::GoogleCloud.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://translation.googleapis.com",
                available_models: &["nmt", "base"],
                requires_api_key: true,
                supported_operations: &TRANSLATE_ONLY,
                supports_vision: false,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Model, Referer],
                preferred_default_model: None,
            }
        );
    }

    #[test]
    fn deepl_capabilities() {
        assert_eq!(
            *ProviderType::DeepL.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://api.deepl.com",
                available_models: &[],
                requires_api_key: true,
                supported_operations: &TRANSLATE_ONLY,
                supports_vision: false,
                auto_base_url: true,
                settings_fields: &[ApiKey, BaseUrl, Model],
                preferred_default_model: None,
            }
        );
    }

    #[test]
    fn microsoft_capabilities() {
        assert_eq!(
            *ProviderType::MicrosoftTranslator.capabilities(),
            ProviderCapabilities {
                default_base_url: "https://api.cognitive.microsofttranslator.com",
                available_models: &[],
                requires_api_key: true,
                supported_operations: &TRANSLATE_ONLY,
                supports_vision: false,
                auto_base_url: false,
                settings_fields: &[ApiKey, BaseUrl, Region],
                preferred_default_model: None,
            }
        );
    }

    #[test]
    fn supports_follows_supported_operations() {
        for p in ProviderType::ALL {
            for op in AIOperation::ALL {
                assert_eq!(
                    p.supports(op),
                    p.capabilities().supported_operations.contains(&op),
                    "{p:?} / {op:?}"
                );
            }
        }
        assert!(ProviderType::Claude.supports(Magic));
        assert!(ProviderType::DeepL.supports(ScreenTranslate));
        assert!(!ProviderType::DeepL.supports(Rewrite));
        assert!(!ProviderType::GoogleFree.supports(Magic));
    }

    #[test]
    fn vision_providers_are_exactly_the_chat_providers() {
        let vision: Vec<_> = ProviderType::ALL
            .into_iter()
            .filter(|p| p.capabilities().supports_vision)
            .collect();
        assert_eq!(
            vision,
            [
                ProviderType::Claude,
                ProviderType::OpenAI,
                ProviderType::Gemini,
                ProviderType::Grok
            ]
        );
    }

    fn settings(base: &str, model: &str) -> ProviderConnectionSettings {
        ProviderConnectionSettings {
            base_url: base.into(),
            model: model.into(),
            referer: None,
            region: None,
        }
    }

    #[test]
    fn default_providers_has_all_eight_providers() {
        let defaults = default_providers();
        assert_eq!(defaults.len(), 8);
        assert_eq!(
            defaults.keys().copied().collect::<Vec<_>>(),
            ProviderType::ALL
        );
    }

    #[test]
    fn default_providers_match_settings_store() {
        let d = default_providers();
        assert_eq!(
            d[&ProviderType::Claude],
            settings("https://api.anthropic.com", "claude-haiku-5-5")
        );
        assert_eq!(
            d[&ProviderType::OpenAI],
            settings("https://api.openai.com", "gpt-6-luna")
        );
        assert_eq!(
            d[&ProviderType::Gemini],
            settings(
                "https://generativelanguage.googleapis.com",
                "gemini-3.8-flash"
            )
        );
        assert_eq!(
            d[&ProviderType::Grok],
            settings("https://api.x.ai", "grok-4.3")
        );
        assert_eq!(
            d[&ProviderType::GoogleFree],
            settings("https://translate.googleapis.com", "")
        );
        assert_eq!(
            d[&ProviderType::GoogleCloud],
            settings("https://translation.googleapis.com", "nmt")
        );
        // autoBaseURL: the endpoint is chosen by the ":fx" key suffix later.
        assert_eq!(d[&ProviderType::DeepL], settings("", ""));
        assert_eq!(
            d[&ProviderType::MicrosoftTranslator],
            settings("https://api.cognitive.microsofttranslator.com", "")
        );
    }

    #[test]
    fn connection_settings_omit_none_and_use_swift_field_names() {
        let s = settings("https://x", "m");
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"baseURL":"https://x","model":"m"}"#
        );
        let with = ProviderConnectionSettings {
            referer: Some("https://r".into()),
            region: Some("westeurope".into()),
            ..s
        };
        let json = serde_json::to_string(&with).unwrap();
        assert_eq!(
            json,
            r#"{"baseURL":"https://x","model":"m","referer":"https://r","region":"westeurope"}"#
        );
        assert_eq!(
            serde_json::from_str::<ProviderConnectionSettings>(&json).unwrap(),
            with
        );
        assert_eq!(
            serde_json::from_str::<ProviderConnectionSettings>(r#"{"baseURL":"a","model":"b"}"#)
                .unwrap(),
            settings("a", "b")
        );
    }

    #[test]
    fn provider_field_serializes_camel_case() {
        assert_eq!(serde_json::to_string(&ApiKey).unwrap(), "\"apiKey\"");
        assert_eq!(serde_json::to_string(&BaseUrl).unwrap(), "\"baseUrl\"");
    }
}
