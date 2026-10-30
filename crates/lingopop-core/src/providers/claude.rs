//! Claude (`/v1/messages`), port of `ClaudeProvider.swift`.

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

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

#[derive(Debug, Default)]
pub struct ClaudeProvider;

impl ClaudeProvider {
    pub fn new() -> Self {
        Self
    }

    async fn send(
        transport: &dyn Transport,
        config: &ProviderConfig,
        system: Option<String>,
        content: Vec<RequestBlock>,
        sanitize: bool,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = ProviderType::Claude.capabilities();
        let url = endpoint(&config.base_url, caps.default_base_url, "/v1/messages", &[])?;
        let body = RequestBody {
            model: &config.model,
            max_tokens: MAX_TOKENS,
            system,
            messages: vec![RequestMessage {
                role: "user",
                content,
            }],
        };
        let request = json_post(
            url,
            &[
                ("x-api-key", &config.api_key),
                ("anthropic-version", ANTHROPIC_VERSION),
            ],
            &body,
        )?;
        let bytes = perform(transport, request).await?;
        let decoded: ResponseBody = decode(&bytes)?;
        let raw: String = decoded
            .content
            .iter()
            .filter_map(|block| block.text.as_deref())
            .collect();
        Ok(ProviderResult {
            text: finish_text(&raw, sanitize)?,
            usage: decoded.usage.map(|usage| TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            }),
            truncated: decoded.stop_reason.as_deref() == Some("max_tokens"),
        })
    }
}

#[async_trait]
impl AIProvider for ClaudeProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Claude
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let system = system_prompt(operation, &config.target_language, &config.magic_prompt);
        let content = vec![RequestBlock::Text {
            text: user_content(text),
        }];
        Self::send(transport, config, Some(system), content, true).await
    }

    async fn execute_vision(
        &self,
        transport: &dyn Transport,
        image: &[u8],
        target_language: &str,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        require_api_key(config)?;
        let (media_type, data) = encode_image(image);
        let content = vec![
            RequestBlock::Image {
                source: ImageSource {
                    kind: "base64",
                    media_type,
                    data,
                },
            },
            RequestBlock::Text {
                text: vision_prompt(target_language),
            },
        ];
        Self::send(transport, config, None, content, false).await
    }
}

#[derive(Serialize)]
struct RequestBody<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<RequestMessage>,
}

#[derive(Serialize)]
struct RequestMessage {
    role: &'static str,
    content: Vec<RequestBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RequestBlock {
    Text { text: String },
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: String,
}

#[derive(Deserialize)]
struct ResponseBody {
    content: Vec<ResponseBlock>,
    stop_reason: Option<String>,
    usage: Option<ResponseUsage>,
}

#[derive(Deserialize)]
struct ResponseBlock {
    // Required like the Swift struct, though only `text` is read.
    #[allow(dead_code)]
    r#type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct ResponseUsage {
    input_tokens: i64,
    output_tokens: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod};
    use crate::operation::AIOperation;
    use crate::prompts::{system_prompt, user_content, vision_prompt, DEFAULT_MAGIC_PROMPT};
    use serde_json::{json, Value};

    const MODEL: &str = "claude-sonnet-5-5";
    const KEY: &str = "sk-ant-FIXTURE";
    const URL: &str = "https://api.anthropic.com/v1/messages";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/claude/success.json");
    const TRUNCATED: &str = include_str!("../../tests/fixtures/providers/claude/truncated.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::Claude,
            api_key: KEY.into(),
            base_url: "https://api.anthropic.com".into(),
            model: MODEL.into(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn reply(text: &str) -> String {
        json!({"content": [{"type": "text", "text": text}], "stop_reason": "end_turn"}).to_string()
    }

    async fn run(fake: &FakeTransport, text: &str) -> Result<ProviderResult, TranslationError> {
        ClaudeProvider::new()
            .execute(fake, text, AIOperation::Translate, &config())
            .await
    }

    async fn run_vision(fake: &FakeTransport) -> Result<ProviderResult, TranslationError> {
        ClaudeProvider::new()
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
            ("x-api-key".to_string(), KEY.to_string()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ];
        headers.sort();
        headers
    }

    #[tokio::test]
    async fn claude_translate_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "hello").await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(request.header("Authorization"), None);
        assert_eq!(request.header("anthropic-beta"), None);
        assert_eq!(
            request.json_body(),
            json!({
                "model": MODEL,
                "max_tokens": 4096,
                "system": system_prompt(AIOperation::Translate, "Ukrainian", DEFAULT_MAGIC_PROMPT),
                "messages": [{
                    "role": "user",
                    "content": [{"type": "text", "text": user_content("hello")}]
                }]
            })
        );
    }

    #[tokio::test]
    async fn claude_vision_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run_vision(&fake).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(sorted_headers(request), expected_headers());
        let body = request.json_body();
        assert!(body.get("system").is_none());
        assert_eq!(
            body,
            json!({
                "model": MODEL,
                "max_tokens": 4096,
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "image", "source": {
                            "type": "base64",
                            "media_type": "image/jpeg",
                            "data": "/9j/4AA="
                        }},
                        {"type": "text", "text": vision_prompt("Ukrainian")}
                    ]
                }]
            })
        );
    }

    #[tokio::test]
    async fn claude_magic_operation_puts_magic_prompt_in_system() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.magic_prompt = "Do X in {{targetLanguage}}".into();
        ClaudeProvider::new()
            .execute(&fake, "t", AIOperation::Magic, &cfg)
            .await
            .unwrap();
        let system = fake.requests()[0].json_body()["system"].clone();
        let expected = system_prompt(
            AIOperation::Magic,
            "Ukrainian",
            "Do X in {{targetLanguage}}",
        );
        assert_eq!(system, Value::String(expected.clone()));
        assert!(expected.starts_with("Do X in Ukrainian\n"));
    }

    #[tokio::test]
    async fn claude_text_execute_does_not_check_api_key() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let result = ClaudeProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(result.text, "ok");
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(fake.requests()[0].header("x-api-key"), Some(""));
    }

    #[tokio::test]
    async fn claude_vision_without_api_key_is_missing_api_key_and_sends_nothing() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let err = ClaudeProvider::new()
            .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &cfg)
            .await
            .unwrap_err();
        assert_eq!(err, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn claude_success_parses_text_and_usage() {
        let fake = FakeTransport::json(200, SUCCESS);
        let result = run(&fake, "Hello, world").await.unwrap();
        assert_eq!(
            result,
            ProviderResult {
                text: "Привіт, світе".into(),
                usage: Some(TokenUsage {
                    input_tokens: 21,
                    output_tokens: 7
                }),
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn claude_truncated_flag() {
        let fake = FakeTransport::json(200, TRUNCATED);
        let result = run(&fake, "Hello").await.unwrap();
        assert!(result.truncated);
        assert_eq!(result.text, "Привіт, світе");
    }

    #[tokio::test]
    async fn claude_concatenates_blocks_with_text_and_skips_blocks_without() {
        let body = json!({"content": [
            {"type": "text", "text": "a"},
            {"type": "tool_use"},
            {"type": "text", "text": "b"}
        ]})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let result = run(&fake, "x").await.unwrap();
        assert_eq!(result.text, "ab");
        assert_eq!(result.usage, None);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn claude_text_op_sanitizes_output_but_vision_only_trims() {
        let fake = FakeTransport::json(200, &reply("  <text>x &amp; y</text>\n"));
        assert_eq!(run(&fake, "t").await.unwrap().text, "x & y");
        assert_eq!(
            run_vision(&fake).await.unwrap().text,
            "<text>x &amp; y</text>"
        );
    }

    #[tokio::test]
    async fn claude_empty_response() {
        for body in [
            json!({"content": []}).to_string(),
            reply("  "),
            json!({"content": [{"type": "tool_use"}]}).to_string(),
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
    async fn claude_missing_required_key_is_decoding_error() {
        for body in [
            json!({"stop_reason": "end_turn"}).to_string(),
            json!({"content": [{"text": "a"}]}).to_string(),
            json!({"content": [{"type": "text", "text": "a"}],
                   "usage": {"input_tokens": 1}})
            .to_string(),
        ] {
            let fake = FakeTransport::json(200, &body);
            let err = run(&fake, "t").await.unwrap_err();
            assert!(
                matches!(err, TranslationError::Decoding(_)),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn claude_error_object_body_is_decoding() {
        let body = json!({"type": "error",
            "error": {"type": "overloaded_error", "message": "x"}})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn claude_http_error_passes_status_body_and_redacted_url() {
        let fake = FakeTransport::json(429, "slow down");
        let err = run(&fake, "t").await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 429,
                body: "slow down".into(),
                url: URL.into(),
            }
        );
        assert!(!format!("{err} {err:?}").contains(KEY));
    }

    #[tokio::test]
    async fn claude_rejects_http_base_url() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = "http://api.anthropic.com".into();
        let err = ClaudeProvider::new()
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
    async fn claude_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        ClaudeProvider::new()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/v1/messages"
        );
    }

    #[tokio::test]
    async fn claude_user_text_with_markup_is_escaped_in_body() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "a & <b> 😀\n+%").await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(
            request.json_body()["messages"][0]["content"][0]["text"],
            json!("<text>\na &amp; &lt;b&gt; 😀\n+%\n</text>")
        );
        let raw = String::from_utf8(request.body.clone().unwrap()).unwrap();
        assert!(raw.contains("😀"), "{raw}");
        assert!(!raw.contains("\\u"), "{raw}");
    }
}
