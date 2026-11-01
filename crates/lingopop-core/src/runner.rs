//! The operation runner: one provider call with at most one fallback (port of
//! `Core/OperationRunner.swift`) and the vision path used by Screen Translate retry.
//!
//! The runner holds no state between calls. Everything it needs about settings, provider
//! connections and API keys is copied into a [`ConfigSnapshot`] up front, so no lock is held
//! across an `.await`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::errors::{FallbackInfo, TranslationError};
use crate::http::Transport;
use crate::operation::AIOperation;
use crate::provider::{default_providers, ProviderConnectionSettings, ProviderType};
use crate::providers::{ProviderConfig, ProviderRegistry, ProviderResult, TokenUsage};
use crate::records::{HistoryEntry, UsageRecord};
use crate::secrets::{ProviderSettingsState, SecretStore};
use crate::settings::{sanitize_magic_prompt, Settings};
use crate::time::Timestamp;

/// The part of the app state one run needs, copied out of `Settings`, the provider settings
/// and the `SecretStore`. `Debug` lists which providers have a key, never the keys.
#[derive(Clone)]
pub struct ConfigSnapshot {
    pub magic_prompt: String,
    pub fallbacks: BTreeMap<ProviderType, ProviderType>,
    pub connections: BTreeMap<ProviderType, ProviderConnectionSettings>,
    pub api_keys: BTreeMap<ProviderType, String>,
}

impl fmt::Debug for ConfigSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let providers_with_key: Vec<ProviderType> = self
            .api_keys
            .iter()
            .filter(|(_, key)| !key.is_empty())
            .map(|(provider, _)| *provider)
            .collect();
        f.debug_struct("ConfigSnapshot")
            .field("magic_prompt", &self.magic_prompt)
            .field("fallbacks", &self.fallbacks)
            .field("connections", &self.connections)
            .field("api_keys", &providers_with_key)
            .finish()
    }
}

impl ConfigSnapshot {
    /// Loaded provider settings are laid over `default_providers()` (an unreadable or missing
    /// entry means the defaults); the magic prompt is sanitized once here.
    pub fn from_parts(
        settings: &Settings,
        provider_settings: &ProviderSettingsState,
        secrets: &SecretStore,
    ) -> Self {
        let mut connections = default_providers();
        if let ProviderSettingsState::Loaded(loaded) = provider_settings {
            connections.extend(loaded.iter().map(|(p, c)| (*p, c.clone())));
        }
        let api_keys = ProviderType::ALL
            .into_iter()
            .filter_map(|provider| {
                let key = secrets.api_key(provider)?;
                (!key.is_empty()).then(|| (provider, key.to_owned()))
            })
            .collect();
        Self {
            magic_prompt: sanitize_magic_prompt(&settings.magic_prompt),
            fallbacks: settings.fallbacks.0.clone(),
            connections,
            api_keys,
        }
    }

    /// A snapshot with the default connections and no API keys, for an app whose secrets
    /// store could not be opened.
    pub fn without_keys(settings: &Settings) -> Self {
        Self {
            magic_prompt: sanitize_magic_prompt(&settings.magic_prompt),
            fallbacks: settings.fallbacks.0.clone(),
            connections: default_providers(),
            api_keys: BTreeMap::new(),
        }
    }

    /// Port of `SettingsStore.config(for:overrideTargetLanguage:)`: a provider without a
    /// connection gets an empty base URL and model, one without a key an empty key.
    pub fn config(&self, provider: ProviderType, target_language: &str) -> ProviderConfig {
        let connection = self.connections.get(&provider);
        ProviderConfig {
            provider_type: provider,
            api_key: self.api_keys.get(&provider).cloned().unwrap_or_default(),
            base_url: connection.map(|c| c.base_url.clone()).unwrap_or_default(),
            model: connection.map(|c| c.model.clone()).unwrap_or_default(),
            target_language: target_language.to_owned(),
            magic_prompt: self.magic_prompt.clone(),
            referer: connection.and_then(|c| c.referer.clone()),
            region: connection.and_then(|c| c.region.clone()),
        }
    }

    pub fn fallback(&self, provider: ProviderType) -> Option<ProviderType> {
        self.fallbacks.get(&provider).copied()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunRequest {
    pub operation: AIOperation,
    pub text: String,
    pub detected_source: Option<String>,
    pub target_language: String,
    pub primary: ProviderType,
    pub allow_fallback: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    pub target_language: String,
    /// The provider that answered (the fallback after a fallback).
    pub provider: ProviderType,
    pub operation: AIOperation,
    /// The model configured for `provider` at the time of the call.
    pub model: String,
    pub via_fallback: bool,
    pub fallback_info: Option<FallbackInfo>,
    pub truncated: bool,
    pub usage: Option<TokenUsage>,
    pub detected_source: Option<String>,
}

impl TranslationResult {
    /// The usage row for this call; `None` when the provider reported no token counts.
    pub fn usage_record(&self, now: Timestamp) -> Option<UsageRecord> {
        let usage = self.usage?;
        Some(UsageRecord::new(
            self.provider,
            self.model.clone(),
            self.operation,
            usage.input_tokens,
            usage.output_tokens,
            now,
        ))
    }

    pub fn history_entry(&self, now: Timestamp) -> HistoryEntry {
        HistoryEntry::new(
            self.original.clone(),
            self.translated.clone(),
            self.detected_source.clone(),
            self.target_language.clone(),
            self.provider,
            self.operation,
            now,
        )
    }
}

/// A fallback is tried only for rate limits (429), server errors (5xx) and network failures.
pub fn should_retry_with_fallback(error: &TranslationError) -> bool {
    match error {
        TranslationError::Http { status, .. } => *status == 429 || (500..=599).contains(status),
        TranslationError::Network(_) => true,
        TranslationError::MissingApiKey
        | TranslationError::InvalidUrl
        | TranslationError::Decoding(_)
        | TranslationError::EmptyResponse
        | TranslationError::OperationNotSupported { .. }
        | TranslationError::InvalidConfiguration(_)
        | TranslationError::ProviderNotFound(_) => false,
    }
}

pub struct OperationRunner {
    registry: ProviderRegistry,
    transport: Arc<dyn Transport>,
}

impl OperationRunner {
    pub fn new(registry: ProviderRegistry, transport: Arc<dyn Transport>) -> Self {
        Self {
            registry,
            transport,
        }
    }

    /// Runs `request` on its primary provider, then at most once on the configured fallback.
    ///
    /// The primary is validated first (registered, supports the operation, has a key if one
    /// is required); a validation error never falls back. The fallback is tried only when
    /// `allow_fallback` is set, it differs from the primary, the primary error passes
    /// [`should_retry_with_fallback`] and the fallback itself validates. Whatever goes wrong
    /// with the fallback, the PRIMARY error is returned.
    ///
    /// Cancellation is dropping the returned future: nothing is left running and no state
    /// is kept between calls.
    pub async fn run(
        &self,
        request: RunRequest,
        configs: &ConfigSnapshot,
    ) -> Result<TranslationResult, TranslationError> {
        let primary_config = configs.config(request.primary, &request.target_language);
        self.validate(request.primary, request.operation, &primary_config)?;
        let primary_error = match self
            .execute(request.primary, &request, &primary_config)
            .await
        {
            Ok(provider_result) => {
                return Ok(finish(
                    &request,
                    request.primary,
                    &primary_config,
                    provider_result,
                    None,
                ));
            }
            Err(error) => error,
        };
        if request.allow_fallback {
            if let Some(result) = self.try_fallback(&request, configs, &primary_error).await {
                return Ok(result);
            }
        }
        Err(primary_error)
    }

    /// Screen Translate retry: sends the captured image to a vision-capable provider for
    /// combined OCR and translation. No fallback; the provider performs its own key check.
    pub async fn run_vision(
        &self,
        provider: ProviderType,
        image: &[u8],
        original: String,
        target_language: String,
        configs: &ConfigSnapshot,
    ) -> Result<TranslationResult, TranslationError> {
        let implementation = self.registry.get(provider)?;
        if !provider.capabilities().supports_vision {
            return Err(TranslationError::OperationNotSupported {
                provider,
                operation: AIOperation::ScreenTranslate,
            });
        }
        let config = configs.config(provider, &target_language);
        let provider_result = implementation
            .execute_vision(self.transport.as_ref(), image, &target_language, &config)
            .await?;
        Ok(TranslationResult {
            original,
            translated: provider_result.text,
            target_language,
            provider,
            operation: AIOperation::ScreenTranslate,
            model: config.model,
            via_fallback: false,
            fallback_info: None,
            truncated: provider_result.truncated,
            usage: provider_result.usage,
            detected_source: None,
        })
    }

    async fn try_fallback(
        &self,
        request: &RunRequest,
        configs: &ConfigSnapshot,
        primary_error: &TranslationError,
    ) -> Option<TranslationResult> {
        let fallback = configs
            .fallback(request.primary)
            .filter(|fallback| *fallback != request.primary)?;
        if !should_retry_with_fallback(primary_error) {
            return None;
        }
        let config = configs.config(fallback, &request.target_language);
        self.validate(fallback, request.operation, &config).ok()?;
        let provider_result = self.execute(fallback, request, &config).await.ok()?;
        Some(finish(
            request,
            fallback,
            &config,
            provider_result,
            Some(FallbackInfo::new(request.primary, primary_error)),
        ))
    }

    fn validate(
        &self,
        provider: ProviderType,
        operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<(), TranslationError> {
        self.registry.get(provider)?;
        let capabilities = provider.capabilities();
        if !capabilities.supported_operations.contains(&operation) {
            return Err(TranslationError::OperationNotSupported {
                provider,
                operation,
            });
        }
        if capabilities.requires_api_key && config.api_key.is_empty() {
            return Err(TranslationError::MissingApiKey);
        }
        Ok(())
    }

    async fn execute(
        &self,
        provider: ProviderType,
        request: &RunRequest,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        self.registry
            .get(provider)?
            .execute(
                self.transport.as_ref(),
                &request.text,
                request.operation,
                config,
            )
            .await
    }
}

fn finish(
    request: &RunRequest,
    provider: ProviderType,
    config: &ProviderConfig,
    provider_result: ProviderResult,
    fallback_info: Option<FallbackInfo>,
) -> TranslationResult {
    TranslationResult {
        original: request.text.clone(),
        translated: provider_result.text,
        target_language: request.target_language.clone(),
        provider,
        operation: request.operation,
        model: config.model.clone(),
        via_fallback: fallback_info.is_some(),
        fallback_info,
        truncated: provider_result.truncated,
        usage: provider_result.usage,
        detected_source: request.detected_source.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::errors::{FallbackInfo, TranslationError};
    use crate::http::{FakeTransport, HttpResponse};
    use crate::operation::AIOperation;
    use crate::prompts::DEFAULT_MAGIC_PROMPT;
    use crate::provider::{default_providers, ProviderConnectionSettings, ProviderType};
    use crate::providers::{ClaudeProvider, ProviderRegistry, TokenUsage};
    use crate::secrets::{ProtectError, ProviderSettingsState, SecretProtector, SecretStore};
    use crate::settings::Settings;
    use crate::time::{ManualClock, Timestamp};
    use crate::ProviderKeyedMap;

    const CLAUDE_OK: &str = include_str!("../tests/fixtures/providers/claude/success.json");
    const OPENAI_OK: &str = include_str!("../tests/fixtures/providers/openai/success.json");
    const GOOGLE_FREE_OK: &str =
        include_str!("../tests/fixtures/providers/google_free/success.json");
    const CLAUDE_HOST: &str = "api.anthropic.com";
    const OPENAI_HOST: &str = "api.openai.com";
    const KEY: &str = "sk-ant-FIXTURE";

    use ProviderType::{Claude, DeepL, GoogleFree, OpenAI};

    struct PlainProtector;

    impl SecretProtector for PlainProtector {
        fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Ok(plain.to_vec())
        }

        fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Ok(data.to_vec())
        }
    }

    fn snapshot(
        keys: &[(ProviderType, &str)],
        fallbacks: &[(ProviderType, ProviderType)],
    ) -> ConfigSnapshot {
        ConfigSnapshot {
            magic_prompt: DEFAULT_MAGIC_PROMPT.to_string(),
            fallbacks: fallbacks.iter().copied().collect(),
            connections: default_providers(),
            api_keys: keys
                .iter()
                .map(|(provider, key)| (*provider, (*key).to_string()))
                .collect(),
        }
    }

    fn runner(fake: &Arc<FakeTransport>) -> OperationRunner {
        OperationRunner::new(ProviderRegistry::standard(), fake.clone())
    }

    fn req(operation: AIOperation, primary: ProviderType, allow_fallback: bool) -> RunRequest {
        RunRequest {
            operation,
            text: "hello".to_string(),
            detected_source: Some("English".to_string()),
            target_language: "Ukrainian".to_string(),
            primary,
            allow_fallback,
        }
    }

    fn resp(status: u16) -> HttpResponse {
        HttpResponse::new(status, r#"{"error":"nope"}"#)
    }

    fn http(status: u16) -> TranslationError {
        TranslationError::Http {
            status,
            body: String::new(),
            url: String::new(),
        }
    }

    fn assert_http_status(error: &TranslationError, expected: u16) {
        match error {
            TranslationError::Http { status, .. } => assert_eq!(*status, expected, "{error:?}"),
            other => panic!("expected Http {expected}, got {other:?}"),
        }
    }

    fn requests_to(fake: &FakeTransport, host: &str) -> usize {
        fake.requests()
            .iter()
            .filter(|r| r.url.host_str() == Some(host))
            .count()
    }

    fn claude_then_openai(claude: HttpResponse) -> Arc<FakeTransport> {
        Arc::new(FakeTransport::by_host(vec![
            (CLAUDE_HOST, claude),
            (OPENAI_HOST, HttpResponse::new(200, OPENAI_OK)),
        ]))
    }

    fn model_of(provider: ProviderType) -> String {
        default_providers()[&provider].model.clone()
    }

    fn both_keys() -> ConfigSnapshot {
        snapshot(
            &[(Claude, KEY), (OpenAI, "sk-FIXTURE")],
            &[(Claude, OpenAI)],
        )
    }

    fn secrets_with(keys: &[(ProviderType, &str)]) -> (tempfile::TempDir, SecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1.0)));
        let (mut store, _) = SecretStore::open(
            dir.path().join("secrets.dat"),
            Arc::new(PlainProtector),
            clock,
        )
        .unwrap();
        for (provider, key) in keys {
            store.set_api_key(*provider, key).unwrap();
        }
        (dir, store)
    }

    #[test]
    fn should_retry_table() {
        for status in [429, 500, 503, 599] {
            assert!(should_retry_with_fallback(&http(status)), "{status}");
        }
        assert!(should_retry_with_fallback(&TranslationError::Network(
            "down".into()
        )));
        for status in [400, 401, 403, 404, 499, 600] {
            assert!(!should_retry_with_fallback(&http(status)), "{status}");
        }
        for error in [
            TranslationError::Decoding("x".into()),
            TranslationError::EmptyResponse,
            TranslationError::MissingApiKey,
            TranslationError::InvalidUrl,
            TranslationError::InvalidConfiguration("x".into()),
            TranslationError::OperationNotSupported {
                provider: Claude,
                operation: AIOperation::Translate,
            },
            TranslationError::ProviderNotFound(Claude),
        ] {
            assert!(!should_retry_with_fallback(&error), "{error:?}");
        }
    }

    #[tokio::test]
    async fn run_success_without_fallback() {
        let fake = Arc::new(FakeTransport::json(200, CLAUDE_OK));
        let configs = snapshot(&[(Claude, KEY)], &[(Claude, OpenAI)]);
        let result = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &configs)
            .await
            .unwrap();
        assert_eq!(result.provider, Claude);
        assert!(!result.via_fallback);
        assert_eq!(result.fallback_info, None);
        assert_eq!(result.original, "hello");
        assert_eq!(result.translated, "Привіт, світе");
        assert_eq!(result.target_language, "Ukrainian");
        assert_eq!(result.operation, AIOperation::Translate);
        assert_eq!(result.model, model_of(Claude));
        assert_eq!(result.detected_source.as_deref(), Some("English"));
        assert_eq!(
            result.usage,
            Some(TokenUsage {
                input_tokens: 21,
                output_tokens: 7
            })
        );
        assert!(!result.truncated);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn run_429_falls_back_with_info() {
        let fake = claude_then_openai(resp(429));
        let result = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap();
        assert_eq!(result.provider, OpenAI);
        assert!(result.via_fallback);
        assert_eq!(result.model, model_of(OpenAI));
        assert_eq!(result.translated, "Привіт");
        let primary_error = TranslationError::Http {
            status: 429,
            body: r#"{"error":"nope"}"#.to_string(),
            url: "https://api.anthropic.com/v1/messages".to_string(),
        };
        assert_eq!(
            result.fallback_info,
            Some(FallbackInfo::new(Claude, &primary_error))
        );
        assert_eq!(fake.requests().len(), 2);
    }

    #[tokio::test]
    async fn run_503_falls_back() {
        let fake = claude_then_openai(resp(503));
        let result = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap();
        assert_eq!(result.provider, OpenAI);
        assert!(result.via_fallback);
    }

    #[tokio::test]
    async fn run_network_error_falls_back() {
        let fake = Arc::new(FakeTransport::new(|request| match request.url.host_str() {
            Some(CLAUDE_HOST) => Err(TranslationError::Network("down".into())),
            Some(OPENAI_HOST) => Ok(HttpResponse::new(200, OPENAI_OK)),
            _ => Err(TranslationError::Network("no route".into())),
        }));
        let result = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap();
        assert_eq!(result.provider, OpenAI);
        assert!(result.via_fallback);
        assert_eq!(
            result.fallback_info,
            Some(FallbackInfo::new(
                Claude,
                &TranslationError::Network("down".into())
            ))
        );
    }

    #[tokio::test]
    async fn run_401_does_not_fall_back() {
        let fake = claude_then_openai(resp(401));
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap_err();
        assert_http_status(&error, 401);
        assert_eq!(requests_to(&fake, OPENAI_HOST), 0);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn run_400_and_404_do_not_fall_back() {
        for status in [400, 404] {
            let fake = claude_then_openai(resp(status));
            let error = runner(&fake)
                .run(req(AIOperation::Translate, Claude, true), &both_keys())
                .await
                .unwrap_err();
            assert_http_status(&error, status);
            assert_eq!(fake.requests().len(), 1, "{status}");
        }
    }

    #[tokio::test]
    async fn run_decoding_and_empty_response_do_not_fall_back() {
        let empty = json!({"content": [{"type": "text", "text": "  "}], "stop_reason": "end_turn"})
            .to_string();
        let cases = [("{}".to_string(), "decoding"), (empty, "empty")];
        for (body, kind) in cases {
            let fake = claude_then_openai(HttpResponse::new(200, body));
            let error = runner(&fake)
                .run(req(AIOperation::Translate, Claude, true), &both_keys())
                .await
                .unwrap_err();
            match (kind, &error) {
                ("decoding", TranslationError::Decoding(_)) => {}
                ("empty", TranslationError::EmptyResponse) => {}
                _ => panic!("{kind}: unexpected {error:?}"),
            }
            assert_eq!(fake.requests().len(), 1, "{kind}");
        }
    }

    #[tokio::test]
    async fn run_allow_fallback_false_does_not_fall_back() {
        let fake = claude_then_openai(resp(429));
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, false), &both_keys())
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn run_without_configured_fallback_returns_primary_error() {
        let fake = claude_then_openai(resp(429));
        let configs = snapshot(&[(Claude, KEY), (OpenAI, "sk-FIXTURE")], &[]);
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &configs)
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn fallback_equal_to_primary_makes_no_second_request() {
        let fake = claude_then_openai(resp(429));
        let configs = snapshot(&[(Claude, KEY)], &[(Claude, Claude)]);
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &configs)
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn fallback_without_key_returns_primary_error_and_sends_nothing() {
        let fake = claude_then_openai(resp(429));
        let configs = snapshot(&[(Claude, KEY)], &[(Claude, OpenAI)]);
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &configs)
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(requests_to(&fake, OPENAI_HOST), 0);
    }

    #[tokio::test]
    async fn fallback_not_supporting_the_operation_returns_primary_error() {
        let fake = Arc::new(FakeTransport::by_host(vec![
            (CLAUDE_HOST, resp(429)),
            ("api.deepl.com", HttpResponse::new(200, "{}")),
        ]));
        let configs = snapshot(
            &[(Claude, KEY), (DeepL, "deepl-FIXTURE")],
            &[(Claude, DeepL)],
        );
        let error = runner(&fake)
            .run(req(AIOperation::Rewrite, Claude, true), &configs)
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(requests_to(&fake, "api.deepl.com"), 0);
    }

    #[tokio::test]
    async fn fallback_also_failing_returns_primary_error() {
        let fake = Arc::new(FakeTransport::by_host(vec![
            (CLAUDE_HOST, resp(429)),
            (OPENAI_HOST, resp(500)),
        ]));
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 2);
    }

    #[tokio::test]
    async fn fallback_missing_provider_returns_primary_error() {
        let fake = claude_then_openai(resp(429));
        let mut registry = ProviderRegistry::empty();
        registry.insert(Box::new(ClaudeProvider::new()));
        let error = OperationRunner::new(registry, fake.clone())
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn primary_unsupported_operation_is_not_supported_with_zero_requests() {
        let fake = Arc::new(FakeTransport::json(200, "{}"));
        let configs = snapshot(&[(DeepL, "deepl-FIXTURE")], &[]);
        let error = runner(&fake)
            .run(req(AIOperation::Rewrite, DeepL, true), &configs)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            TranslationError::OperationNotSupported {
                provider: DeepL,
                operation: AIOperation::Rewrite,
            }
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn primary_missing_key_is_missing_api_key_with_zero_requests() {
        let fake = claude_then_openai(resp(429));
        let configs = snapshot(&[(OpenAI, "sk-FIXTURE")], &[(Claude, OpenAI)]);
        let error = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &configs)
            .await
            .unwrap_err();
        assert_eq!(error, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn primary_not_registered_is_provider_not_found() {
        let fake = Arc::new(FakeTransport::json(200, CLAUDE_OK));
        let error = OperationRunner::new(ProviderRegistry::empty(), fake.clone())
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap_err();
        assert_eq!(error, TranslationError::ProviderNotFound(Claude));
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn google_free_needs_no_key() {
        let fake = Arc::new(FakeTransport::json(200, GOOGLE_FREE_OK));
        let result = runner(&fake)
            .run(
                req(AIOperation::Translate, GoogleFree, true),
                &snapshot(&[], &[]),
            )
            .await
            .unwrap();
        assert_eq!(result.provider, GoogleFree);
        assert_eq!(result.translated, "Доброго ранку, мій друже.");
        assert_eq!(result.usage, None);
    }

    #[tokio::test]
    async fn magic_prompt_is_sanitized_and_reaches_the_system_prompt() {
        let settings = Settings {
            magic_prompt: "  Fix {{targetLanguage}}\u{1}  ".to_string(),
            ..Settings::default()
        };
        let (_dir, store) = secrets_with(&[(Claude, KEY)]);
        let configs = ConfigSnapshot::from_parts(&settings, &ProviderSettingsState::Absent, &store);
        let fake = Arc::new(FakeTransport::json(200, CLAUDE_OK));
        runner(&fake)
            .run(req(AIOperation::Magic, Claude, false), &configs)
            .await
            .unwrap();
        let body = fake.requests()[0].json_body();
        let system = body["system"].as_str().unwrap();
        assert!(system.starts_with("Fix Ukrainian\n"), "{system:?}");
        assert!(!system.contains('\u{1}'), "{system:?}");
    }

    #[tokio::test]
    async fn target_language_override_reaches_the_request() {
        let fake = Arc::new(FakeTransport::json(200, CLAUDE_OK));
        let mut request = req(AIOperation::Translate, Claude, false);
        request.target_language = "German".to_string();
        let result = runner(&fake)
            .run(request, &snapshot(&[(Claude, KEY)], &[]))
            .await
            .unwrap();
        assert_eq!(result.target_language, "German");
        let body = fake.requests()[0].json_body();
        let system = body["system"].as_str().unwrap();
        assert!(system.contains("German"), "{system:?}");
        assert!(!system.contains("Ukrainian"), "{system:?}");
    }

    fn sample_result() -> TranslationResult {
        TranslationResult {
            original: "hello".to_string(),
            translated: "Привіт".to_string(),
            target_language: "Ukrainian".to_string(),
            provider: OpenAI,
            operation: AIOperation::Translate,
            model: "gpt-x".to_string(),
            via_fallback: true,
            fallback_info: None,
            truncated: false,
            usage: Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            }),
            detected_source: Some("English".to_string()),
        }
    }

    #[test]
    fn usage_record_maps_provider_model_operation_tokens() {
        let now = Timestamp::from_unix_seconds(1_800_000_000.0);
        let record = sample_result().usage_record(now).unwrap();
        assert_eq!(record.provider, "OpenAI");
        assert_eq!(record.model, "gpt-x");
        assert_eq!(record.operation, "translate");
        assert_eq!((record.input_tokens, record.output_tokens), (3, 4));
        assert_eq!(record.timestamp, now);

        let mut no_usage = sample_result();
        no_usage.usage = None;
        assert_eq!(no_usage.usage_record(now), None);
    }

    #[test]
    fn history_entry_maps_fields_and_detected_source() {
        let now = Timestamp::from_unix_seconds(1_800_000_000.0);
        let entry = sample_result().history_entry(now);
        assert_eq!(entry.original, "hello");
        assert_eq!(entry.translated, "Привіт");
        assert_eq!(entry.source_language.as_deref(), Some("English"));
        assert_eq!(entry.target_language, "Ukrainian");
        assert_eq!(entry.provider, "OpenAI");
        assert_eq!(entry.operation, "translate");
        assert_eq!(entry.timestamp, now);

        let mut no_source = sample_result();
        no_source.detected_source = None;
        assert_eq!(no_source.history_entry(now).source_language, None);
    }

    #[tokio::test]
    async fn history_entry_after_fallback_names_the_answering_provider() {
        let fake = claude_then_openai(resp(429));
        let result = runner(&fake)
            .run(req(AIOperation::Translate, Claude, true), &both_keys())
            .await
            .unwrap();
        let entry = result.history_entry(Timestamp::from_unix_seconds(1.0));
        assert_eq!(entry.provider, "OpenAI");
        assert_eq!(entry.source_language.as_deref(), Some("English"));
    }

    const PNG: [u8; 5] = [0x89, 0x50, 0x4E, 0x47, 1];

    fn vision_reply() -> String {
        json!({
            "content": [{"type": "text", "text": "  Hello  "}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 3, "output_tokens": 4}
        })
        .to_string()
    }

    #[tokio::test]
    async fn run_vision_on_claude_success() {
        let fake = Arc::new(FakeTransport::json(200, &vision_reply()));
        let result = runner(&fake)
            .run_vision(
                Claude,
                &PNG,
                "captured text".to_string(),
                "German".to_string(),
                &both_keys(),
            )
            .await
            .unwrap();
        assert_eq!(result.provider, Claude);
        assert_eq!(result.operation, AIOperation::ScreenTranslate);
        assert_eq!(result.original, "captured text");
        assert_eq!(result.translated, "Hello");
        assert_eq!(result.target_language, "German");
        assert_eq!(result.model, model_of(Claude));
        assert!(result.truncated);
        assert!(!result.via_fallback);
        assert_eq!(result.fallback_info, None);
        assert_eq!(result.detected_source, None);
        assert_eq!(
            result.usage,
            Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4
            })
        );
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let body = requests[0].json_body();
        assert_eq!(
            body["messages"][0]["content"][0]["source"]["media_type"],
            "image/png"
        );
    }

    #[tokio::test]
    async fn run_vision_on_deepl_is_operation_not_supported() {
        let fake = Arc::new(FakeTransport::json(200, "{}"));
        let configs = snapshot(&[(DeepL, "deepl-FIXTURE")], &[]);
        let error = runner(&fake)
            .run_vision(DeepL, &PNG, String::new(), "German".to_string(), &configs)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            TranslationError::OperationNotSupported {
                provider: DeepL,
                operation: AIOperation::ScreenTranslate,
            }
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn run_vision_does_not_fall_back() {
        let fake = claude_then_openai(resp(429));
        let error = runner(&fake)
            .run_vision(
                Claude,
                &PNG,
                String::new(),
                "German".to_string(),
                &both_keys(),
            )
            .await
            .unwrap_err();
        assert_http_status(&error, 429);
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn run_vision_missing_key_is_missing_api_key() {
        let fake = Arc::new(FakeTransport::json(200, &vision_reply()));
        let error = runner(&fake)
            .run_vision(
                Claude,
                &PNG,
                String::new(),
                "German".to_string(),
                &snapshot(&[], &[]),
            )
            .await
            .unwrap_err();
        assert_eq!(error, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn config_snapshot_from_parts_loaded() {
        let (_dir, store) = secrets_with(&[(Claude, KEY), (OpenAI, "sk-FIXTURE")]);
        let claude = ProviderConnectionSettings {
            base_url: "https://proxy.example.com/litellm".to_string(),
            model: "m".to_string(),
            referer: Some("https://ref.example.com".to_string()),
            region: None,
        };
        let loaded = ProviderSettingsState::Loaded(BTreeMap::from([(Claude, claude.clone())]));
        let settings = Settings {
            fallbacks: ProviderKeyedMap(BTreeMap::from([(Claude, OpenAI)])),
            ..Settings::default()
        };
        let snapshot = ConfigSnapshot::from_parts(&settings, &loaded, &store);
        assert_eq!(snapshot.connections[&Claude], claude);
        let defaults = default_providers();
        for provider in ProviderType::ALL.into_iter().filter(|p| *p != Claude) {
            assert_eq!(snapshot.connections[&provider], defaults[&provider]);
        }
        assert_eq!(snapshot.connections.len(), 8);
        assert_eq!(snapshot.api_keys[&Claude], KEY);
        assert_eq!(snapshot.api_keys[&OpenAI], "sk-FIXTURE");
        assert_eq!(snapshot.api_keys.len(), 2);
        assert_eq!(snapshot.fallback(Claude), Some(OpenAI));
        assert_eq!(snapshot.fallback(OpenAI), None);
        assert_eq!(snapshot.magic_prompt, DEFAULT_MAGIC_PROMPT);
        let config = snapshot.config(Claude, "German");
        assert_eq!(config.base_url, "https://proxy.example.com/litellm");
        assert_eq!(config.referer.as_deref(), Some("https://ref.example.com"));
        assert_eq!(config.api_key, KEY);
    }

    #[test]
    fn config_snapshot_from_parts_absent_and_corrupt() {
        let (_dir, store) = secrets_with(&[]);
        for state in [
            ProviderSettingsState::Absent,
            ProviderSettingsState::Corrupt,
        ] {
            let snapshot = ConfigSnapshot::from_parts(&Settings::default(), &state, &store);
            assert_eq!(snapshot.connections, default_providers());
        }
    }

    #[test]
    fn config_snapshot_config_defaults() {
        let snapshot = ConfigSnapshot {
            magic_prompt: "my magic".to_string(),
            fallbacks: BTreeMap::new(),
            connections: BTreeMap::new(),
            api_keys: BTreeMap::new(),
        };
        let config = snapshot.config(Claude, "German");
        assert_eq!(config.provider_type, Claude);
        assert_eq!(config.base_url, "");
        assert_eq!(config.model, "");
        assert_eq!(config.api_key, "");
        assert_eq!(config.target_language, "German");
        assert_eq!(config.magic_prompt, "my magic");
        assert_eq!(config.referer, None);
        assert_eq!(config.region, None);
    }

    #[test]
    fn config_snapshot_debug_redacts_api_keys() {
        let debug = format!("{:?}", snapshot(&[(Claude, "sk-ant-FIXTURE")], &[]));
        assert!(!debug.contains("sk-ant-FIXTURE"), "{debug}");
        assert!(debug.contains("Claude"), "{debug}");
    }

    #[test]
    fn config_snapshot_debug_strips_url_secrets_from_connections() {
        let mut snap = snapshot(&[], &[]);
        snap.connections.insert(
            Claude,
            ProviderConnectionSettings {
                base_url: "https://user:PWSENTINEL@proxy.example:8443/v1?key=QSENTINEL#FSENTINEL"
                    .into(),
                model: "m-1".into(),
                referer: Some(
                    "https://ref-user:RPWSENTINEL@ref.example/app?t=RQSENTINEL#RFSENTINEL".into(),
                ),
                region: Some("westeurope".into()),
            },
        );
        snap.connections.insert(
            OpenAI,
            ProviderConnectionSettings {
                base_url: "not a url SENTINEL-BASE".into(),
                model: "m-2".into(),
                referer: None,
                region: None,
            },
        );
        let debug = format!("{snap:?}");
        for sentinel in [
            "PWSENTINEL",
            "QSENTINEL",
            "FSENTINEL",
            "RPWSENTINEL",
            "RQSENTINEL",
            "RFSENTINEL",
            "SENTINEL-BASE",
        ] {
            assert!(!debug.contains(sentinel), "{sentinel} leaked: {debug}");
        }
        assert!(debug.contains("https://proxy.example:8443/"), "{debug}");
        assert!(debug.contains("https://ref.example/"), "{debug}");
        assert!(debug.contains("<unparseable, 23 chars>"), "{debug}");
        assert!(
            debug.contains("m-1") && debug.contains("westeurope"),
            "{debug}"
        );
    }
}
