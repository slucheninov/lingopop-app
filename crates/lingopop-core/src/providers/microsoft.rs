//! Microsoft Translator v3 (`/translate?api-version=3.0&to=<code>`), port of
//! `MicrosoftTranslatorProvider.swift`. The key travels in the `Ocp-Apim-Subscription-Key`
//! header (plus `Ocp-Apim-Subscription-Region` when a region is configured), never in the
//! URL. Screen Translate runs the OCR text through the same call; unsupported operations
//! are rejected by the runner.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{AIProvider, ProviderConfig, ProviderResult};
use crate::errors::TranslationError;
use crate::http::{decode, endpoint, json_post, perform, Transport};
use crate::languages;
use crate::operation::AIOperation;
use crate::provider::ProviderType;

#[derive(Debug, Default)]
pub struct MicrosoftProvider;

impl MicrosoftProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AIProvider for MicrosoftProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::MicrosoftTranslator
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        _operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = ProviderType::MicrosoftTranslator.capabilities();
        let code = languages::microsoft_code(&config.target_language);
        let url = endpoint(
            &config.base_url,
            caps.default_base_url,
            "/translate",
            &[("api-version", "3.0"), ("to", code.as_str())],
        )?;
        let mut headers = vec![("Ocp-Apim-Subscription-Key", config.api_key.as_str())];
        if let Some(region) = config.region.as_deref().filter(|r| !r.is_empty()) {
            headers.push(("Ocp-Apim-Subscription-Region", region));
        }
        let request = json_post(url, &headers, &[RequestItem { text }])?;
        let bytes = perform(transport, request).await?;
        let decoded: Vec<ResponseItem> = decode(&bytes)?;
        // Quirk kept from Swift: emptiness is checked before trimming.
        let translated = decoded
            .into_iter()
            .next()
            .and_then(|item| item.translations.into_iter().next())
            .map(|t| t.text)
            .filter(|t| !t.is_empty())
            .ok_or(TranslationError::EmptyResponse)?;
        Ok(ProviderResult {
            text: translated.trim().to_string(),
            usage: None,
            truncated: false,
        })
    }
}

#[derive(Serialize)]
struct RequestItem<'a> {
    #[serde(rename = "Text")]
    text: &'a str,
}

#[derive(Deserialize)]
struct ResponseItem {
    translations: Vec<Translation>,
}

/// `to` is required by the Swift decoder, so a reply without it is a decoding error even
/// though the value is not used.
#[derive(Deserialize)]
struct Translation {
    text: String,
    #[allow(dead_code)]
    to: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::prompts::DEFAULT_MAGIC_PROMPT;
    use serde_json::json;

    const KEY: &str = "ms-FIXTURE";
    const URL: &str =
        "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=uk";
    const BARE_URL: &str = "https://api.cognitive.microsofttranslator.com/translate";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/microsoft/success.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::MicrosoftTranslator,
            api_key: KEY.into(),
            base_url: "https://api.cognitive.microsofttranslator.com".into(),
            model: String::new(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn reply(text: &str) -> String {
        json!([{"translations": [{"text": text, "to": "uk"}]}]).to_string()
    }

    async fn run_with(
        fake: &FakeTransport,
        cfg: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        MicrosoftProvider::new()
            .execute(fake, "hello", AIOperation::Translate, cfg)
            .await
    }

    async fn run(fake: &FakeTransport) -> Result<ProviderResult, TranslationError> {
        run_with(fake, &config()).await
    }

    fn sorted_headers(request: &crate::http::HttpRequest) -> Vec<(String, String)> {
        let mut headers = request.headers.clone();
        headers.sort();
        headers
    }

    #[tokio::test]
    async fn microsoft_translate_request_wire_format() {
        let fake = FakeTransport::json(200, SUCCESS);
        run(&fake).await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        let mut expected = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Ocp-Apim-Subscription-Key".to_string(), KEY.to_string()),
        ];
        expected.sort();
        assert_eq!(sorted_headers(request), expected);
        assert_eq!(request.json_body(), json!([{"Text": "hello"}]));
    }

    #[tokio::test]
    async fn microsoft_empty_base_uses_default_host() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = String::new();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(fake.requests()[0].url.as_str(), URL);
    }

    #[tokio::test]
    async fn microsoft_uses_microsoft_language_codes() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.target_language = "Chinese".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            format!("{BARE_URL}?api-version=3.0&to=zh-Hans")
        );
    }

    #[tokio::test]
    async fn microsoft_sends_region_only_when_set() {
        for region in [None, Some(String::new())] {
            let fake = FakeTransport::json(200, SUCCESS);
            let mut cfg = config();
            cfg.region = region.clone();
            run_with(&fake, &cfg).await.unwrap();
            assert_eq!(
                fake.requests()[0].header("Ocp-Apim-Subscription-Region"),
                None,
                "{region:?}"
            );
        }
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.region = Some("westeurope".into());
        run_with(&fake, &cfg).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(
            request.header("Ocp-Apim-Subscription-Region"),
            Some("westeurope")
        );
        assert_eq!(request.header("Ocp-Apim-Subscription-Key"), Some(KEY));
        assert_eq!(request.url.as_str(), URL);
    }

    #[tokio::test]
    async fn microsoft_success_parses_text() {
        let fake = FakeTransport::json(200, SUCCESS);
        let result = run(&fake).await.unwrap();
        assert_eq!(
            result,
            ProviderResult {
                text: "Доброго ранку, мій друже.".into(),
                usage: None,
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn microsoft_ignores_the_operation_and_trims() {
        let fake = FakeTransport::json(200, &reply("  hi \n"));
        let result = MicrosoftProvider::new()
            .execute(&fake, "hello", AIOperation::ScreenTranslate, &config())
            .await
            .unwrap();
        assert_eq!(result.text, "hi");
    }

    #[tokio::test]
    async fn microsoft_empty_response() {
        let no_translations = json!([{"translations": []}]).to_string();
        for body in ["[]".to_string(), no_translations, reply("")] {
            let fake = FakeTransport::json(200, &body);
            assert_eq!(
                run(&fake).await.unwrap_err(),
                TranslationError::EmptyResponse,
                "{body}"
            );
        }
        // Quirk kept: emptiness is checked before trimming.
        let fake = FakeTransport::json(200, &reply("  "));
        assert_eq!(run(&fake).await.unwrap().text, "");
    }

    #[tokio::test]
    async fn microsoft_missing_required_key_is_decoding_error() {
        for body in [r#"[{"translations":[{"text":"x"}]}]"#, "[{}]"] {
            let fake = FakeTransport::json(200, body);
            let err = run(&fake).await.unwrap_err();
            assert!(
                matches!(err, TranslationError::Decoding(_)),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn microsoft_error_object_body_is_decoding() {
        let body = json!({"error": {"code": 401000, "message": "x"}}).to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake).await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn microsoft_http_error_passes_status_body_and_redacted_url() {
        let fake = FakeTransport::json(401, "unauthorized");
        let err = run(&fake).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 401,
                body: "unauthorized".into(),
                url: BARE_URL.into(),
            }
        );
    }

    #[tokio::test]
    async fn microsoft_rejects_http_base_url() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = "http://api.cognitive.microsofttranslator.com".into();
        let err = run_with(&fake, &cfg).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".into())
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn microsoft_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/translate?api-version=3.0&to=uk"
        );
    }

    #[tokio::test]
    async fn microsoft_error_text_never_contains_api_key() {
        // An upstream that echoes the key back in the error body.
        let echoed = format!("Invalid subscription key: {KEY}");
        let fake = FakeTransport::json(401, &echoed);
        let mut cfg = config();
        cfg.region = Some("westeurope".into());
        let err = run_with(&fake, &cfg).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 401,
                body: echoed,
                url: BARE_URL.into(),
            }
        );
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains(KEY), "{shown}");
        // Nor does the key reach the URL or the request Debug output.
        let request = &fake.requests()[0];
        assert!(!request.url.as_str().contains(KEY));
        assert!(!format!("{request:?}").contains(KEY));
    }

    #[tokio::test]
    async fn microsoft_malformed_200_echoing_the_key_does_not_leak_it() {
        for body in [
            json!(KEY).to_string(),
            json!([KEY]).to_string(),
            json!([{"translations": KEY}]).to_string(),
            json!([{"translations": [{"text": [KEY], "to": "uk"}]}]).to_string(),
        ] {
            let fake = FakeTransport::json(200, &body);
            let err = run(&fake).await.unwrap_err();
            assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
            let shown = format!(
                "{err} {err:?} {} {:?}",
                err.user_summary().english(),
                err.user_detail()
            );
            assert!(!shown.contains(KEY), "{body}: {shown}");
        }
    }
}
