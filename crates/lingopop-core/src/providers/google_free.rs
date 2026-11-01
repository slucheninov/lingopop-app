//! Google Translate, unofficial `translate_a/single` endpoint (no API key), port of
//! `GoogleFreeProvider.swift`. Translate only: Screen Translate runs the OCR text through
//! the same call, and the runner rejects unsupported operations before getting here.

use async_trait::async_trait;
use serde_json::Value;

use super::{AIProvider, ProviderConfig, ProviderResult};
use crate::errors::TranslationError;
use crate::http::{endpoint, perform, HttpRequest, Transport};
use crate::languages;
use crate::operation::AIOperation;
use crate::provider::ProviderType;

#[derive(Debug, Default)]
pub struct GoogleFreeProvider;

impl GoogleFreeProvider {
    pub fn new() -> Self {
        Self
    }
}

/// The first string element of every segment that is an array, concatenated. The response
/// is `[[[translated, original, ...], ...], ...]`.
fn join_segments(response: &Value) -> Result<String, TranslationError> {
    let segments = response
        .as_array()
        .and_then(|outer| outer.first())
        .and_then(Value::as_array)
        .ok_or_else(|| TranslationError::Decoding("Unexpected response shape".into()))?;
    Ok(segments
        .iter()
        .filter_map(|segment| segment.as_array()?.first()?.as_str())
        .collect())
}

#[async_trait]
impl AIProvider for GoogleFreeProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::GoogleFree
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        _operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = ProviderType::GoogleFree.capabilities();
        let code = languages::code(&config.target_language);
        // `endpoint` form-encodes the pairs, so "+", "&" and "%" in the text survive
        // (Swift sends "+" raw and Google reads it as a space; this port fixes that).
        let url = endpoint(
            &config.base_url,
            caps.default_base_url,
            "/translate_a/single",
            &[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", &code),
                ("dt", "t"),
                ("q", text),
            ],
        )?;
        // Google answers 429 to hyper's HTTP/2 on this endpoint; HTTP/1.1 works.
        let request = HttpRequest::get(url, &[("User-Agent", "Mozilla/5.0")]).http1_only();
        let bytes = perform(transport, request).await?;
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| TranslationError::Decoding("Invalid JSON".into()))?;
        let joined = join_segments(&response)?;
        // Quirk kept from Swift: trim first, then check for emptiness.
        let text = joined.trim();
        if text.is_empty() {
            return Err(TranslationError::EmptyResponse);
        }
        Ok(ProviderResult {
            text: text.to_string(),
            usage: None,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::prompts::DEFAULT_MAGIC_PROMPT;

    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/google_free/success.json");
    const URL_NO_QUERY: &str = "https://translate.googleapis.com/translate_a/single";

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::GoogleFree,
            api_key: String::new(),
            base_url: "https://translate.googleapis.com".into(),
            model: String::new(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    async fn run(fake: &FakeTransport, text: &str) -> Result<ProviderResult, TranslationError> {
        GoogleFreeProvider::new()
            .execute(fake, text, AIOperation::Translate, &config())
            .await
    }

    #[tokio::test]
    async fn google_free_translate_request_wire_format() {
        let fake = FakeTransport::json(200, SUCCESS);
        run(&fake, "hello").await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(
            request.url.as_str(),
            "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl=uk&dt=t&q=hello"
        );
        assert_eq!(
            request.headers,
            vec![("User-Agent".to_string(), "Mozilla/5.0".to_string())]
        );
        assert_eq!(request.body, None);
        assert!(request.http1_only);
    }

    #[tokio::test]
    async fn google_free_encodes_special_characters_in_query() {
        let text = "Привіт 😀 a&b=c+d 100% <x>\nline2";
        let fake = FakeTransport::json(200, SUCCESS);
        run(&fake, text).await.unwrap();
        let url = &fake.requests()[0].url;
        let q = url
            .query_pairs()
            .find(|(name, _)| name == "q")
            .map(|(_, value)| value.into_owned());
        assert_eq!(q.as_deref(), Some(text));
        let raw = url.query().unwrap();
        for encoded in ["%2B", "%26", "%25", "%0A"] {
            assert!(raw.contains(encoded), "{encoded} missing in {raw}");
        }
        // The only "+" left is the form encoding of a space.
        assert_eq!(raw.matches('+').count(), text.matches(' ').count(), "{raw}");
    }

    #[tokio::test]
    async fn google_free_success_joins_segments_and_trims() {
        let fake = FakeTransport::json(200, SUCCESS);
        let result = run(&fake, "Good morning, my friend.").await.unwrap();
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
    async fn google_free_skips_segments_without_string_first_element() {
        let fake = FakeTransport::json(200, r#"[[[null,"x"],["ok","y"],"bare",[],{"a":1}]]"#);
        assert_eq!(run(&fake, "t").await.unwrap().text, "ok");
    }

    #[tokio::test]
    async fn google_free_trims_before_the_empty_check() {
        let fake = FakeTransport::json(200, r#"[[["  \n","x"]]]"#);
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
    }

    #[tokio::test]
    async fn google_free_empty_response() {
        let fake = FakeTransport::json(200, "[[]]");
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
    }

    #[tokio::test]
    async fn google_free_invalid_json_is_decoding_invalid_json() {
        for body in ["<html>", ""] {
            let fake = FakeTransport::json(200, body);
            assert_eq!(
                run(&fake, "t").await.unwrap_err(),
                TranslationError::Decoding("Invalid JSON".into()),
                "{body:?}"
            );
        }
    }

    #[tokio::test]
    async fn google_free_unexpected_shape_is_decoding() {
        for body in [r#"{"a":1}"#, "[]", r#"["x"]"#, "[null]", "[{}]"] {
            let fake = FakeTransport::json(200, body);
            assert_eq!(
                run(&fake, "t").await.unwrap_err(),
                TranslationError::Decoding("Unexpected response shape".into()),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn google_free_needs_no_api_key() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.api_key = String::new();
        let result = GoogleFreeProvider::new()
            .execute(&fake, "t", AIOperation::ScreenTranslate, &cfg)
            .await
            .unwrap();
        assert_eq!(result.text, "Доброго ранку, мій друже.");
        assert_eq!(fake.requests().len(), 1);
    }

    #[tokio::test]
    async fn google_free_unknown_language_falls_back_to_lowercased_name() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.target_language = "Klingon".into();
        GoogleFreeProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert!(fake.requests()[0]
            .url
            .query()
            .unwrap()
            .contains("tl=klingon"));
    }

    #[tokio::test]
    async fn google_free_http_error_url_has_no_query() {
        let text = "secret user text";
        let fake = FakeTransport::json(429, "slow down");
        let err = run(&fake, text).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 429,
                body: "slow down".into(),
                url: URL_NO_QUERY.into(),
            }
        );
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[tokio::test]
    async fn google_free_rejects_http_base_url() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = "http://translate.googleapis.com".into();
        let err = GoogleFreeProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".into())
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn google_free_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        GoogleFreeProvider::new()
            .execute(&fake, "hi", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/translate_a/single?client=gtx&sl=auto&tl=uk&dt=t&q=hi"
        );
    }
}
