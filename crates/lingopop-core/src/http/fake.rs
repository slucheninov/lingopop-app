//! In-memory `Transport` for tests (always compiled so later phases can reuse it).

use std::sync::Mutex;

use async_trait::async_trait;

use super::{HttpRequest, HttpResponse, Transport};
use crate::errors::TranslationError;

type Handler = dyn Fn(&HttpRequest) -> Result<HttpResponse, TranslationError> + Send + Sync;

/// A `Transport` that answers from a closure and records every request in order.
pub struct FakeTransport {
    handler: Box<Handler>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeTransport {
    pub fn new(
        handler: impl Fn(&HttpRequest) -> Result<HttpResponse, TranslationError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            handler: Box::new(handler),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Always answers with `status` and `body`.
    pub fn json(status: u16, body: &str) -> Self {
        let response = HttpResponse::new(status, body.as_bytes());
        Self::new(move |_| Ok(response.clone()))
    }

    /// Answers by `url.host_str()`; an unknown host is `Network("no route")`.
    pub fn by_host(routes: Vec<(&str, HttpResponse)>) -> Self {
        let routes: Vec<(String, HttpResponse)> = routes
            .into_iter()
            .map(|(host, response)| (host.to_owned(), response))
            .collect();
        Self::new(move |request| {
            let host = request.url.host_str().unwrap_or_default();
            routes
                .iter()
                .find(|(h, _)| h == host)
                .map(|(_, response)| response.clone())
                .ok_or_else(|| TranslationError::Network("no route".to_string()))
        })
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl Transport for FakeTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TranslationError> {
        let result = (self.handler)(&request);
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn get(url: &str) -> HttpRequest {
        HttpRequest::get(Url::parse(url).unwrap(), &[])
    }

    #[tokio::test]
    async fn fake_records_requests_in_order() {
        let fake = FakeTransport::json(200, "{}");
        fake.send(get("https://a.example.com/1")).await.unwrap();
        fake.send(get("https://b.example.com/2")).await.unwrap();
        let urls: Vec<String> = fake.requests().iter().map(|r| r.url.to_string()).collect();
        assert_eq!(urls, ["https://a.example.com/1", "https://b.example.com/2"]);
    }

    #[tokio::test]
    async fn fake_json_returns_fixed_response() {
        let fake = FakeTransport::json(201, r#"{"ok":true}"#);
        let first = fake.send(get("https://a.example.com/")).await.unwrap();
        let second = fake.send(get("https://b.example.com/")).await.unwrap();
        assert_eq!(first, HttpResponse::new(201, r#"{"ok":true}"#));
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn fake_by_host_routes_and_unknown_host_is_network_error() {
        let fake = FakeTransport::by_host(vec![
            ("a.example.com", HttpResponse::new(200, "A")),
            ("b.example.com", HttpResponse::new(404, "B")),
        ]);
        let a = fake.send(get("https://a.example.com/x")).await.unwrap();
        let b = fake.send(get("https://b.example.com/x")).await.unwrap();
        assert_eq!(a, HttpResponse::new(200, "A"));
        assert_eq!(b, HttpResponse::new(404, "B"));
        let err = fake.send(get("https://c.example.com/x")).await.unwrap_err();
        assert_eq!(err, TranslationError::Network("no route".into()));
        assert_eq!(fake.requests().len(), 3);
    }
}
