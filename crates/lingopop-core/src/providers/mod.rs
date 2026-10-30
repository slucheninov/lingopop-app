//! The provider contract: `AIProvider`, the value types every provider shares, the
//! `ProviderRegistry`, and the small helpers (`finish_text`, `require_api_key`,
//! `encode_image`) the provider implementations call. Capabilities are not part of the
//! trait: the single source is `ProviderType::capabilities()`.

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::errors::TranslationError;
use crate::http::{mime_type, Transport};
use crate::operation::AIOperation;
use crate::prompts::sanitize_output;
use crate::provider::ProviderType;

pub mod claude;
pub mod gemini;
pub mod openai;

pub use claude::ClaudeProvider;
pub use gemini::GeminiProvider;
pub use openai::OpenAiCompatible;

/// Everything a provider needs for one call. `Debug` never prints the API key.
#[derive(Clone)]
pub struct ProviderConfig {
    pub provider_type: ProviderType,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub target_language: String,
    pub magic_prompt: String,
    pub referer: Option<String>,
    pub region: Option<String>,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("provider_type", &self.provider_type)
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("target_language", &self.target_language)
            .field("magic_prompt", &self.magic_prompt)
            .field("referer", &self.referer)
            .field("region", &self.region)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderResult {
    pub text: String,
    pub usage: Option<TokenUsage>,
    /// The model stopped because it hit its output limit.
    pub truncated: bool,
}

#[async_trait]
pub trait AIProvider: Send + Sync {
    fn provider_type(&self) -> ProviderType;

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError>;

    /// Combined OCR + translation of a captured image. Only vision-capable providers
    /// override this.
    async fn execute_vision(
        &self,
        _transport: &dyn Transport,
        _image: &[u8],
        _target_language: &str,
        _config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        Err(TranslationError::OperationNotSupported {
            provider: self.provider_type(),
            operation: AIOperation::ScreenTranslate,
        })
    }
}

/// The providers by type.
pub struct ProviderRegistry {
    providers: HashMap<ProviderType, Box<dyn AIProvider>>,
}

impl ProviderRegistry {
    pub fn empty() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Every built-in provider.
    pub fn standard() -> Self {
        let mut registry = Self::empty();
        registry.insert(Box::new(ClaudeProvider::new()));
        registry.insert(Box::new(OpenAiCompatible::openai()));
        registry.insert(Box::new(OpenAiCompatible::grok()));
        registry.insert(Box::new(GeminiProvider::new()));
        registry
    }

    /// Registers `provider` under its `provider_type()`, replacing an earlier one.
    pub fn insert(&mut self, provider: Box<dyn AIProvider>) {
        self.providers.insert(provider.provider_type(), provider);
    }

    pub fn get(&self, provider_type: ProviderType) -> Result<&dyn AIProvider, TranslationError> {
        self.providers
            .get(&provider_type)
            .map(Box::as_ref)
            .ok_or(TranslationError::ProviderNotFound(provider_type))
    }
}

/// The final text of a response: `sanitize_output` for text operations, plain `trim` for
/// vision; an empty result is `EmptyResponse`.
pub(crate) fn finish_text(raw: &str, sanitize: bool) -> Result<String, TranslationError> {
    let text = if sanitize {
        sanitize_output(raw)
    } else {
        raw.trim().to_string()
    };
    if text.is_empty() {
        Err(TranslationError::EmptyResponse)
    } else {
        Ok(text)
    }
}

pub(crate) fn require_api_key(config: &ProviderConfig) -> Result<(), TranslationError> {
    if config.api_key.is_empty() {
        Err(TranslationError::MissingApiKey)
    } else {
        Ok(())
    }
}

/// The MIME type sniffed from `image` and its standard base64 (with padding).
pub(crate) fn encode_image(image: &[u8]) -> (&'static str, String) {
    (mime_type(image), STANDARD.encode(image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FakeTransport;
    use crate::prompts::DEFAULT_MAGIC_PROMPT;

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::Claude,
            api_key: "sk-ant-FIXTURE".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-5-5".into(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: Some("https://ref.example.com".into()),
            region: Some("westeurope".into()),
        }
    }

    /// A provider that only implements the required methods, to exercise the trait defaults.
    struct Dummy;

    #[async_trait]
    impl AIProvider for Dummy {
        fn provider_type(&self) -> ProviderType {
            ProviderType::DeepL
        }

        async fn execute(
            &self,
            _transport: &dyn Transport,
            _text: &str,
            _operation: AIOperation,
            _config: &ProviderConfig,
        ) -> Result<ProviderResult, TranslationError> {
            Err(TranslationError::EmptyResponse)
        }
    }

    #[test]
    fn provider_config_debug_redacts_api_key() {
        let debug = format!("{:?}", config());
        assert!(!debug.contains("sk-ant-FIXTURE"), "{debug}");
        assert!(debug.contains("api_key: \"<redacted>\""), "{debug}");
        assert!(debug.contains("claude-sonnet-5-5"), "{debug}");
        assert!(debug.contains("Ukrainian"), "{debug}");
        assert!(debug.contains("https://api.anthropic.com"), "{debug}");
        assert!(debug.contains("westeurope"), "{debug}");
    }

    #[tokio::test]
    async fn default_execute_vision_is_operation_not_supported() {
        let fake = FakeTransport::json(200, "{}");
        let err = Dummy
            .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &config())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            TranslationError::OperationNotSupported {
                provider: ProviderType::DeepL,
                operation: AIOperation::ScreenTranslate,
            }
        );
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn registry_get_missing_is_provider_not_found() {
        let registry = ProviderRegistry::empty();
        assert!(matches!(
            registry.get(ProviderType::Claude),
            Err(TranslationError::ProviderNotFound(ProviderType::Claude))
        ));
    }

    #[test]
    fn registry_insert_then_get_returns_provider() {
        let mut registry = ProviderRegistry::empty();
        registry.insert(Box::new(Dummy));
        let provider = registry.get(ProviderType::DeepL).unwrap();
        assert_eq!(provider.provider_type(), ProviderType::DeepL);
        assert!(registry.get(ProviderType::Claude).is_err());
    }

    #[test]
    fn registry_standard_registers_claude() {
        let registry = ProviderRegistry::standard();
        let provider = registry.get(ProviderType::Claude).unwrap();
        assert_eq!(provider.provider_type(), ProviderType::Claude);
    }

    #[test]
    fn registry_standard_registers_openai_and_grok() {
        let registry = ProviderRegistry::standard();
        for provider_type in [ProviderType::OpenAI, ProviderType::Grok] {
            let provider = registry.get(provider_type).unwrap();
            assert_eq!(provider.provider_type(), provider_type);
        }
    }

    #[test]
    fn registry_standard_registers_gemini() {
        let registry = ProviderRegistry::standard();
        let provider = registry.get(ProviderType::Gemini).unwrap();
        assert_eq!(provider.provider_type(), ProviderType::Gemini);
    }

    #[test]
    fn finish_text_sanitizes_or_trims_and_rejects_empty() {
        let raw = "<text>\nhi &amp; bye\n</text>";
        assert_eq!(finish_text(raw, true).unwrap(), "hi & bye");
        assert_eq!(finish_text(raw, false).unwrap(), raw);
        assert_eq!(finish_text("  x \n", false).unwrap(), "x");
        for sanitize in [true, false] {
            assert_eq!(
                finish_text("  \n", sanitize).unwrap_err(),
                TranslationError::EmptyResponse
            );
        }
    }

    #[test]
    fn require_api_key_rejects_empty() {
        let mut cfg = config();
        assert!(require_api_key(&cfg).is_ok());
        cfg.api_key = String::new();
        assert_eq!(
            require_api_key(&cfg).unwrap_err(),
            TranslationError::MissingApiKey
        );
    }

    #[test]
    fn encode_image_uses_standard_base64_with_padding_and_sniffed_mime() {
        assert_eq!(
            encode_image(&[0x89, 0x50, 0x4E, 0x47, 1]),
            ("image/png", "iVBORwE=".to_string())
        );
        assert_eq!(
            encode_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0]),
            ("image/jpeg", "/9j/4AA=".to_string())
        );
    }
}
