//! OpenAI and Grok (`/v1/chat/completions`), port of `OpenAIProvider.swift` and
//! `GrokProvider.swift`. Grok is the same wire format at a different host, so one
//! struct serves both and only the provider type differs.

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

#[derive(Debug, Clone, Copy)]
pub struct OpenAiCompatible {
    provider_type: ProviderType,
}

impl OpenAiCompatible {
    pub fn openai() -> Self {
        Self {
            provider_type: ProviderType::OpenAI,
        }
    }

    pub fn grok() -> Self {
        Self {
            provider_type: ProviderType::Grok,
        }
    }

    async fn send(
        &self,
        transport: &dyn Transport,
        config: &ProviderConfig,
        messages: Vec<RequestMessage>,
        sanitize: bool,
    ) -> Result<ProviderResult, TranslationError> {
        let caps = self.provider_type.capabilities();
        let url = endpoint(
            &config.base_url,
            caps.default_base_url,
            "/v1/chat/completions",
            &[],
        )?;
        let body = RequestBody {
            model: &config.model,
            messages,
        };
        let authorization = format!("Bearer {}", config.api_key);
        let request = json_post(url, &[("Authorization", &authorization)], &body)?;
        let bytes = perform(transport, request).await?;
        let decoded: ResponseBody = decode(&bytes)?;
        let choice = decoded
            .choices
            .into_iter()
            .next()
            .ok_or(TranslationError::EmptyResponse)?;
        let raw = choice
            .message
            .content
            .ok_or(TranslationError::EmptyResponse)?;
        Ok(ProviderResult {
            text: finish_text(&raw, sanitize)?,
            usage: decoded.usage.map(|usage| TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            }),
            truncated: choice.finish_reason.as_deref() == Some("length"),
        })
    }
}

#[async_trait]
impl AIProvider for OpenAiCompatible {
    fn provider_type(&self) -> ProviderType {
        self.provider_type
    }

    async fn execute(
        &self,
        transport: &dyn Transport,
        text: &str,
        operation: AIOperation,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        let messages = vec![
            RequestMessage {
                role: "system",
                content: Content::Text(system_prompt(
                    operation,
                    &config.target_language,
                    &config.magic_prompt,
                )),
            },
            RequestMessage {
                role: "user",
                content: Content::Text(user_content(text)),
            },
        ];
        self.send(transport, config, messages, true).await
    }

    async fn execute_vision(
        &self,
        transport: &dyn Transport,
        image: &[u8],
        target_language: &str,
        config: &ProviderConfig,
    ) -> Result<ProviderResult, TranslationError> {
        require_api_key(config)?;
        let (mime, data) = encode_image(image);
        let messages = vec![RequestMessage {
            role: "user",
            content: Content::Parts(vec![
                Part::ImageUrl {
                    image_url: ImageUrl {
                        url: format!("data:{mime};base64,{data}"),
                    },
                },
                Part::Text {
                    text: vision_prompt(target_language),
                },
            ]),
        }];
        self.send(transport, config, messages, false).await
    }
}

#[derive(Serialize)]
struct RequestBody<'a> {
    model: &'a str,
    messages: Vec<RequestMessage>,
}

#[derive(Serialize)]
struct RequestMessage {
    role: &'static str,
    content: Content,
}

/// `Text` serializes as a plain JSON string, `Parts` as an array.
#[derive(Serialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Part {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct ResponseBody {
    choices: Vec<Choice>,
    usage: Option<ResponseUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ResponseUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{FakeTransport, HttpMethod, HttpRequest};
    use crate::operation::AIOperation;
    use crate::prompts::{system_prompt, user_content, vision_prompt, DEFAULT_MAGIC_PROMPT};
    use serde_json::{json, Value};

    const MODEL: &str = "gpt-5.5";
    const KEY: &str = "sk-FIXTURE";
    const URL: &str = "https://api.openai.com/v1/chat/completions";
    const GROK_URL: &str = "https://api.x.ai/v1/chat/completions";
    const SUCCESS: &str = include_str!("../../tests/fixtures/providers/openai/success.json");
    const TRUNCATED: &str = include_str!("../../tests/fixtures/providers/openai/truncated.json");
    const GROK_SUCCESS: &str = include_str!("../../tests/fixtures/providers/grok/success.json");

    fn config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::OpenAI,
            api_key: KEY.into(),
            base_url: "https://api.openai.com".into(),
            model: MODEL.into(),
            target_language: "Ukrainian".into(),
            magic_prompt: DEFAULT_MAGIC_PROMPT.into(),
            referer: None,
            region: None,
        }
    }

    fn grok_config() -> ProviderConfig {
        ProviderConfig {
            provider_type: ProviderType::Grok,
            base_url: "https://api.x.ai".into(),
            model: "grok-4.3".into(),
            ..config()
        }
    }

    fn reply(text: &str) -> String {
        json!({"choices": [{"message": {"content": text}, "finish_reason": "stop"}]}).to_string()
    }

    async fn run(fake: &FakeTransport, text: &str) -> Result<ProviderResult, TranslationError> {
        OpenAiCompatible::openai()
            .execute(fake, text, AIOperation::Translate, &config())
            .await
    }

    async fn run_vision(fake: &FakeTransport) -> Result<ProviderResult, TranslationError> {
        OpenAiCompatible::openai()
            .execute_vision(fake, &[0xFF, 0xD8, 0xFF, 0xE0, 0], "Ukrainian", &config())
            .await
    }

    fn sorted_headers(request: &HttpRequest) -> Vec<(String, String)> {
        let mut headers = request.headers.clone();
        headers.sort();
        headers
    }

    fn expected_headers() -> Vec<(String, String)> {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), format!("Bearer {KEY}")),
        ];
        headers.sort();
        headers
    }

    #[tokio::test]
    async fn openai_translate_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "hello").await.unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(
            request.json_body(),
            json!({
                "model": MODEL,
                "messages": [
                    {"role": "system",
                     "content": system_prompt(AIOperation::Translate, "Ukrainian", DEFAULT_MAGIC_PROMPT)},
                    {"role": "user", "content": user_content("hello")}
                ]
            })
        );
    }

    #[tokio::test]
    async fn openai_vision_request_wire_format() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run_vision(&fake).await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), URL);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(
            request.json_body(),
            json!({
                "model": MODEL,
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "image_url",
                         "image_url": {"url": "data:image/jpeg;base64,/9j/4AA="}},
                        {"type": "text", "text": vision_prompt("Ukrainian")}
                    ]
                }]
            })
        );
    }

    #[tokio::test]
    async fn openai_magic_operation_puts_magic_prompt_in_system_message() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.magic_prompt = "Do X in {{targetLanguage}}".into();
        OpenAiCompatible::openai()
            .execute(&fake, "t", AIOperation::Magic, &cfg)
            .await
            .unwrap();
        let body = fake.requests()[0].json_body();
        let expected = system_prompt(
            AIOperation::Magic,
            "Ukrainian",
            "Do X in {{targetLanguage}}",
        );
        assert_eq!(body["messages"][0]["role"], json!("system"));
        assert_eq!(
            body["messages"][0]["content"],
            Value::String(expected.clone())
        );
        assert!(expected.starts_with("Do X in Ukrainian\n"));
    }

    #[tokio::test]
    async fn openai_text_execute_does_not_check_api_key() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let result = OpenAiCompatible::openai()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(result.text, "ok");
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(fake.requests()[0].header("Authorization"), Some("Bearer "));
    }

    #[tokio::test]
    async fn openai_vision_without_api_key_is_missing_api_key_and_sends_nothing() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.api_key = String::new();
        let err = OpenAiCompatible::openai()
            .execute_vision(&fake, &[1, 2, 3], "Ukrainian", &cfg)
            .await
            .unwrap_err();
        assert_eq!(err, TranslationError::MissingApiKey);
        assert!(fake.requests().is_empty());
    }

    #[tokio::test]
    async fn openai_success_parses_text_and_usage() {
        let fake = FakeTransport::json(200, SUCCESS);
        let result = run(&fake, "Hello").await.unwrap();
        assert_eq!(
            result,
            ProviderResult {
                text: "Привіт".into(),
                usage: Some(TokenUsage {
                    input_tokens: 18,
                    output_tokens: 5
                }),
                truncated: false,
            }
        );
    }

    #[tokio::test]
    async fn openai_truncated_flag() {
        let fake = FakeTransport::json(200, TRUNCATED);
        let result = run(&fake, "Hello").await.unwrap();
        assert!(result.truncated);
        assert_eq!(result.text, "Привіт");
    }

    #[tokio::test]
    async fn openai_only_first_choice_is_used() {
        let body = json!({"choices": [
            {"message": {"content": "first"}},
            {"message": {"content": "second"}}
        ]})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let result = run(&fake, "t").await.unwrap();
        assert_eq!(result.text, "first");
        assert_eq!(result.usage, None);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn openai_null_content_is_empty_response() {
        let body =
            json!({"choices": [{"message": {"content": null, "refusal": "no"}}]}).to_string();
        let fake = FakeTransport::json(200, &body);
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
        let body = json!({"choices": [{"message": {}}]}).to_string();
        let fake = FakeTransport::json(200, &body);
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
    }

    #[tokio::test]
    async fn openai_empty_choices_is_empty_response() {
        let fake = FakeTransport::json(200, &json!({"choices": []}).to_string());
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
    }

    #[tokio::test]
    async fn openai_blank_content_is_empty_response() {
        let fake = FakeTransport::json(200, &reply("  \n"));
        assert_eq!(
            run(&fake, "t").await.unwrap_err(),
            TranslationError::EmptyResponse
        );
        assert_eq!(
            run_vision(&fake).await.unwrap_err(),
            TranslationError::EmptyResponse
        );
    }

    #[tokio::test]
    async fn openai_array_content_is_decoding_error() {
        let body = json!({"choices": [{"message": {
            "content": [{"type": "text", "text": "a"}]
        }}]})
        .to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn openai_missing_required_key_is_decoding_error() {
        for body in [
            json!({"id": "x"}).to_string(),
            json!({"choices": [{"finish_reason": "stop"}]}).to_string(),
            json!({"choices": [{"message": {"content": "a"}}],
                   "usage": {"prompt_tokens": 1}})
            .to_string(),
            json!({"choices": [{"message": {"content": "a"}}],
                   "usage": {"completion_tokens": 1}})
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
    async fn openai_error_object_body_is_decoding() {
        let body = json!({"error": {"message": "x", "type": "invalid_request_error"}}).to_string();
        let fake = FakeTransport::json(200, &body);
        let err = run(&fake, "t").await.unwrap_err();
        assert!(matches!(err, TranslationError::Decoding(_)), "{err:?}");
    }

    #[tokio::test]
    async fn openai_text_op_sanitizes_but_vision_only_trims() {
        let fake = FakeTransport::json(200, &reply("  <text>x &amp; y</text>\n"));
        assert_eq!(run(&fake, "t").await.unwrap().text, "x & y");
        assert_eq!(
            run_vision(&fake).await.unwrap().text,
            "<text>x &amp; y</text>"
        );
    }

    #[tokio::test]
    async fn openai_http_error_passes_status_body_and_redacted_url() {
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
    async fn openai_http_error_debug_does_not_echo_body() {
        let echoed = format!("Incorrect API key provided: {KEY}");
        let fake = FakeTransport::json(401, &echoed);
        let err = run(&fake, "t").await.unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 401,
                body: echoed,
                url: URL.into(),
            }
        );
        let text = format!("{err} {err:?}");
        assert!(!text.contains(KEY), "{text}");
    }

    #[tokio::test]
    async fn openai_rejects_http_base_url() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = "http://api.openai.com".into();
        let err = OpenAiCompatible::openai()
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
    async fn openai_custom_base_with_path_trailing_slash_and_port() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = " https://llm.example.com:8443/litellm/ ".into();
        OpenAiCompatible::openai()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://llm.example.com:8443/litellm/v1/chat/completions"
        );
    }

    #[tokio::test]
    async fn openai_base_ending_in_v1_is_not_deduplicated() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = "https://api.openai.com/v1".into();
        OpenAiCompatible::openai()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(
            fake.requests()[0].url.as_str(),
            "https://api.openai.com/v1/v1/chat/completions"
        );
    }

    #[tokio::test]
    async fn openai_empty_base_url_uses_default() {
        let fake = FakeTransport::json(200, &reply("ok"));
        let mut cfg = config();
        cfg.base_url = String::new();
        OpenAiCompatible::openai()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(fake.requests()[0].url.as_str(), URL);
    }

    #[tokio::test]
    async fn openai_user_text_with_markup_is_escaped_in_body() {
        let fake = FakeTransport::json(200, &reply("ok"));
        run(&fake, "a & <b> 😀\n+%").await.unwrap();
        let request = &fake.requests()[0];
        assert_eq!(
            request.json_body()["messages"][1]["content"],
            json!("<text>\na &amp; &lt;b&gt; 😀\n+%\n</text>")
        );
        let raw = String::from_utf8(request.body.clone().unwrap()).unwrap();
        assert!(raw.contains("😀"), "{raw}");
        assert!(!raw.contains("\\u"), "{raw}");
    }

    #[test]
    fn constructors_set_the_provider_type() {
        assert_eq!(
            OpenAiCompatible::openai().provider_type(),
            ProviderType::OpenAI
        );
        assert_eq!(OpenAiCompatible::grok().provider_type(), ProviderType::Grok);
    }

    #[tokio::test]
    async fn grok_uses_xai_default_base_and_same_wire_format() {
        let fake = FakeTransport::json(200, GROK_SUCCESS);
        let provider = OpenAiCompatible::grok();
        assert_eq!(provider.provider_type(), ProviderType::Grok);
        let result = provider
            .execute(&fake, "hello", AIOperation::Translate, &grok_config())
            .await
            .unwrap();
        assert_eq!(
            result,
            ProviderResult {
                text: "Привіт".into(),
                usage: Some(TokenUsage {
                    input_tokens: 18,
                    output_tokens: 5
                }),
                truncated: false,
            }
        );
        let request = &fake.requests()[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url.as_str(), GROK_URL);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(
            request.json_body(),
            json!({
                "model": "grok-4.3",
                "messages": [
                    {"role": "system",
                     "content": system_prompt(AIOperation::Translate, "Ukrainian", DEFAULT_MAGIC_PROMPT)},
                    {"role": "user", "content": user_content("hello")}
                ]
            })
        );
    }

    #[tokio::test]
    async fn grok_empty_base_url_uses_xai_default() {
        let fake = FakeTransport::json(200, GROK_SUCCESS);
        let mut cfg = grok_config();
        cfg.base_url = String::new();
        OpenAiCompatible::grok()
            .execute(&fake, "t", AIOperation::Translate, &cfg)
            .await
            .unwrap();
        assert_eq!(fake.requests()[0].url.as_str(), GROK_URL);
    }

    #[tokio::test]
    async fn grok_vision_request_wire_format() {
        let fake = FakeTransport::json(200, GROK_SUCCESS);
        OpenAiCompatible::grok()
            .execute_vision(
                &fake,
                &[0x89, 0x50, 0x4E, 0x47, 1],
                "Ukrainian",
                &grok_config(),
            )
            .await
            .unwrap();
        let request = &fake.requests()[0];
        assert_eq!(request.url.as_str(), GROK_URL);
        assert_eq!(sorted_headers(request), expected_headers());
        assert_eq!(
            request.json_body(),
            json!({
                "model": "grok-4.3",
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "image_url",
                         "image_url": {"url": "data:image/png;base64,iVBORwE="}},
                        {"type": "text", "text": vision_prompt("Ukrainian")}
                    ]
                }]
            })
        );
    }
}
