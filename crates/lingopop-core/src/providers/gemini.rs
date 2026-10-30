//! Gemini (`generateContent`), port of `GeminiProvider.swift`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    encode_image, finish_text, require_api_key, AIProvider, ProviderConfig, ProviderResult,
    TokenUsage,
};
use crate::errors::TranslationError;
use crate::http::{decode, endpoint, json_post, perform, Transport};
use crate::operation::AIOperation;
use crate::prompts::{system_prompt, user_content, vision_prompt};
use crate::provider::ProviderType;

#[derive(Debug, Default)]
pub struct GeminiProvider;

/// The model name goes into the URL path, so only `[A-Za-z0-9][A-Za-z0-9._-]*` is
/// accepted (the Swift regex `^[A-Za-z0-9][A-Za-z0-9._-]*$`; unlike `$` in that regex, a
/// trailing newline is rejected).
fn is_valid_model(model: &str) -> bool {
    let mut chars = model.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl GeminiProvider {
    pub fn new() -> Self {
        Self
    }

    async fn send(
        transport: &dyn Transport,
        config: &ProviderConfig,
        system: Option<String>,
        parts: Vec<RequestPart>,
        sanitize: bool,
    ) -> Result<ProviderResult, TranslationError> {
        if !is_valid_model(&config.model) {
            return Err(TranslationError::InvalidConfiguration(
                "Gemini model name is empty or contains unsupported characters".into(),
            ));
        }
        let caps = ProviderType::Gemini.capabilities();
        let path = format!("/v1beta/models/{}:generateContent", config.model);
        let url = endpoint(&config.base_url, caps.default_base_url, &path, &[])?;
        let body = RequestBody {
            system_instruction: system.map(|text| Instruction {
                parts: vec![RequestPart::Text { text }],
            }),
            contents: vec![RequestContent {
                role: "user",
                parts,
            }],
        };
        let request = json_post(url, &[("x-goog-api-key", &config.api_key)], &body)?;
        let bytes = perform(transport, request).await?;
        let decoded: ResponseBody = decode(&bytes)?;
        let raw: String = decoded
            .candidates
            .iter()
            .filter_map(|candidate| candidate.content.as_ref()?.parts.as_ref())
            .flatten()
            .filter_map(|part| part.text.as_deref())
            .collect();
        Ok(ProviderResult {
            text: finish_text(&raw, sanitize)?,
            usage: decoded.usage_metadata.map(|usage| TokenUsage {
                input_tokens: usage.prompt_token_count,
                output_tokens: usage.candidates_token_count.unwrap_or(0),
            }),
            truncated: decoded
                .candidates
                .first()
                .and_then(|candidate| candidate.finish_reason.as_deref())
                == Some("MAX_TOKENS"),
        })
    }
}

#[async_trait]
impl AIProvider for GeminiProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Gemini
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let system = system_prompt(operation, &config.target_language, &config.magic_prompt);
        let parts = vec![RequestPart::Text {
            text: user_content(text),
        }];
        Self::send(transport, config, Some(system), parts, true).await
    }

    async fn execute_vision(
        &self,
        transport: &dyn Transport,
        image: &[u8],
        target_language: &str,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        require_api_key(config)?;
        let (mime_type, data) = encode_image(image);
        let parts = vec![
            RequestPart::InlineData {
                inline_data: InlineData { mime_type, data },
            },
            RequestPart::Text {
                text: vision_prompt(target_language),
            },
        ];
        Self::send(transport, config, None, parts, false).await
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Instruction>,
    contents: Vec<RequestContent>,
}

#[derive(Serialize)]
struct Instruction {
    parts: Vec<RequestPart>,
}

#[derive(Serialize)]
struct RequestContent {
    role: &'static str,
    parts: Vec<RequestPart>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum RequestPart {
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    InlineData {
        inline_data: InlineData,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: &'static str,
    data: String,
}

// `candidates` is required (a blocked prompt has none and is a decoding error).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseBody {
    candidates: Vec<Candidate>,
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<ResponseContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseContent {
    parts: Option<Vec<ResponsePart>>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    prompt_token_count: i64,
    candidates_token_count: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::operation::AIOperation;
    use crate::prompts::{system_prompt, user_content, vision_prompt, DEFAULT_MAGIC_PROMPT};
    use serde_json::json;

    const MODEL: &str = "gemini-3.8-flash";
    const KEY: &str = "AIza-FIXTURE";
    const URL: &str =
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent";
    const MODEL_ERROR: &str = "Gemini model name is empty or contains unsupported characters";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/gemini/success.json");
    const TRUNCATED: &str = include_str!("../../tests/fixtures/providers/gemini/truncated.json");
    const BLOCKED: &str = include_str!("../../tests/fixtures/providers/gemini/blocked_prompt.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::Gemini,
            api_key: KEY.into(),
            base_url: "https://generativelanguage.googleapis.com".into(),
            model: MODEL.into(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn reply(text: &str) -> String {
        json!({"candidates": [{"content": {"parts": [{"text": text}]}, "finishReason": "STOP"}]})
            .to_string()
    }

    async fn run(fake: &FakeTransport, text: &str) -> Result<ProviderResult, TranslationError> {
        GeminiProvider::new()
            .execute(fake, text, AIOperation::Translate, &config())
            .await
    }

    async fn run_vision(fake: &FakeTransport) -> Result<ProviderResult, TranslationError> {
        GeminiProvider::new()
            .execute_vision(fake, &[0xFF, 0xD8, 0xFF, 0xE0, 0], "Ukrainian", &config())
            .await
    }

    fn sorted_headers(request: &crate::http::HttpRequest) -> Vec<(String, String)> {
        let mut headers = request.headers.clone();
        headers.sort();
        headers
    }

    fn expected_headers() -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-goog-api-key".to_string(), KEY.to_string()),
        ];
        headers.sort();
        headers
    }

    #[tokio::test]
    async fn gemini_translate_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "hello").await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(request.url.query(), None);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(request.header("Authorization"), None);
        assert_eq!(
            request.json_body(),
            json!({
                "systemInstruction": {"parts": [{
                    "text": system_prompt(AIOperation::Translate, "Ukrainian", DEFAULT_MAGIC_PROMPT)
                }]},
                "contents": [{
                    "role": "user",
                    "parts": [{"text": user_content("hello")}]
                }]
            })
        );
    }

    #[tokio::test]
    async fn gemini_vision_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run_vision(&fake).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(sorted_headers(request), expected_headers());
        let body = request.json_body();
        assert!(body.get("systemInstruction").is_none());
        assert_eq!(
            body,
            json!({
                "contents": [{
                    "role": "user",
                    "parts": [
                        {"inlineData": {"mimeType": "image/jpeg", "data": "/9j/4AA="}},
                        {"text": vision_prompt("Ukrainian")}
                    ]
                }]
            })
        );
    }

    #[tokio::test]
    async fn gemini_text_execute_does_not_check_api_key() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let result = GeminiProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(result.text, "ok");
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(fake.requests()[0].header("x-goog-api-key"), Some(""));
    }

    #[tokio::test]
    async fn gemini_vision_without_api_key_is_missing_api_key_and_sends_nothing() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let err = GeminiProvider::new()
            .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &cfg)
            .await
            .unwrap_err();
        assert_eq!(err, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn gemini_vision_checks_api_key_before_model() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        cfg.model = "bad model".into();
        let err = GeminiProvider::new()
            .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &cfg)
            .await
            .unwrap_err();
        assert_eq!(err, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn gemini_model_validation_table() {
        for valid in ["gemini-3.8-flash", "gemini-2.5-pro", "a", "A1._-"] {
            assert!(is_valid_model(valid), "{valid:?}");
        }
        for invalid in [
            "",
            "-x",
            ".x",
            "_x",
            "gem ini",
            "gemini/x",
            "a:b",
            "модель",
            "x\n",
        ] {
            assert!(!is_valid_model(invalid), "{invalid:?}");
        }
    }

    #[tokio::test]
    async fn gemini_invalid_model_is_invalid_configuration_and_sends_nothing() {
        for model in ["", "-x", "gemini/x", "a:b", "модель", "x\n"] {
            let fake = FakeTransport::json(200, &reply("ok"));
            let mut cfg = config();
            cfg.model = model.into();
            let text_err = GeminiProvider::new()
                .execute(&fake, "t", AIOperation::Translate, &cfg)
                .await
                .unwrap_err();
            let vision_err = GeminiProvider::new()
                .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &cfg)
                .await
                .unwrap_err();
            let expected = TranslationError::InvalidConfiguration(MODEL_ERROR.into());
            assert_eq!(text_err, expected, "{model:?}");
            assert_eq!(vision_err, expected, "{model:?}");
            assert!(fake.requests().is_empty(), "{model:?}");
        }
    }

    #[tokio::test]
    async fn gemini_success_parses_text_and_usage() {
        let fake = FakeTransport::json(200, SUCCESS);
        let result = run(&fake, "Hello").await.unwrap();
        assert_eq!(
            result,
            ProviderResult {
                text: "Привіт".into(),
                usage: Some(TokenUsage {
                    input_tokens: 12,
                    output_tokens: 4
                }),
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn gemini_candidates_token_count_defaults_to_zero() {
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "a"}]}}],
            "usageMetadata": {"promptTokenCount": 5}
        })
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let result = run(&fake, "t").await.unwrap();
        assert_eq!(
            result.usage,
            Some(TokenUsage {
                input_tokens: 5,
                output_tokens: 0
            })
        );
    }

    #[tokio::test]
    async fn gemini_truncated_flag_uses_first_candidate() {
        let fake = FakeTransport::json(200, TRUNCATED);
        let result = run(&fake, "Hello").await.unwrap();
        assert!(result.truncated);
        assert_eq!(result.text, "Привіт");

        let body = json!({"candidates": [
            {"content": {"parts": [{"text": "a"}]}, "finishReason": "STOP"},
            {"content": {"parts": [{"text": "b"}]}, "finishReason": "MAX_TOKENS"}
        ]})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let result = run(&fake, "t").await.unwrap();
        assert!(!result.truncated);
        assert_eq!(result.text, "ab");
    }

    #[tokio::test]
    async fn gemini_concatenates_all_candidates_and_parts_skipping_parts_without_text() {
        let body = json!({"candidates": [
            {"content": {"parts": [
                {"text": "a"},
                {"inlineData": {"mimeType": "image/png", "data": "AA=="}},
                {"text": "b"}
            ]}},
            {"content": {"parts": [{"text": "c"}, {"text": "d"}]}}
        ]})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let result = run(&fake, "x").await.unwrap();
        assert_eq!(result.text, "abcd");
        assert_eq!(result.usage, None);
    }

    #[tokio::test]
    async fn gemini_blocked_prompt_without_candidates_is_decoding() {
        let fake = FakeTransport::json(200, BLOCKED);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn gemini_candidate_without_content_is_empty_response() {
        for body in [
            json!({"candidates": [{"finishReason": "SAFETY"}]}).to_string(),
            json!({"candidates": []}).to_string(),
            json!({"candidates": [{"content": {}}]}).to_string(),
            reply("  \n"),
        ] {
            let fake = FakeTransport::json(200, &body);
            assert_eq!(
                run(&fake, "t").await.unwrap_err(),
                TranslationError::EmptyResponse,
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn gemini_missing_prompt_token_count_is_decoding_error() {
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "a"}]}}],
            "usageMetadata": {}
        })
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn gemini_error_object_body_is_decoding() {
        let body = json!({"error": {
            "code": 400, "message": "x", "status": "INVALID_ARGUMENT"
        }})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn gemini_text_op_sanitizes_but_vision_only_trims() {
        let fake = FakeTransport::json(200, &reply("  <text>x &amp; y</text>\n"));
        assert_eq!(run(&fake, "t").await.unwrap().text, "x & y");
        assert_eq!(
            run_vision(&fake).await.unwrap().text,
            "<text>x &amp; y</text>"
        );
    }

    #[tokio::test]
    async fn gemini_http_error_passes_status_body_and_redacted_url() {
        let fake = FakeTransport::json(400, "bad request");
        let err = run(&fake, "t").await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 400,
                body: "bad request".into(),
                url: URL.into(),
            }
        );
        assert!(!format!("{err} {err:?}").contains(KEY));
    }

    #[tokio::test]
    async fn gemini_http_error_debug_does_not_echo_body() {
        let echoed = format!("API key not valid: {KEY}");
        let fake = FakeTransport::json(400, &echoed);
        let err = run(&fake, "t").await.unwrap_err();
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
    }

    #[tokio::test]
    async fn gemini_rejects_http_base_url() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = "http://generativelanguage.googleapis.com".into();
        let err = GeminiProvider::new()
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
    async fn gemini_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        GeminiProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/v1beta/models/gemini-3.8-flash:generateContent"
        );
    }

    #[tokio::test]
    async fn gemini_user_text_with_markup_is_escaped_in_body() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "a & <b> 😀\n+%").await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(
            request.json_body()["contents"][0]["parts"][0]["text"],
            json!("<text>\na &amp; &lt;b&gt; 😀\n+%\n</text>")
        );
        let raw = String::from_utf8(request.body.clone().unwrap()).unwrap();
        assert!(raw.contains("😀"), "{raw}");
        assert!(!raw.contains("\\u"), "{raw}");
    }
}
