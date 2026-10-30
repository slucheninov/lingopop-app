//! DeepL (`/v2/translate`), port of `DeepLProvider.swift`. The key travels in the
//! `Authorization: DeepL-Auth-Key` header, never in the URL. An empty base URL picks the
//! free or the pro endpoint by the key suffix `:fx`. Screen Translate runs the OCR text
//! through the same call; unsupported operations are rejected by the runner.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{AIProvider, ProviderConfig, ProviderResult};
use crate::errors::TranslationError;
use crate::http::{decode, endpoint, json_post, perform, Transport};
use crate::languages;
use crate::operation::AIOperation;
use crate::provider::ProviderType;

const FREE_BASE_URL: &str = "https://api-free.deepl.com";

#[derive(Debug, Default)]
pub struct DeepLProvider;

impl DeepLProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AIProvider for DeepLProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::DeepL
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        _operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = ProviderType::DeepL.capabilities();
        // Quirk kept from Swift: the emptiness test is on the raw base URL, so a
        // whitespace-only base skips the key-suffix selection and `endpoint` then trims it
        // to the pro default.
        let effective = if config.base_url.is_empty() {
            if config.api_key.ends_with(":fx") {
                FREE_BASE_URL
            } else {
                caps.default_base_url
            }
        } else {
            config.base_url.as_str()
        };
        let url = endpoint(effective, caps.default_base_url, "/v2/translate", &[])?;
        let body = RequestBody {
            text: [text],
            target_lang: languages::deepl_code(&config.target_language),
        };
        let authorization = format!("DeepL-Auth-Key {}", config.api_key);
        let request = json_post(url, &[("Authorization", authorization.as_str())], &body)?;
        let bytes = perform(transport, request).await?;
        let decoded: ResponseBody = decode(&bytes)?;
        // Quirk kept from Swift: emptiness is checked before trimming.
        let translated = decoded
            .translations
            .into_iter()
            .next()
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
struct RequestBody<'a> {
    text: [&'a str; 1],
    target_lang: String,
}

#[derive(Deserialize)]
struct ResponseBody {
    translations: Vec<Translation>,
}

#[derive(Deserialize)]
struct Translation {
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::prompts::DEFAULT_MAGIC_PROMPT;
    use serde_json::json;

    const KEY: &str = "deepl-FIXTURE";
    const FREE_KEY: &str = "deepl-FIXTURE:fx";
    const URL: &str = "https://api.deepl.com/v2/translate";
    const FREE_URL: &str = "https://api-free.deepl.com/v2/translate";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/deepl/success.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::DeepL,
            api_key: KEY.into(),
            base_url: String::new(),
            model: String::new(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn reply(text: &str) -> String {
        json!({"translations": [{"text": text}]}).to_string()
    }

    async fn run_with(
        fake: &FakeTransport,
        cfg: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        DeepLProvider::new()
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
    async fn deepl_translate_request_wire_format() {
        let fake = FakeTransport::json(200, SUCCESS);
        run(&fake).await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        let mut expected = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), format!("DeepL-Auth-Key {KEY}")),
        ];
        expected.sort();
        assert_eq!(sorted_headers(request), expected);
        assert_eq!(
            request.json_body(),
            json!({"text": ["hello"], "target_lang": "UK"})
        );
    }

    #[tokio::test]
    async fn deepl_uses_deepl_language_codes() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.target_language = "Norwegian".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(fake.requests()[0].json_body()["target_lang"], json!("NB"));
    }

    #[tokio::test]
    async fn deepl_free_key_suffix_selects_free_host() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.api_key = FREE_KEY.into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(fake.requests()[0].url.as_str(), FREE_URL);
    }

    #[tokio::test]
    async fn deepl_explicit_base_wins_over_key_suffix() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.api_key = FREE_KEY.into();
        cfg.base_url = "https://proxy.example.com".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://proxy.example.com/v2/translate"
        );
    }

    #[tokio::test]
    async fn deepl_whitespace_only_base_falls_to_pro_default() {
        // Swift quirk: the emptiness test is on the raw base, so "  " is not empty and the
        // key suffix is ignored; `endpoint` then trims it to the pro default.
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.api_key = FREE_KEY.into();
        cfg.base_url = "  ".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(fake.requests()[0].url.as_str(), URL);
    }

    #[tokio::test]
    async fn deepl_success_parses_text() {
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
    async fn deepl_ignores_the_operation() {
        for operation in [AIOperation::Translate, AIOperation::ScreenTranslate] {
            let fake = FakeTransport::json(200, SUCCESS);
            let result = DeepLProvider::new()
                .execute(&fake, "hello", operation, &config())
                .await
                .unwrap();
            assert_eq!(result.text, "Доброго ранку, мій друже.");
            assert_eq!(
                fake.requests()[0].json_body(),
                json!({"text": ["hello"], "target_lang": "UK"})
            );
        }
    }

    #[tokio::test]
    async fn deepl_trims_the_text() {
        let fake = FakeTransport::json(200, &reply("  hi \n"));
        assert_eq!(run(&fake).await.unwrap().text, "hi");
    }

    #[tokio::test]
    async fn deepl_empty_response() {
        let empty_list = json!({"translations": []}).to_string();
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
    async fn deepl_missing_required_key_is_decoding_error() {
        for body in ["{}", r#"{"translations":[{}]}"#] {
            let fake = FakeTransport::json(200, body);
            let err = run(&fake).await.unwrap_err();
            assert!(
                matches!(err, TranslationError::Decoding(_)),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn deepl_error_object_body_is_decoding() {
        let body = json!({"message": "Quota exceeded"}).to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake).await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn deepl_http_error_passes_status_body_and_redacted_url() {
        let fake = FakeTransport::json(456, "quota exceeded");
        let err = run(&fake).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 456,
                body: "quota exceeded".into(),
                url: URL.into(),
            }
        );
        let text = format!("{err} {err:?}");
        assert!(!text.contains(KEY), "{text}");
    }

    #[tokio::test]
    async fn deepl_rejects_http_base_url() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = "http://api.deepl.com".into();
        let err = run_with(&fake, &cfg).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".into())
        );
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn deepl_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, SUCCESS);
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        run_with(&fake, &cfg).await.unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/v2/translate"
        );
    }

    #[tokio::test]
    async fn deepl_http_error_echoing_the_key_does_not_leak_it() {
        // An upstream that echoes the key back in the error body.
        let echoed = format!("Authorization failure: DeepL-Auth-Key {KEY}");
        let fake = FakeTransport::json(403, &echoed);
        let err = run(&fake).await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 403,
                body: echoed,
                url: URL.into(),
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
    async fn deepl_malformed_200_echoing_the_key_does_not_leak_it() {
        for body in [
            json!({"translations": KEY}).to_string(),
            json!({"translations": [KEY]}).to_string(),
            json!({"translations": [{"text": [KEY]}]}).to_string(),
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
