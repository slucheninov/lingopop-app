//! User-facing errors (port of `Models/TranslationError.swift` and `UserFacingError.swift`).
//!
//! Texts are `UserMessage`s: the English catalog key plus its interpolated arguments, so a
//! later localization layer can translate the key and fill the placeholders. Nothing here
//! ever includes an API key: URLs are redacted and response bodies are never echoed.

use std::fmt;

use url::Url;

use crate::operation::AIOperation;
use crate::provider::ProviderType;

const KEY_HTTP_FROM: &str = "HTTP %@ from %@";
const KEY_BODY_OMITTED: &str = "Response body omitted for privacy (%@ characters).";
/// Key of a message that is not translated: `english()` renders `args[0]` verbatim. Used for
/// technical detail text (decoding errors, network error descriptions) that Swift also
/// passes through unlocalized. It is deliberately not a catalog key.
pub const VERBATIM_KEY: &str = "%@";

/// A user-visible text: the English catalog key (with `%@` / `%lld` placeholders) and the
/// arguments for those placeholders, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessage {
    pub key: &'static str,
    pub args: Vec<String>,
}

impl UserMessage {
    fn plain(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    fn with_args(key: &'static str, args: Vec<String>) -> Self {
        Self { key, args }
    }

    /// The English text: `%@` and `%lld` placeholders are replaced by the arguments in
    /// order. Substituted values are never re-scanned; a placeholder without an argument
    /// is left as is.
    pub fn english(&self) -> String {
        let mut out = String::with_capacity(self.key.len());
        let mut args = self.args.iter();
        let mut rest = self.key;
        while let Some(pos) = rest.find('%') {
            out.push_str(&rest[..pos]);
            let tail = &rest[pos..];
            let placeholder_len = if tail.starts_with("%@") {
                2
            } else if tail.starts_with("%lld") {
                4
            } else {
                out.push('%');
                rest = &tail[1..];
                continue;
            };
            match args.next() {
                Some(arg) => out.push_str(arg),
                None => out.push_str(&tail[..placeholder_len]),
            }
            rest = &tail[placeholder_len..];
        }
        out.push_str(rest);
        out
    }
}

/// The English rendering of detail lines, joined by a line break like the Swift detail.
pub fn english_lines(lines: &[UserMessage]) -> String {
    lines
        .iter()
        .map(UserMessage::english)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strips credentials, query and fragment from a URL before it goes into user text, so
/// key-bearing query parameters (`?key=...`) never leak. Unparsable or relative input
/// gives `<redacted-url>`. (The `url` crate normalizes: a bare host gets a trailing `/`
/// and the host is lowercased.)
pub fn redacted_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return "<redacted-url>".to_string();
    };
    // Setters fail only for cannot-be-a-base URLs, which carry no credentials anyway.
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

/// A privacy note that stands in for an HTTP response body (the body may echo secrets);
/// `None` for a blank body.
pub fn sanitized_http_body_detail(body: &str) -> Option<UserMessage> {
    if body.trim().is_empty() {
        return None;
    }
    Some(UserMessage::with_args(
        KEY_BODY_OMITTED,
        vec![body.chars().count().to_string()],
    ))
}

/// Errors from a provider call, convertible to user text. `Debug` never prints the HTTP
/// response body (a custom endpoint may echo the submitted API key back); it shows the
/// body's character count only.
#[derive(Clone, PartialEq)]
pub enum TranslationError {
    MissingApiKey,
    InvalidUrl,
    Http {
        status: u16,
        body: String,
        url: String,
    },
    Decoding(String),
    EmptyResponse,
    Network(String),
    OperationNotSupported {
        provider: ProviderType,
        operation: AIOperation,
    },
    InvalidConfiguration(String),
    ProviderNotFound(ProviderType),
}

impl fmt::Debug for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => f.write_str("MissingApiKey"),
            Self::InvalidUrl => f.write_str("InvalidUrl"),
            Self::Http { status, body, url } => f
                .debug_struct("Http")
                .field("status", status)
                .field("body", &format_args!("<{} chars>", body.chars().count()))
                .field("url", url)
                .finish(),
            Self::Decoding(text) => f.debug_tuple("Decoding").field(text).finish(),
            Self::EmptyResponse => f.write_str("EmptyResponse"),
            Self::Network(text) => f.debug_tuple("Network").field(text).finish(),
            Self::OperationNotSupported {
                provider,
                operation,
            } => f
                .debug_struct("OperationNotSupported")
                .field("provider", provider)
                .field("operation", operation)
                .finish(),
            Self::InvalidConfiguration(text) => {
                f.debug_tuple("InvalidConfiguration").field(text).finish()
            }
            Self::ProviderNotFound(provider) => {
                f.debug_tuple("ProviderNotFound").field(provider).finish()
            }
        }
    }
}

impl TranslationError {
    /// The short, actionable sentence.
    pub fn user_summary(&self) -> UserMessage {
        match self {
            Self::MissingApiKey => UserMessage::plain(
                "API key is not set. Open Settings → Providers and add one.",
            ),
            Self::InvalidUrl => UserMessage::plain("Invalid base URL."),
            Self::Http { status, .. } => match status_hint(*status) {
                Some(hint) => UserMessage::plain(hint),
                None => UserMessage::with_args(
                    "Request failed (HTTP %lld).",
                    vec![status.to_string()],
                ),
            },
            Self::Decoding(_) => UserMessage::plain("Failed to parse the provider response."),
            Self::EmptyResponse => UserMessage::plain("Provider returned empty translation."),
            Self::Network(_) => {
                UserMessage::plain("Network error. Check your connection and try again.")
            }
            Self::OperationNotSupported { provider, operation } => UserMessage::with_args(
                "%@ doesn't support %@. Switch to an LLM provider (Claude / OpenAI / Gemini / Grok) in Settings → Providers.",
                vec![
                    provider.display_name().to_string(),
                    operation.english_name().to_lowercase(),
                ],
            ),
            Self::InvalidConfiguration(message) => {
                UserMessage::with_args("Invalid configuration: %@", vec![message.clone()])
            }
            Self::ProviderNotFound(provider) => UserMessage::with_args(
                "Provider %@ is not registered. Please report this as a bug.",
                vec![provider.display_name().to_string()],
            ),
        }
    }

    /// Optional technical context a UI may hide behind a disclosure, as separate lines (join
    /// them with `english_lines`). Every line is its own catalog key, except `VERBATIM_KEY`
    /// lines.
    pub fn user_detail(&self) -> Option<Vec<UserMessage>> {
        match self {
            Self::Http { status, body, url } => {
                let mut lines = vec![UserMessage::with_args(
                    KEY_HTTP_FROM,
                    vec![status.to_string(), redacted_url(url)],
                )];
                lines.extend(sanitized_http_body_detail(body));
                Some(lines)
            }
            Self::Decoding(detail) | Self::Network(detail) => Some(vec![UserMessage::with_args(
                VERBATIM_KEY,
                vec![detail.clone()],
            )]),
            _ => None,
        }
    }
}

fn status_hint(status: u16) -> Option<&'static str> {
    match status {
        401 | 403 => Some("Check the API key in Settings → Providers."),
        404 => Some("Endpoint not found. Verify Base URL and Model in Settings."),
        429 => Some("Rate limit exceeded. Wait a moment and retry."),
        500..=599 => Some(
            "Server error from your endpoint. If you use a proxy (LiteLLM), check the model name is configured there.",
        ),
        _ => None,
    }
}

/// Logs and `to_string()` keep the combined summary + technical detail (`errorDescription`).
impl fmt::Display for TranslationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_summary().english())?;
        if let Some(detail) = self.user_detail() {
            write!(f, "\n\n{}", english_lines(&detail))?;
        }
        Ok(())
    }
}

impl std::error::Error for TranslationError {}

/// Why the primary provider failed, shown when a fallback provider produced the result.
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackInfo {
    pub primary_provider: ProviderType,
    pub message: UserMessage,
    pub details: Option<Vec<UserMessage>>,
}

impl FallbackInfo {
    pub fn new(primary_provider: ProviderType, error: &TranslationError) -> Self {
        Self {
            primary_provider,
            message: error.user_summary(),
            details: error.user_detail(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::AIOperation;
    use crate::provider::ProviderType;

    #[test]
    fn translation_error_debug_omits_http_body() {
        let err = TranslationError::Http {
            status: 401,
            body: "key sk-ant-FIXTURE echoed".to_string(),
            url: "https://api.x.com/v1".to_string(),
        };
        let debug = format!("{err:?}");
        assert!(!debug.contains("sk-ant-FIXTURE"), "{debug}");
        assert!(debug.contains("401"), "{debug}");
        assert!(debug.contains("https://api.x.com/v1"), "{debug}");
        assert!(debug.contains("<25 chars>"), "{debug}");
        assert!(!format!("{err}").contains("sk-ant-FIXTURE"));
        assert_eq!(
            format!("{:?}", TranslationError::Decoding("x".to_string())),
            "Decoding(\"x\")"
        );
    }

    fn http(status: u16) -> TranslationError {
        TranslationError::Http {
            status,
            body: "secret body".to_string(),
            url: "https://x".to_string(),
        }
    }

    fn msg(key: &'static str, args: &[&str]) -> UserMessage {
        UserMessage {
            key,
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn summary_keys() {
        assert_eq!(
            TranslationError::MissingApiKey.user_summary(),
            msg(
                "API key is not set. Open Settings → Providers and add one.",
                &[]
            )
        );
        assert_eq!(
            TranslationError::InvalidUrl.user_summary(),
            msg("Invalid base URL.", &[])
        );
        assert_eq!(
            TranslationError::Decoding("d".into()).user_summary(),
            msg("Failed to parse the provider response.", &[])
        );
        assert_eq!(
            TranslationError::EmptyResponse.user_summary(),
            msg("Provider returned empty translation.", &[])
        );
        assert_eq!(
            TranslationError::Network("m".into()).user_summary(),
            msg("Network error. Check your connection and try again.", &[])
        );
        assert_eq!(
            TranslationError::InvalidConfiguration("bad".into()).user_summary(),
            msg("Invalid configuration: %@", &["bad"])
        );
        assert_eq!(
            TranslationError::ProviderNotFound(ProviderType::GoogleFree).user_summary(),
            msg(
                "Provider %@ is not registered. Please report this as a bug.",
                &["Google (Free)"]
            )
        );
        assert_eq!(
            TranslationError::OperationNotSupported {
                provider: ProviderType::DeepL,
                operation: AIOperation::GrammarFix,
            }
            .user_summary(),
            msg(
                "%@ doesn't support %@. Switch to an LLM provider (Claude / OpenAI / Gemini / Grok) in Settings → Providers.",
                &["DeepL", "fix grammar"]
            )
        );
    }

    #[test]
    fn http_summary_hints() {
        let key_hint = "Check the API key in Settings → Providers.";
        assert_eq!(http(401).user_summary(), msg(key_hint, &[]));
        assert_eq!(http(403).user_summary(), msg(key_hint, &[]));
        assert_eq!(
            http(404).user_summary(),
            msg(
                "Endpoint not found. Verify Base URL and Model in Settings.",
                &[]
            )
        );
        assert_eq!(
            http(429).user_summary(),
            msg("Rate limit exceeded. Wait a moment and retry.", &[])
        );
        let server = "Server error from your endpoint. If you use a proxy (LiteLLM), check the model name is configured there.";
        assert_eq!(http(500).user_summary(), msg(server, &[]));
        assert_eq!(http(599).user_summary(), msg(server, &[]));
        assert_eq!(
            http(418).user_summary(),
            msg("Request failed (HTTP %lld).", &["418"])
        );
        // Boundaries around the 5xx range are not hints.
        assert_eq!(
            http(499).user_summary(),
            msg("Request failed (HTTP %lld).", &["499"])
        );
        assert_eq!(
            http(600).user_summary(),
            msg("Request failed (HTTP %lld).", &["600"])
        );
    }

    #[test]
    fn details() {
        let detail = http(500).user_detail().unwrap();
        assert_eq!(
            detail,
            vec![
                msg("HTTP %@ from %@", &["500", "https://x/"]),
                msg(
                    "Response body omitted for privacy (%@ characters).",
                    &["11"]
                ),
            ]
        );
        assert_eq!(
            english_lines(&detail),
            "HTTP 500 from https://x/\nResponse body omitted for privacy (11 characters)."
        );
        assert_eq!(
            TranslationError::Decoding("raw detail".into()).user_detail(),
            Some(vec![msg(VERBATIM_KEY, &["raw detail"])])
        );
        assert_eq!(
            TranslationError::Network("timed out".into()).user_detail(),
            Some(vec![msg(VERBATIM_KEY, &["timed out"])])
        );
        for e in [
            TranslationError::MissingApiKey,
            TranslationError::InvalidUrl,
            TranslationError::EmptyResponse,
            TranslationError::InvalidConfiguration("x".into()),
            TranslationError::ProviderNotFound(ProviderType::Claude),
            TranslationError::OperationNotSupported {
                provider: ProviderType::Claude,
                operation: AIOperation::Magic,
            },
        ] {
            assert_eq!(e.user_detail(), None, "{e:?}");
        }
    }

    #[test]
    fn http_detail_without_body_is_only_the_status_line() {
        let e = TranslationError::Http {
            status: 502,
            body: "  \n".into(),
            url: "https://x".into(),
        };
        let detail = e.user_detail().unwrap();
        assert_eq!(detail, vec![msg("HTTP %@ from %@", &["502", "https://x/"])]);
        assert_eq!(english_lines(&detail), "HTTP 502 from https://x/");
    }

    #[test]
    fn english_renders_placeholders() {
        assert_eq!(
            msg("HTTP %@ from %@", &["500", "https://x"]).english(),
            "HTTP 500 from https://x"
        );
        assert_eq!(
            msg("Request failed (HTTP %lld).", &["418"]).english(),
            "Request failed (HTTP 418)."
        );
        assert_eq!(msg("plain", &[]).english(), "plain");
        // Mixed placeholders fill in order; a substituted value is never re-scanned.
        assert_eq!(msg("%@ then %lld", &["%lld", "7"]).english(), "%lld then 7");
        // Missing arguments leave the placeholder as is.
        assert_eq!(msg("a %@ b %@", &["1"]).english(), "a 1 b %@");
    }

    #[test]
    fn sanitized_http_body_detail_never_echoes_the_body() {
        assert_eq!(sanitized_http_body_detail(""), None);
        assert_eq!(sanitized_http_body_detail("  \n\t "), None);
        let m = sanitized_http_body_detail("secret body").unwrap();
        assert_eq!(m.key, "Response body omitted for privacy (%@ characters).");
        assert_eq!(m.args, ["11"]);
        assert!(!m.english().contains("secret"));
    }

    #[test]
    fn redacted_url_strips_credentials_query_and_fragment() {
        let out = redacted_url("https://user:pw@api.x.com/v1/t?key=SECRET#frag");
        assert_eq!(out, "https://api.x.com/v1/t");
        for leak in ["SECRET", "user", "pw", "key", "frag"] {
            assert!(!out.contains(leak), "{leak} leaked in {out}");
        }
    }

    #[test]
    fn redacted_url_handles_unparsable_input() {
        assert_eq!(redacted_url("not a url"), "<redacted-url>");
        assert_eq!(redacted_url("/relative/path?key=SECRET"), "<redacted-url>");
        assert_eq!(redacted_url(""), "<redacted-url>");
    }

    #[test]
    fn redacted_url_keeps_plain_urls() {
        assert_eq!(
            redacted_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            redacted_url("https://host:8443/v1/models/m:generateContent?key=K"),
            "https://host:8443/v1/models/m:generateContent"
        );
    }

    #[test]
    fn http_error_never_carries_secrets_into_user_text() {
        let e = TranslationError::Http {
            status: 401,
            body: "{\"error\":\"bad key sk-LEAK\"}".into(),
            url: redacted_url("https://api.x.com/v1?key=sk-LEAK"),
        };
        let text = e.to_string();
        assert!(!text.contains("sk-LEAK"), "{text}");
    }

    #[test]
    fn http_url_is_redacted_inside_user_detail_and_display() {
        let e = TranslationError::Http {
            status: 401,
            body: String::new(),
            url: "https://h/p?key=SECRET".into(),
        };
        let detail = english_lines(&e.user_detail().unwrap());
        assert_eq!(detail, "HTTP 401 from https://h/p");
        let text = e.to_string();
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("key="), "{text}");
    }

    #[test]
    fn display_includes_summary_and_detail_like_error_description() {
        assert_eq!(
            TranslationError::InvalidUrl.to_string(),
            "Invalid base URL."
        );
        assert_eq!(
            http(401).to_string(),
            "Check the API key in Settings → Providers.\n\nHTTP 401 from https://x/\nResponse body omitted for privacy (11 characters)."
        );
        assert_eq!(
            TranslationError::Network("timed out".into()).to_string(),
            "Network error. Check your connection and try again.\n\ntimed out"
        );
        assert_eq!(
            TranslationError::InvalidConfiguration("bad".into()).to_string(),
            "Invalid configuration: bad"
        );
    }

    #[test]
    fn fallback_info_copies_summary_and_detail() {
        let e = http(429);
        let info = FallbackInfo::new(ProviderType::OpenAI, &e);
        assert_eq!(info.primary_provider, ProviderType::OpenAI);
        assert_eq!(info.message, e.user_summary());
        assert_eq!(info.details, e.user_detail());
        let none = FallbackInfo::new(ProviderType::Claude, &TranslationError::EmptyResponse);
        assert_eq!(none.details, None);
    }

    /// Every catalog key the errors can produce must exist verbatim in the mac app's
    /// `Localizable.xcstrings` (the translations are keyed by the English text). Local-only:
    /// the mac repo is a sibling checkout (`../lingo-pop`, or `LINGOPOP_MAC_CATALOG`); when the
    /// file is absent (CI) the test returns early without asserting.
    #[test]
    fn every_produced_key_exists_in_the_mac_catalog() {
        let path = std::env::var_os("LINGOPOP_MAC_CATALOG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../lingo-pop/App/Localizable.xcstrings")
            });
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("skipped: mac catalog not found at {}", path.display());
            return;
        };
        let catalog: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let strings = catalog["strings"].as_object().unwrap();

        let mut errors = vec![
            TranslationError::MissingApiKey,
            TranslationError::InvalidUrl,
            TranslationError::Decoding("d".into()),
            TranslationError::EmptyResponse,
            TranslationError::Network("n".into()),
            TranslationError::InvalidConfiguration("c".into()),
        ];
        for provider in ProviderType::ALL {
            errors.push(TranslationError::ProviderNotFound(provider));
        }
        for operation in AIOperation::ALL {
            errors.push(TranslationError::OperationNotSupported {
                provider: ProviderType::DeepL,
                operation,
            });
        }
        for status in [200, 401, 403, 404, 418, 429, 500, 599] {
            errors.push(http(status));
            errors.push(TranslationError::Http {
                status,
                body: " ".into(),
                url: "https://x".into(),
            });
        }
        let mut keys = std::collections::BTreeSet::new();
        for e in &errors {
            keys.insert(e.user_summary().key);
            for line in e.user_detail().unwrap_or_default() {
                if line.key != VERBATIM_KEY {
                    keys.insert(line.key);
                }
            }
        }
        for key in keys {
            assert!(strings.contains_key(key), "key not in mac catalog: {key:?}");
        }
    }
}
