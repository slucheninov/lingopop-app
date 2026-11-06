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

/// The minisign (Ed25519) public key that signs every update asset (`<asset>.minisig`).
/// Rotating it breaks updates for every installed copy: they can never accept an update again.
pub const UPDATE_PUBLIC_KEY: &str = "RWSfFicENCknt5zLwsSazNoe+KYxLfJt8HaJQMPtLQAOSZEp8w90h11V";

/// SHA-1 of the self-signed macOS release signing certificate (40 lowercase hex). It is pinned in
/// the designated requirement, so it keeps Accessibility and Screen Recording across updates.
/// Rotating it breaks macOS permissions for every user. Must equal `release-signing.json`.
pub const RELEASE_CERT_SHA1: &str = "fed8ca55ad74676a130508b87ea9d7051a2bc0f8";

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

/// The version of a stable release tag: `X.Y.Z` with an optional leading `v`
/// and nothing else. Pre-release (`-alpha.1`, `-preview`, `-rc1`), build
/// metadata (`+build`) and any other shape is `None`, so the update check
/// never offers such a release.
pub fn stable_version(tag: &str) -> Option<String> {
    let text = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = text.split('.').collect();
    let numeric = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    (parts.len() == 3 && parts.iter().all(numeric)).then(|| text.to_owned())
}

/// `latest` counts only when it is a stable `X.Y.Z` version.
pub fn is_newer(latest: &str, current: &str) -> bool {
    if stable_version(latest).is_none() {
        return false;
    }
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
    fetch_release_with(transport, deadline)
        .await
        .map(|r| r.version)
}

/// Largest update asset the app downloads (300 MB).
pub const MAX_UPDATE_BYTES: u64 = 300 * 1024 * 1024;
/// Largest accepted `.minisig` asset.
const MAX_SIGNATURE_BYTES: u64 = 4096;
const DOWNLOAD_PATH_PREFIX: &str = "/slucheninov/lingopop-app/releases/download/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateTarget {
    WindowsX64,
    MacosArm64,
    MacosX64,
}

impl UpdateTarget {
    pub fn asset_name(&self, version: &str) -> String {
        match self {
            Self::WindowsX64 => format!("LingoPop-{version}-windows-x64-portable.exe"),
            Self::MacosArm64 => format!("LingoPop-{version}-macos-arm64.zip"),
            Self::MacosX64 => format!("LingoPop-{version}-macos-x64.zip"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// A stable, published release. `version` has no leading `v`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub version: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub signature_url: String,
    pub expected_comment: String,
}

/// Parses GitHub's release JSON. Prerelease, draft and non-stable tags are
/// `None`; an asset with missing or invalid fields is skipped.
pub fn parse_release(json: &[u8]) -> Option<Release> {
    #[derive(Deserialize)]
    struct Raw {
        tag_name: String,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        assets: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct RawAsset {
        name: String,
        size: u64,
        browser_download_url: String,
    }
    let raw: Raw = serde_json::from_slice(json).ok()?;
    if raw.prerelease || raw.draft {
        return None;
    }
    let version = stable_version(&raw.tag_name)?;
    let assets = raw
        .assets
        .into_iter()
        .filter_map(|value| serde_json::from_value::<RawAsset>(value).ok())
        .map(|a| ReleaseAsset {
            name: a.name,
            size: a.size,
            url: a.browser_download_url,
        })
        .collect();
    Some(Release { version, assets })
}

/// The URL is exactly the GitHub release download path of this repository,
/// tag and asset name: https, host `github.com`, no credentials, port, query
/// or fragment.
fn is_release_download_url(url: &str, version: &str, asset_name: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == format!("{DOWNLOAD_PATH_PREFIX}v{version}/{asset_name}")
}

/// `None` unless the release is newer than `current` and carries the target's
/// asset (1 byte..=`MAX_UPDATE_BYTES`) and its `.minisig` (1..=4096 bytes),
/// both at the exact release download URL.
pub fn update_plan(release: &Release, current: &str, target: UpdateTarget) -> Option<UpdatePlan> {
    if !is_newer(&release.version, current) {
        return None;
    }
    let asset_name = target.asset_name(&release.version);
    let signature_name = format!("{asset_name}.minisig");
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let asset = find(&asset_name)?;
    let signature = find(&signature_name)?;
    if asset.size == 0 || asset.size > MAX_UPDATE_BYTES {
        return None;
    }
    if signature.size == 0 || signature.size > MAX_SIGNATURE_BYTES {
        return None;
    }
    if !is_release_download_url(&asset.url, &release.version, &asset_name)
        || !is_release_download_url(&signature.url, &release.version, &signature_name)
    {
        return None;
    }
    Some(UpdatePlan {
        version: release.version.clone(),
        expected_comment: crate::update_sig::update_comment(&asset_name),
        asset_url: asset.url.clone(),
        asset_size: asset.size,
        signature_url: signature.url.clone(),
        asset_name,
    })
}

/// Silent latest-release lookup; the deadline wins if both are ready together.
pub async fn fetch_release_with(
    transport: &dyn Transport,
    deadline: impl Future<Output = ()>,
) -> Option<Release> {
    let request = async {
        let url = Url::parse(RELEASES_API_URL).ok()?;
        let bytes = perform(
            transport,
            HttpRequest::get(url, &[("Accept", "application/vnd.github+json")]),
        )
        .await
        .ok()?;
        parse_release(&bytes)
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
    fn only_stable_versions_count() {
        for tag in ["v0.3.0", "0.3.0", "v10.20.30"] {
            assert_eq!(
                stable_version(tag).as_deref(),
                Some(tag.trim_start_matches('v'))
            );
        }
        for tag in [
            "v0.3.0-alpha.1",
            "0.3.0-preview",
            "0.3.0-rc1",
            "0.3.0+build.5",
            "0.3",
            "0.3.0.1",
            "v0.3.x",
            "vv0.3.0",
            "",
            "preview",
        ] {
            assert_eq!(stable_version(tag), None, "{tag}");
        }
        assert!(!is_newer("0.3.0-preview", "0.2.0"));
        assert!(!is_newer("0.3.0+build", "0.2.0"));
        assert!(!is_newer("0.3", "0.2.0"));
        assert!(is_newer("v0.3.0", "0.2.0"));
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
        let fake = FakeTransport::json(
            200,
            r#"{"tag_name":"v0.3.0","prerelease":false,"draft":false}"#,
        );
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
        for body in [
            "bad",
            "{}",
            r#"{"tag_name":3}"#,
            r#"{"tag_name":"v0.3.0","prerelease":true}"#,
            r#"{"tag_name":"v0.3.0","draft":true}"#,
            r#"{"tag_name":"v0.3.0-preview"}"#,
            r#"{"tag_name":"v0.3.0-alpha.1","prerelease":false}"#,
            r#"{"tag_name":"v0.3.0+build"}"#,
        ] {
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

    #[test]
    fn release_cert_sha1_matches_release_signing_json() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../../release-signing.json")).unwrap();
        assert_eq!(json["macosCertificateSha1"], RELEASE_CERT_SHA1);
        assert_eq!(RELEASE_CERT_SHA1.len(), 40);
        assert!(RELEASE_CERT_SHA1
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn update_public_key_is_a_minisign_key_line() {
        assert!(UPDATE_PUBLIC_KEY.starts_with("RW"));
        assert_eq!(UPDATE_PUBLIC_KEY.len(), 56);
    }

    fn asset_json(name: &str, size: u64, version: &str) -> String {
        format!(
            r#"{{"name":"{name}","size":{size},"browser_download_url":"https://github.com/slucheninov/lingopop-app/releases/download/v{version}/{name}"}}"#
        )
    }

    fn release_json(version: &str, assets: &[String]) -> String {
        format!(
            r#"{{"tag_name":"v{version}","prerelease":false,"draft":false,"assets":[{}]}}"#,
            assets.join(",")
        )
    }

    fn full_release(version: &str) -> Release {
        let mut assets = Vec::new();
        for t in [
            UpdateTarget::WindowsX64,
            UpdateTarget::MacosArm64,
            UpdateTarget::MacosX64,
        ] {
            let name = t.asset_name(version);
            assets.push(asset_json(&name, 1000, version));
            assets.push(asset_json(&format!("{name}.minisig"), 300, version));
        }
        parse_release(release_json(version, &assets).as_bytes()).unwrap()
    }

    #[test]
    fn plan_for_each_target() {
        let release = full_release("0.2.2");
        for (t, name) in [
            (
                UpdateTarget::WindowsX64,
                "LingoPop-0.2.2-windows-x64-portable.exe",
            ),
            (UpdateTarget::MacosArm64, "LingoPop-0.2.2-macos-arm64.zip"),
            (UpdateTarget::MacosX64, "LingoPop-0.2.2-macos-x64.zip"),
        ] {
            let plan = update_plan(&release, "0.2.1", t).unwrap();
            let base = "https://github.com/slucheninov/lingopop-app/releases/download/v0.2.2/";
            assert_eq!(plan.version, "0.2.2");
            assert_eq!(plan.asset_name, name);
            assert_eq!(plan.asset_url, format!("{base}{name}"));
            assert_eq!(plan.signature_url, format!("{base}{name}.minisig"));
            assert_eq!(plan.asset_size, 1000);
            assert_eq!(plan.expected_comment, format!("lingopop-update {name}"));
        }
        assert_eq!(
            update_plan(&release, "0.2.1", UpdateTarget::MacosArm64)
                .unwrap()
                .expected_comment,
            "lingopop-update LingoPop-0.2.2-macos-arm64.zip"
        );
    }

    #[test]
    fn release_without_signature_has_no_plan() {
        let name = UpdateTarget::MacosArm64.asset_name("0.2.2");
        let r = parse_release(release_json("0.2.2", &[asset_json(&name, 10, "0.2.2")]).as_bytes())
            .unwrap();
        assert_eq!(update_plan(&r, "0.2.1", UpdateTarget::MacosArm64), None);
        // empty or oversized signature
        for size in [0, 4097] {
            let r = parse_release(
                release_json(
                    "0.2.2",
                    &[
                        asset_json(&name, 10, "0.2.2"),
                        asset_json(&format!("{name}.minisig"), size, "0.2.2"),
                    ],
                )
                .as_bytes(),
            )
            .unwrap();
            assert_eq!(update_plan(&r, "0.2.1", UpdateTarget::MacosArm64), None);
        }
    }

    #[test]
    fn not_newer_has_no_plan() {
        let release = full_release("0.2.2");
        for current in ["0.2.2", "0.2.3", "1.0.0"] {
            assert_eq!(update_plan(&release, current, UpdateTarget::MacosX64), None);
        }
    }

    #[test]
    fn foreign_url_has_no_plan() {
        let name = UpdateTarget::MacosArm64.asset_name("0.2.2");
        let good =
            format!("https://github.com/slucheninov/lingopop-app/releases/download/v0.2.2/{name}");
        let bad_urls = [
            format!("https://github.com/evil/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("http://github.com/slucheninov/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("https://evil.example/slucheninov/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("https://github.com.evil.example/slucheninov/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("https://github.com/slucheninov/lingopop-app/releases/download/v0.2.1/{name}"),
            format!("https://github.com/slucheninov/lingopop-app/releases/download/v0.2.2/other-{name}"),
            format!("https://user@github.com/slucheninov/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("https://github.com:444/slucheninov/lingopop-app/releases/download/v0.2.2/{name}"),
            format!("{good}?x=1"),
            "not a url".to_string(),
        ];
        for bad in bad_urls {
            // bad asset url
            let a = format!(r#"{{"name":"{name}","size":10,"browser_download_url":"{bad}"}}"#);
            let s = asset_json(&format!("{name}.minisig"), 10, "0.2.2");
            let r = parse_release(release_json("0.2.2", &[a, s]).as_bytes()).unwrap();
            assert_eq!(
                update_plan(&r, "0.2.1", UpdateTarget::MacosArm64),
                None,
                "{bad}"
            );
            // bad signature url
            let a = asset_json(&name, 10, "0.2.2");
            let s =
                format!(r#"{{"name":"{name}.minisig","size":10,"browser_download_url":"{bad}"}}"#);
            let r = parse_release(release_json("0.2.2", &[a, s]).as_bytes()).unwrap();
            assert_eq!(
                update_plan(&r, "0.2.1", UpdateTarget::MacosArm64),
                None,
                "{bad}"
            );
        }
    }

    #[test]
    fn oversized_asset_has_no_plan() {
        let name = UpdateTarget::WindowsX64.asset_name("0.2.2");
        for (size, ok) in [
            (0, false),
            (MAX_UPDATE_BYTES, true),
            (MAX_UPDATE_BYTES + 1, false),
            (301 * 1024 * 1024, false),
        ] {
            let r = parse_release(
                release_json(
                    "0.2.2",
                    &[
                        asset_json(&name, size, "0.2.2"),
                        asset_json(&format!("{name}.minisig"), 300, "0.2.2"),
                    ],
                )
                .as_bytes(),
            )
            .unwrap();
            assert_eq!(
                update_plan(&r, "0.2.1", UpdateTarget::WindowsX64).is_some(),
                ok,
                "{size}"
            );
        }
    }

    #[test]
    fn prerelease_or_draft_release_is_none() {
        for body in [
            r#"{"tag_name":"v0.3.0","prerelease":true,"assets":[]}"#,
            r#"{"tag_name":"v0.3.0","draft":true,"assets":[]}"#,
            r#"{"tag_name":"v0.3.0-rc1","assets":[]}"#,
            r#"{"tag_name":3}"#,
            "bad",
        ] {
            assert_eq!(parse_release(body.as_bytes()), None, "{body}");
        }
        let r = parse_release(br#"{"tag_name":"v0.3.0"}"#).unwrap();
        assert_eq!(r.version, "0.3.0");
        assert!(r.assets.is_empty());
    }

    #[test]
    fn malformed_assets_are_skipped_not_fatal() {
        let name = UpdateTarget::MacosX64.asset_name("0.2.2");
        let body = format!(
            r#"{{"tag_name":"v0.2.2","assets":[{{"name":1}},{{"name":"x"}},{{"name":"y","size":-1,"browser_download_url":"u"}},{},{}]}}"#,
            asset_json(&name, 5, "0.2.2"),
            asset_json(&format!("{name}.minisig"), 5, "0.2.2"),
        );
        let r = parse_release(body.as_bytes()).unwrap();
        assert_eq!(r.assets.len(), 2);
        assert!(update_plan(&r, "0.2.1", UpdateTarget::MacosX64).is_some());
    }

    #[tokio::test]
    async fn fetch_release_returns_assets() {
        let body = release_json("0.2.2", &[asset_json("a.zip", 1, "0.2.2")]);
        let r = fetch_release_with(&FakeTransport::json(200, &body), std::future::pending())
            .await
            .unwrap();
        assert_eq!(r.version, "0.2.2");
        assert_eq!(r.assets[0].name, "a.zip");
        assert_eq!(MAX_UPDATE_BYTES, 300 * 1024 * 1024);
    }
}
