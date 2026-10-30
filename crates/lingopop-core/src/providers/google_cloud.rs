//! Google Cloud Translation v2 (API key), port of `GoogleCloudProvider.swift`. The key
//! travels in the `X-goog-api-key` header, never in the URL. Screen Translate runs the OCR
//! text through the same call; unsupported operations are rejected by the runner.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{AIProvider, ProviderConfig, ProviderResult};
use crate::errors::TranslationError;
use crate::http::{decode, endpoint, json_post, perform, Transport};
use crate::languages;
use crate::operation::AIOperation;
use crate::provider::ProviderType;

#[derive(Debug, Default)]
pub struct GoogleCloudProvider;

impl GoogleCloudProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AIProvider for GoogleCloudProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GoogleCloud
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        _operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = ProviderType::GoogleCloud.capabilities();
        let url = endpoint(
            &config.base_url,
            caps.default_base_url,
            "/language/translate/v2",
            &[],
        )?;
        let body = RequestBody {
            q: text,
            target: languages::code(&config.target_language),
            model: (!config.model.is_empty()).then_some(config.model.as_str()),
            format: "text",
        };
        let mut headers = vec![("X-goog-api-key", config.api_key.as_str())];
        if let Some(referer) = config.referer.as_deref().filter(|r| !r.is_empty()) {
            headers.push(("Referer", referer));
        }
        let request = json_post(url, &headers, &body)?;
        let bytes = perform(transport, request).await?;
        let decoded: ResponseBody = decode(&bytes)?;
        // Quirk kept from Swift: emptiness is checked before trimming.
        let translated = decoded
            .data
            .translations
            .into_iter()
            .next()
            .map(|t| t.translated_text)
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
struct RequestBody<'a> {
    q: &'a str,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    format: &'static str,
}

#[derive(Deserialize)]
struct ResponseBody {
    data: DataField,
}

#[derive(Deserialize)]
struct DataField {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Translation {
    translated_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::prompts::DEFAULT_MAGIC_PROMPT;
    use serde_json::json;

    const KEY: &str = "AIza-FIXTURE";
    const URL: &str = "https://translation.googleapis.com/language/translate/v2";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/google_cloud/success.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::GoogleCloud,
            api_key: KEY.into(),
            base_url: "https://translation.googleapis.com".into(),
            model: "nmt".into(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn reply(text: &str) -> String {
        json!({"data": {"translations": [{"translatedText": text}]}}).to_string()
    }

    async fn run_with(
        fake: &FakeTransport,
        cfg: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        GoogleCloudProvider::new()
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
    async fn google_cloud_translate_request_wire_format() {
        let fake = FakeTransport::json(200, SUCCESS);
        run(&fake).await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(request.url.query(), None);
        let mut expected = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-goog-api-key".to_string(), KEY.to_string()),
        ];
        expected.sort();
        assert_eq!(sorted_headers(request), expected);
        assert_eq!(request.header("Referer"), None);
        assert_eq!(
            request.json_body(),
            json!({"q": "hello", "target": "uk", "model": "nmt", "format": "text"})
        );
    }

    #[tokio::test]
    async fn google_cloud_omits_model_when_empty() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.model = String::new();
        run_with(&fake, &cfg).await.unwrap();
        let body = fake.requests()[0].json_body();
        assert!(body.get("model").is_none(), "{body}");
        assert_eq!(
            body,
            json!({"q": "hello", "target": "uk", "format": "text"})
        );
    }

    #[tokio::test]
    async fn google_cloud_sends_referer_only_when_set() {
        for referer in [None, Some(String::new())] {
            let fake = FakeTransport::json(200, SUCCESS);
            let mut cfg = config();
            cfg.referer = referer.clone();
            run_with(&fake, &cfg).await.unwrap();
            assert_eq!(fake.requests()[0].header("Referer"), None, "{referer:?}");
        }
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.referer = Some("https://app.example.com".into());
        run_with(&fake, &cfg).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.header("Referer"), Some("https://app.example.com"));
        assert_eq!(request.header("X-goog-api-key"), Some(KEY));
        assert_eq!(request.url.as_str(), URL);
    }

    #[tokio::test]
    async fn google_cloud_success_parses_text() {
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
    async fn google_cloud_empty_response() {
        let empty_list = json!({"data": {"translations": []}}).to_string();
        for body in [empty_list, reply("")] {
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
    async fn google_cloud_missing_required_key_is_decoding_error() {
        for body in [r#"{"data":{}}"#, "{}"] {
            let fake = FakeTransport::json(200, body);
            let err = run(&fake).await.unwrap_err();
            assert!(
                matches!(err, TranslationError::Decoding(_)),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn google_cloud_error_object_body_is_decoding() {
        let body = json!({"error": {"code": 403, "message": "x"}}).to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake).await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn google_cloud_http_error_passes_status_body_and_redacted_url() {
        let fake = FakeTransport::json(403, "forbidden");
        let err = run(&fake).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 403,
                body: "forbidden".into(),
                url: URL.into(),
            }
        );
    }

    #[tokio::test]
    async fn google_cloud_rejects_http_base_url() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = "http://translation.googleapis.com".into();
        let err = run_with(&fake, &cfg).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".into())
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn google_cloud_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/language/translate/v2"
        );
    }

    #[tokio::test]
    async fn google_cloud_error_text_never_contains_api_key() {
        // An upstream that echoes the key back in the error body.
        let echoed = format!("API key not valid: {KEY}");
        let fake = FakeTransport::json(400, &echoed);
        let mut cfg = config();
        cfg.referer = Some("https://app.example.com".into());
        let err = run_with(&fake, &cfg).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 400,
                body: echoed,
                url: URL.into(),
            }
        );
        let text = format!("{err} {err:?}");
        assert!(!text.contains(KEY), "{text}");
        // Nor does the key reach the URL or the request Debug output.
        let request = &fake.requests()[0];
        assert!(!request.url.as_str().contains(KEY));
        assert!(!format!("{request:?}").contains(KEY));
    }
}
