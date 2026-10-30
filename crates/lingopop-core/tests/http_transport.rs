//! Loopback tests for `ReqwestTransport`: real sockets on 127.0.0.1 (random ports, plain
//! http), never the external network and never the developer's proxy settings.

use std::net::TcpListener;
use std::time::Duration;

use lingopop_core::errors::TranslationError;
use lingopop_core::http::{
    json_post, perform, HttpMethod, HttpRequest, HttpTimeouts, ReqwestTransport, Transport,
};
use serde_json::json;
use url::Url;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn transport() -> ReqwestTransport {
    ReqwestTransport::without_system_proxy(HttpTimeouts::default()).unwrap()
}

fn url(server: &MockServer, path_and_query: &str) -> Url {
    Url::parse(&format!("{}{path_and_query}", server.uri())).unwrap()
}

fn get(url: Url) -> HttpRequest {
    HttpRequest::get(url, &[])
}

#[tokio::test]
async fn success_200_passes_status_body_and_request_details() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/echo"))
        .and(header("x-api-key", "sk-ant-FIXTURE"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({"text": "hello"})))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"{\"ok\":true}"[..]))
        .expect(2)
        .mount(&server)
        .await;

    let request = json_post(
        url(&server, "/v1/echo"),
        &[("x-api-key", "sk-ant-FIXTURE")],
        &json!({"text": "hello"}),
    )
    .unwrap();
    assert_eq!(request.method, HttpMethod::Post);

    let response = transport().send(request.clone()).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{\"ok\":true}");

    let body = perform(&transport(), request).await.unwrap();
    assert_eq!(body, b"{\"ok\":true}");
}

#[tokio::test]
async fn default_user_agent_is_lingopop_with_crate_version() {
    let server = MockServer::start().await;
    let expected = format!("LingoPop/{}", lingopop_core::VERSION);
    Mock::given(method("GET"))
        .and(header("user-agent", expected.as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let response = transport().send(get(url(&server, "/ua"))).await.unwrap();
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn request_user_agent_overrides_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("user-agent", "Mozilla/5.0"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let request = HttpRequest::get(url(&server, "/ua"), &[("User-Agent", "Mozilla/5.0")]);
    let response = transport().send(request).await.unwrap();
    assert_eq!(response.status, 200);

    let seen = server.received_requests().await.unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].headers.get_all("user-agent").iter().count(), 1);
}

#[tokio::test]
async fn server_error_500_becomes_http_error_with_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let err = perform(&transport(), get(url(&server, "/fail?q=SECRETTEXT")))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        TranslationError::Http {
            status: 500,
            body: "boom".into(),
            url: url(&server, "/fail").to_string(),
        }
    );
}

#[tokio::test]
async fn not_found_with_non_utf8_body_gives_empty_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_bytes(vec![0xFF, 0xFE]))
        .mount(&server)
        .await;

    let err = perform(&transport(), get(url(&server, "/missing")))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        TranslationError::Http {
            status: 404,
            body: String::new(),
            url: url(&server, "/missing").to_string(),
        }
    );
}

#[tokio::test]
async fn same_origin_redirect_is_followed_and_keeps_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .and(header("x-api-key", "sk-ant-FIXTURE"))
        .respond_with(ResponseTemplate::new(200).set_body_string("landed"))
        .expect(1)
        .mount(&server)
        .await;

    let request = HttpRequest::get(url(&server, "/a"), &[("x-api-key", "sk-ant-FIXTURE")]);
    let response = transport().send(request).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"landed");
}

#[tokio::test]
async fn cross_origin_redirect_is_refused_and_not_contacted() {
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    assert_ne!(origin.address().port(), other.address().port());
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&other)
        .await;
    Mock::given(method("GET"))
        .and(path("/a"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", url(&other, "/steal").as_str()),
        )
        .mount(&origin)
        .await;

    let request = HttpRequest::get(url(&origin, "/a"), &[("x-api-key", "sk-ant-FIXTURE")]);
    let err = perform(&transport(), request).await.unwrap_err();
    assert!(
        matches!(err, TranslationError::Http { status: 302, .. }),
        "{err:?}"
    );
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn redirect_loop_is_cut_after_ten_hops() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", url(&server, "/loop").as_str()),
        )
        .mount(&server)
        .await;

    let err = perform(&transport(), get(url(&server, "/loop")))
        .await
        .unwrap_err();
    assert!(
        matches!(err, TranslationError::Http { status: 302, .. }),
        "{err:?}"
    );
    let count = server.received_requests().await.unwrap().len();
    assert!((2..=11).contains(&count), "{count} requests");
}

#[tokio::test]
async fn slow_response_times_out_as_network_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;

    let slow = ReqwestTransport::without_system_proxy(HttpTimeouts {
        connect: Duration::from_secs(1),
        read: Duration::from_millis(200),
        total: Duration::from_millis(500),
    })
    .unwrap();
    let err = slow.send(get(url(&server, "/slow"))).await.unwrap_err();
    assert!(matches!(err, TranslationError::Network(_)), "{err:?}");
}

/// A loopback URL whose port had a listener a moment ago, so connecting is refused.
fn refused_url(path_and_query: &str) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    Url::parse(&format!("http://127.0.0.1:{port}{path_and_query}")).unwrap()
}

#[tokio::test]
async fn connection_refused_is_network_error() {
    let err = transport().send(get(refused_url("/x"))).await.unwrap_err();
    assert!(matches!(err, TranslationError::Network(_)), "{err:?}");
}

#[tokio::test]
async fn network_error_message_never_contains_request_url() {
    let err = transport()
        .send(get(refused_url("/secret-path?q=SECRETTEXT")))
        .await
        .unwrap_err();
    let TranslationError::Network(message) = &err else {
        panic!("expected Network, got {err:?}");
    };
    assert!(!message.is_empty());
    for text in [message.clone(), format!("{err}"), format!("{err:?}")] {
        assert!(!text.contains("secret-path"), "{text}");
        assert!(!text.contains("SECRETTEXT"), "{text}");
    }
}

#[tokio::test]
async fn invalid_header_value_error_does_not_leak_it() {
    let request = HttpRequest::get(refused_url("/x"), &[("x-api-key", "sk-secret\nx")]);
    let err = transport().send(request).await.unwrap_err();
    assert!(
        matches!(err, TranslationError::InvalidConfiguration(_)),
        "{err:?}"
    );
    for text in [format!("{err}"), format!("{err:?}")] {
        assert!(!text.contains("sk-secret"), "{text}");
    }
}
