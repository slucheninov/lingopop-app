//! Loopback tests for `UpdateDownloader` (wiremock on 127.0.0.1, no external network).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use lingopop_core::http::UpdateDownloader;
use lingopop_core::update_sig::UpdateError;
use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn host_port(server: &MockServer) -> String {
    server.uri().trim_start_matches("http://").to_owned()
}

fn noop(_: u64, _: Option<u64>) {}

#[tokio::test]
async fn downloads_and_reports_progress() {
    let server = MockServer::start().await;
    let body = vec![7u8; 100_000];
    Mock::given(method("GET"))
        .and(path("/a"))
        .and(header_regex("user-agent", "^LingoPop/"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let calls = Mutex::new(Vec::new());
    let dl = UpdateDownloader::for_tests(&[&host_port(&server)]);
    let n = dl
        .download(&format!("{}/a", server.uri()), 1 << 20, &dest, &|r, t| {
            calls.lock().unwrap().push((r, t))
        })
        .await
        .unwrap();
    assert_eq!(n, 100_000);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    let calls = calls.lock().unwrap();
    assert!(!calls.is_empty());
    assert_eq!(calls.last().unwrap(), &(100_000, Some(100_000)));
    assert!(calls.windows(2).all(|w| w[0].0 < w[1].0));
}

#[tokio::test]
async fn redirect_to_allowed_host_is_followed() {
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/obj"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"hello"[..]))
        .mount(&target)
        .await;
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/a"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/obj", target.uri())),
        )
        .mount(&origin)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&host_port(&origin), &host_port(&target)]);
    let n = dl
        .download(&format!("{}/a", origin.uri()), 100, &dest, &noop)
        .await
        .unwrap();
    assert_eq!(n, 5);
}

#[tokio::test]
async fn redirect_to_foreign_host_is_rejected() {
    let target = MockServer::start().await;
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/obj", target.uri())),
        )
        .mount(&origin)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&host_port(&origin)]);
    let err = dl
        .download(&format!("{}/a", origin.uri()), 100, &dest, &noop)
        .await
        .unwrap_err();
    assert_eq!(err, UpdateError::Network);
    assert!(!dest.exists());
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn first_url_must_be_allowed_and_https() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    // not on the list
    let dl = UpdateDownloader::for_tests(&["127.0.0.1:1"]);
    let err = dl
        .download(&format!("{}/a", server.uri()), 100, &dest, &noop)
        .await;
    assert_eq!(err, Err(UpdateError::Network));
    // production list: http and foreign hosts are refused before any connection
    let prod = UpdateDownloader::new();
    for u in [
        "http://github.com/x",
        "https://evil.example/x",
        "https://github.com.evil.example/x",
        "not a url",
    ] {
        assert_eq!(
            prod.download(u, 100, &dest, &noop).await,
            Err(UpdateError::Network)
        );
    }
    assert!(!dest.exists());
}

#[tokio::test]
async fn oversized_body_fails_and_removes_dest() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 50]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&host_port(&server)]);
    let err = dl
        .download(&format!("{}/a", server.uri()), 10, &dest, &noop)
        .await;
    assert_eq!(err, Err(UpdateError::TooLarge));
    assert!(!dest.exists());
}

#[tokio::test]
async fn truncated_body_fails_and_removes_dest() {
    // Declares 1000 bytes, sends 5, then closes: the stream errors.
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = tokio::io::AsyncReadExt::read(&mut s, &mut buf).await;
        s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\nhello")
            .await
            .unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&addr.to_string()]);
    let err = dl
        .download(&format!("http://{addr}/a"), 10_000, &dest, &noop)
        .await;
    assert_eq!(err, Err(UpdateError::Network));
    assert!(!dest.exists());
}

#[tokio::test]
async fn short_content_length_returns_its_own_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 10]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&host_port(&server)]);
    let n = dl
        .download(&format!("{}/a", server.uri()), 20, &dest, &noop)
        .await;
    assert_eq!(n, Ok(10));
}

#[tokio::test]
async fn http_error_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&host_port(&server)]);
    let err = dl
        .download(&format!("{}/a", server.uri()), 20, &dest, &noop)
        .await;
    assert_eq!(err, Err(UpdateError::Download));
    assert!(!dest.exists());
}

#[tokio::test]
async fn existing_dest_is_refused_and_kept() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    std::fs::write(&dest, b"mine").unwrap();
    let dl = UpdateDownloader::for_tests(&[&host_port(&server)]);
    let err = dl
        .download(&format!("{}/a", server.uri()), 20, &dest, &noop)
        .await;
    assert_eq!(
        err,
        Err(UpdateError::Install("download destination exists"))
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"mine");
}

#[tokio::test]
async fn cancel_removes_partial_file() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = tokio::io::AsyncReadExt::read(&mut s, &mut buf).await;
        s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\nhello")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.bin");
    let dl = UpdateDownloader::for_tests(&[&addr.to_string()]);
    let seen = AtomicU64::new(0);
    let url = format!("http://{addr}/a");
    let report = |r: u64, _: Option<u64>| seen.store(r, Ordering::SeqCst);
    let fut = dl.download(&url, 10_000, &dest, &report);
    // Dropped as soon as the first chunk was reported.
    let _ = tokio::time::timeout(Duration::from_secs(2), fut).await;
    assert_eq!(seen.load(Ordering::SeqCst), 5);
    assert!(!dest.exists());
}
