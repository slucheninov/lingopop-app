//! The production `Transport`: a `reqwest` client with rustls (webpki roots, ring), timeouts
//! like the Swift `URLSession` setup and a same-origin-only redirect policy, so an API key
//! header never follows a redirect to another host.

use std::error::Error as _;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use url::Url;

use super::{is_secret_header, HttpMethod, HttpRequest, HttpResponse, Transport};
use crate::errors::TranslationError;

/// Redirect hops followed before the redirect response itself is returned.
const MAX_REDIRECTS: usize = 10;

/// Client timeouts. The default mirrors the Swift session: 30 s per request (here: connect
/// and read/idle) and 60 s for the whole transfer.
#[derive(Debug, Clone, Copy)]
pub struct HttpTimeouts {
    pub connect: Duration,
    pub read: Duration,
    pub total: Duration,
}

impl Default for HttpTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(30),
            read: Duration::from_secs(30),
            total: Duration::from_secs(60),
        }
    }
}

/// A redirect to `target` is followed only when it keeps the origin of the ORIGINAL request:
/// same scheme and host (case-insensitive) and same effective port, so an explicit default
/// port equals an omitted one.
pub fn redirect_allowed(original: &Url, target: &Url) -> bool {
    original.scheme().eq_ignore_ascii_case(target.scheme())
        && original.host_str().map(str::to_ascii_lowercase)
            == target.host_str().map(str::to_ascii_lowercase)
        && original.port_or_known_default() == target.port_or_known_default()
}

/// `Transport` over `reqwest`. It does not enforce https (that is `endpoint`'s job), which
/// lets the loopback tests talk plain http to 127.0.0.1.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(timeouts: HttpTimeouts) -> Result<Self, TranslationError> {
        Self::build(timeouts, false)
    }

    /// Same client, but it never consults proxy settings, so a developer's `HTTP_PROXY`
    /// cannot swallow loopback test requests.
    pub fn without_system_proxy(timeouts: HttpTimeouts) -> Result<Self, TranslationError> {
        Self::build(timeouts, true)
    }

    fn build(timeouts: HttpTimeouts, no_proxy: bool) -> Result<Self, TranslationError> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .read_timeout(timeouts.read)
            .timeout(timeouts.total)
            .user_agent(format!("LingoPop/{}", crate::VERSION))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    return attempt.stop();
                }
                match attempt.previous().first() {
                    Some(original) if redirect_allowed(original, attempt.url()) => attempt.follow(),
                    _ => attempt.stop(),
                }
            }));
        if no_proxy {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(network_error)?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TranslationError> {
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        let mut builder = self
            .client
            .request(method, request.url)
            .headers(header_map(&request.headers)?);
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(network_error)?;
        Ok(HttpResponse::new(status, body.to_vec()))
    }
}

/// Converts our headers into a `HeaderMap`, marking credential headers (see
/// `is_secret_header`) sensitive so they stay out of HTTP/2 HPACK indexing and `Debug`
/// output. Repeated names are kept. Only the header NAME may appear in an error; the value
/// can be an API key.
fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, TranslationError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let invalid =
            || TranslationError::InvalidConfiguration(format!("invalid HTTP header \"{name}\""));
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?;
        let mut header_value = HeaderValue::from_str(value).map_err(|_| invalid())?;
        if is_secret_header(name) {
            header_value.set_sensitive(true);
        }
        map.append(header_name, header_value);
    }
    Ok(map)
}

/// `Network` error text: the reqwest message without its URL, then the source chain joined
/// with ": ". The request URL (path, query) never appears.
fn network_error(err: reqwest::Error) -> TranslationError {
    let err = err.without_url();
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    TranslationError::Network(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn redirect_allowed_same_origin() {
        assert!(redirect_allowed(
            &url("https://api.x.com/a?q=1"),
            &url("https://api.x.com/b")
        ));
    }

    #[test]
    fn redirect_allowed_ignores_host_case() {
        assert!(redirect_allowed(
            &url("https://API.X.com/a"),
            &url("https://api.x.COM/b")
        ));
    }

    #[test]
    fn redirect_allowed_treats_explicit_default_port_as_equal() {
        assert!(redirect_allowed(
            &url("https://h:443/a"),
            &url("https://h/b")
        ));
        assert!(redirect_allowed(&url("http://h/a"), &url("http://h:80/b")));
    }

    #[test]
    fn redirect_refused_for_other_host() {
        assert!(!redirect_allowed(
            &url("https://api.x.com/a"),
            &url("https://evil.com/a")
        ));
    }

    #[test]
    fn redirect_refused_for_other_port() {
        assert!(!redirect_allowed(
            &url("https://h:8443/a"),
            &url("https://h/a")
        ));
    }

    #[test]
    fn redirect_refused_for_scheme_change() {
        assert!(!redirect_allowed(&url("https://h/a"), &url("http://h/a")));
    }

    fn hdrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn header_map_marks_credential_headers_sensitive() {
        let map = header_map(&hdrs(&[
            ("x-api-key", "k1"),
            ("Authorization", "Bearer k2"),
            ("x-goog-api-key", "k3"),
            ("Ocp-Apim-Subscription-Key", "k4"),
        ]))
        .unwrap();
        assert_eq!(map.len(), 4);
        for (name, value) in &map {
            assert!(value.is_sensitive(), "{name} should be sensitive");
        }
    }

    #[test]
    fn header_map_secret_match_is_case_insensitive() {
        let map = header_map(&hdrs(&[("X-API-KEY", "k"), ("AUTHORIZATION", "Bearer k")])).unwrap();
        assert!(map.get("x-api-key").unwrap().is_sensitive());
        assert!(map.get("authorization").unwrap().is_sensitive());
    }

    #[test]
    fn header_map_leaves_normal_headers_not_sensitive() {
        let map = header_map(&hdrs(&[
            ("Content-Type", "application/json"),
            ("x-api-key", "k"),
        ]))
        .unwrap();
        assert!(!map.get("content-type").unwrap().is_sensitive());
        assert!(map.get("x-api-key").unwrap().is_sensitive());
    }

    #[test]
    fn header_map_error_names_header_but_not_value() {
        let err = header_map(&hdrs(&[("x-api-key", "bad\nSECRET")])).unwrap_err();
        let shown = format!("{err:?} {err}");
        assert!(shown.contains("x-api-key"), "{shown}");
        assert!(!shown.contains("SECRET"), "{shown}");
    }

    #[test]
    fn http_timeouts_default_is_30_30_60() {
        let t = HttpTimeouts::default();
        assert_eq!(t.connect, Duration::from_secs(30));
        assert_eq!(t.read, Duration::from_secs(30));
        assert_eq!(t.total, Duration::from_secs(60));
    }
}
