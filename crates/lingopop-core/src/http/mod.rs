//! Async HTTP seam for the providers: a `Transport` trait (the real client arrives in a later
//! phase), request/response value types, and the shared helpers every provider goes through
//! (port of `Providers/ProviderHTTP.swift`). No secret ever appears in `Debug` output or in
//! error text: secret headers are redacted and URLs lose their query.

mod endpoint;
mod fake;

use std::fmt;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

pub use endpoint::endpoint;
pub use fake::FakeTransport;

use crate::errors::{redacted_url, TranslationError};

/// Header names whose values are never printed (compared case-insensitively).
const SECRET_HEADERS: [&str; 4] = [
    "x-api-key",
    "authorization",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    pub fn get(url: Url, headers: &[(&str, &str)]) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::Get,
            url,
            headers: owned_headers(headers),
            body: None,
        }
    }

    /// The value of the first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body parsed as JSON; `Null` when absent or not JSON.
    pub fn json_body(&self) -> serde_json::Value {
        self.body
            .as_deref()
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or(serde_json::Value::Null)
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let secret = SECRET_HEADERS.iter().any(|s| name.eq_ignore_ascii_case(s));
                (name.as_str(), if secret { "<redacted>" } else { value })
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &redacted_url(self.url.as_str()))
            .field("headers", &headers)
            .field("body_len", &self.body.as_ref().map(Vec::len))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }
}

/// Sends one request. Transport failures are `TranslationError::Network`; HTTP status
/// codes are not interpreted here (see `perform`).
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TranslationError>;
}

/// Sends `request` and returns the body of a 2xx response; any other status becomes
/// `TranslationError::Http` with the (redacted) URL. No retries, no per-status branching.
pub async fn perform(
    transport: &dyn Transport,
    request: HttpRequest,
) -> Result<Vec<u8>, TranslationError> {
    let url = redacted_url(request.url.as_str());
    let response = transport.send(request).await?;
    if (200..=299).contains(&response.status) {
        Ok(response.body)
    } else {
        Err(TranslationError::Http {
            status: response.status,
            body: String::from_utf8(response.body).unwrap_or_default(),
            url,
        })
    }
}

/// A POST with `Content-Type: application/json` first, then `headers`, and `body` as
/// compact JSON.
pub fn json_post(
    url: Url,
    headers: &[(&str, &str)],
    body: &impl Serialize,
) -> Result<HttpRequest, TranslationError> {
    let bytes =
        serde_json::to_vec(body).map_err(|err| TranslationError::Decoding(err.to_string()))?;
    let mut all = vec![("Content-Type".to_string(), "application/json".to_string())];
    all.extend(owned_headers(headers));
    Ok(HttpRequest {
        method: HttpMethod::Post,
        url,
        headers: all,
        body: Some(bytes),
    })
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, TranslationError> {
    serde_json::from_slice(bytes).map_err(|err| TranslationError::Decoding(err.to_string()))
}

/// MIME type of an image by its magic bytes: JPEG when it starts with `FF D8 FF`,
/// otherwise PNG (also for unknown or too-short data), like the Swift code.
pub fn mime_type(image: &[u8]) -> &'static str {
    if image.starts_with(&[0xFF, 0xD8, 0xFF]) && image.len() >= 4 {
        "image/jpeg"
    } else {
        "image/png"
    }
}

fn owned_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::TranslationError;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use url::Url;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn get(s: &str) -> HttpRequest {
        HttpRequest::get(url(s), &[])
    }

    #[tokio::test]
    async fn perform_returns_body_for_200_204_and_299() {
        for status in [200, 204, 299] {
            let fake = FakeTransport::json(status, "ok");
            let body = perform(&fake, get("https://api.x.com/v1")).await.unwrap();
            assert_eq!(body, b"ok", "status {status}");
        }
    }

    #[tokio::test]
    async fn perform_maps_non_2xx_to_http_error() {
        for status in [300, 302, 404, 429, 500] {
            let fake = FakeTransport::json(status, "boom");
            let err = perform(&fake, get("https://api.x.com/v1"))
                .await
                .unwrap_err();
            assert_eq!(
                err,
                TranslationError::Http {
                    status,
                    body: "boom".into(),
                    url: "https://api.x.com/v1".into(),
                }
            );
        }
    }

    #[tokio::test]
    async fn perform_non_utf8_body_becomes_empty_string() {
        let fake = FakeTransport::new(|_| Ok(HttpResponse::new(404, vec![0xFF, 0xFE])));
        let err = perform(&fake, get("https://api.x.com/v1"))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            TranslationError::Http {
                status: 404,
                body: String::new(),
                url: "https://api.x.com/v1".into(),
            }
        );
    }

    #[tokio::test]
    async fn perform_redacts_url_in_http_error() {
        let fake = FakeTransport::json(500, "boom");
        let err = perform(&fake, get("https://api.x.com/v1?q=SECRET"))
            .await
            .unwrap_err();
        let TranslationError::Http { url, .. } = &err else {
            panic!("expected Http, got {err:?}");
        };
        assert_eq!(url, "https://api.x.com/v1");
        assert!(!format!("{err:?} {err}").contains("SECRET"));
    }

    #[tokio::test]
    async fn perform_propagates_transport_error() {
        let fake = FakeTransport::new(|_| Err(TranslationError::Network("down".into())));
        let err = perform(&fake, get("https://api.x.com/v1"))
            .await
            .unwrap_err();
        assert_eq!(err, TranslationError::Network("down".into()));
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Reply {
        text: String,
    }

    #[test]
    fn decode_ok_and_decode_failure_is_decoding_error() {
        assert_eq!(
            decode::<Reply>(br#"{"text":"hi"}"#).unwrap(),
            Reply { text: "hi".into() }
        );
        for bad in [&b"not json"[..], br#"{"other":1}"#, br#"{"text":5}"#] {
            let err = decode::<Reply>(bad).unwrap_err();
            let TranslationError::Decoding(text) = err else {
                panic!("expected Decoding");
            };
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn json_post_sets_content_type_first_then_headers() {
        let request = json_post(
            url("https://api.x.com/v1"),
            &[("x-api-key", "k")],
            &json!({"a": 1}),
        )
        .unwrap();
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.headers,
            vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("x-api-key".to_string(), "k".to_string()),
            ]
        );
    }

    #[test]
    fn json_post_body_is_compact_utf8_json() {
        #[derive(Serialize)]
        struct Body {
            a: &'static str,
        }
        let request = json_post(url("https://api.x.com/v1"), &[], &json!({"a": "Привет"})).unwrap();
        assert_eq!(
            request.body.as_deref(),
            Some("{\"a\":\"Привет\"}".as_bytes())
        );
        let typed = json_post(url("https://api.x.com/v1"), &[], &Body { a: "x" }).unwrap();
        assert_eq!(typed.json_body(), json!({"a": "x"}));
    }

    #[test]
    fn json_body_is_null_when_absent_or_not_json() {
        assert_eq!(
            get("https://api.x.com/").json_body(),
            serde_json::Value::Null
        );
        let mut request = get("https://api.x.com/");
        request.body = Some(b"nope".to_vec());
        assert_eq!(request.json_body(), serde_json::Value::Null);
    }

    #[test]
    fn mime_type_follows_swift_rule() {
        assert_eq!(mime_type(&[0x89, 0x50, 0x4E, 0x47]), "image/png");
        assert_eq!(mime_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(mime_type(&[0xFF, 0xD8, 0xFF]), "image/png");
        assert_eq!(mime_type(b"GIF89a"), "image/png");
        assert_eq!(mime_type(&[]), "image/png");
    }

    #[test]
    fn http_request_debug_redacts_secret_headers() {
        let request = json_post(
            url("https://api.x.com/v1?q=URLSECRET"),
            &[
                ("x-api-key", "sk-ant-FIXTURE"),
                ("Authorization", "Bearer sk-ant-FIXTURE"),
                ("X-Goog-Api-Key", "sk-ant-FIXTURE"),
                ("OCP-APIM-SUBSCRIPTION-KEY", "sk-ant-FIXTURE"),
                ("Referer", "https://ref.example.com"),
            ],
            &json!({"prompt": "BODYSECRET"}),
        )
        .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("sk-ant-FIXTURE"), "{debug}");
        assert!(!debug.contains("URLSECRET"), "{debug}");
        assert!(!debug.contains("BODYSECRET"), "{debug}");
        assert_eq!(debug.matches("<redacted>").count(), 4, "{debug}");
        assert!(debug.contains("Content-Type"), "{debug}");
        assert!(debug.contains("application/json"), "{debug}");
        assert!(debug.contains("https://ref.example.com"), "{debug}");
        assert!(debug.contains("body_len: Some("), "{debug}");
    }

    #[test]
    fn http_request_header_lookup_is_case_insensitive() {
        let request = HttpRequest::get(
            url("https://api.x.com/"),
            &[("X-Api-Key", "k"), ("Accept", "a")],
        );
        assert_eq!(request.header("x-api-key"), Some("k"));
        assert_eq!(request.header("ACCEPT"), Some("a"));
        assert_eq!(request.header("missing"), None);
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.body, None);
    }
}
