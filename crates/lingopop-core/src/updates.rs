//! Pure update rules and a silent release lookup with an injected deadline.
use crate::http::{perform, HttpRequest, Transport};
use serde::Deserialize;
use std::{
    future::{poll_fn, Future},
    pin::pin,
    task::Poll,
    time::Duration,
};
use url::Url;

pub const RELEASES_API_URL: &str =
    "https://api.github.com/repos/slucheninov/lingopop-app/releases/latest";
pub const RELEASES_PAGE_URL: &str = "https://github.com/slucheninov/lingopop-app/releases/latest";
pub const CHECK_INTERVAL_SECONDS: f64 = 10_800.0;
pub const UPDATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: bool,
}

pub fn parse_tag(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_owned()
}

pub fn parse_version(text: &str) -> Version {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok());
    Version {
        major: parts.next().unwrap_or(0),
        minor: parts.next().unwrap_or(0),
        patch: parts.next().unwrap_or(0),
        prerelease: text.contains('-'),
    }
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let latest = parse_version(latest);
    let current = parse_version(current);
    let l = (latest.major, latest.minor, latest.patch);
    let c = (current.major, current.minor, current.patch);
    l > c || (l == c && current.prerelease && !latest.prerelease)
}

pub fn should_check(last_check_unix: Option<f64>, now_unix: f64) -> bool {
    last_check_unix.is_none_or(|last| now_unix - last >= CHECK_INTERVAL_SECONDS)
}

/// No runtime timer in core: the shell should use `fetch_latest_version_with`
/// with its runtime's sleep for `UPDATE_REQUEST_TIMEOUT`.
pub async fn fetch_latest_version(transport: &dyn Transport) -> Option<String> {
    fetch_latest_version_with(transport, std::future::pending()).await
}

/// Drops the in-flight request when the supplied deadline completes.
/// The deadline wins if both futures are ready in the same poll.
pub async fn fetch_latest_version_with(
    transport: &dyn Transport,
    deadline: impl Future<Output = ()>,
) -> Option<String> {
    let request = async {
        let bytes = perform(
            transport,
            HttpRequest::get(
                Url::parse(RELEASES_API_URL).expect("const url"),
                &[("Accept", "application/vnd.github+json")],
            ),
        )
        .await
        .ok()?;
        #[derive(Deserialize)]
        struct Release {
            tag_name: String,
        }
        let release: Release = serde_json::from_slice(&bytes).ok()?;
        Some(parse_tag(&release.tag_name))
    };
    let mut request = pin!(request);
    let mut deadline = pin!(deadline);
    poll_fn(|cx| {
        if deadline.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        request.as_mut().poll(cx)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::TranslationError;
    use crate::http::{FakeTransport, HttpResponse};

    #[test]
    fn parse_version_strips_v_and_suffix() {
        for (text, numbers, prerelease) in [
            ("v1.2.3-beta.4", (1, 2, 3), true),
            ("1.2", (1, 2, 0), false),
            ("1.x.3", (1, 3, 0), false),
            ("bad", (0, 0, 0), false),
            ("1.2.3.4", (1, 2, 3), false),
        ] {
            let v = parse_version(text);
            assert_eq!((v.major, v.minor, v.patch), numbers);
            assert_eq!(v.prerelease, prerelease);
        }
        assert_eq!(parse_tag("vv1"), "v1");
    }

    #[test]
    fn newer_major_minor_patch() {
        for (latest, current, want) in [
            ("0.3.0", "0.2.9", true),
            ("0.2.0", "0.2.0", false),
            ("0.2.1", "0.2.1-alpha.5", true),
            ("1.0.0", "0.9.9", true),
            ("0.2.2", "0.2.1", true),
            ("0.1.9", "0.2.0", false),
        ] {
            assert_eq!(is_newer(latest, current), want);
        }
    }

    #[test]
    fn prerelease_current_sees_stable() {
        assert!(is_newer("0.2.0", "0.2.0-alpha.5"));
        assert!(!is_newer("0.2.0-alpha.6", "0.2.0-alpha.5"));
        assert!(!is_newer("0.2.0-rc1", "0.2.0"));
    }

    #[test]
    fn should_check_respects_three_hours() {
        let now = 1_700_000_000.0;
        assert!(should_check(None, now));
        assert!(!should_check(Some(now - 10_799.0), now));
        assert!(should_check(Some(now - 10_800.0), now));
        assert!(!should_check(Some(now + 1.0), now));
        assert_eq!(UPDATE_REQUEST_TIMEOUT, Duration::from_secs(10));
        assert_eq!(
            RELEASES_PAGE_URL,
            "https://github.com/slucheninov/lingopop-app/releases/latest"
        );
    }

    #[tokio::test]
    async fn fetch_latest_reads_tag_name() {
        let fake = FakeTransport::json(200, r#"{"tag_name":"v0.3.0"}"#);
        assert_eq!(fetch_latest_version(&fake).await, Some("0.3.0".into()));
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url.as_str(),
            "https://api.github.com/repos/slucheninov/lingopop-app/releases/latest"
        );
        assert_eq!(
            requests[0].header("Accept"),
            Some("application/vnd.github+json")
        );
    }

    #[tokio::test]
    async fn fetch_latest_is_none_on_404_and_network() {
        assert_eq!(
            fetch_latest_version(&FakeTransport::json(404, r#"{"tag_name":"v9.9.9"}"#)).await,
            None
        );
        let fake = FakeTransport::new(|_| Err(TranslationError::Network("offline".into())));
        assert_eq!(fetch_latest_version(&fake).await, None);
        for body in ["bad", "{}", r#"{"tag_name":3}"#] {
            assert_eq!(
                fetch_latest_version(&FakeTransport::json(200, body)).await,
                None
            );
        }
    }

    // FakeTransport has a synchronous handler, so use a test-only pending Transport.
    struct PendingTransport;
    #[async_trait::async_trait]
    impl Transport for PendingTransport {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, TranslationError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn fetch_latest_gives_up_at_deadline() {
        assert_eq!(
            fetch_latest_version_with(&PendingTransport, std::future::ready(())).await,
            None
        );
    }
}
