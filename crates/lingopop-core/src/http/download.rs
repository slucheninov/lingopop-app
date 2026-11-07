//! Streaming download of an update asset (or its `.minisig`) to a file.
//!
//! Rules: HTTPS only; the first URL and every redirect target must be on the host allowlist
//! (exact, case-insensitive host match, default port), at most 5 redirects. The production
//! allowlist is `github.com`, `objects.githubusercontent.com` and
//! `release-assets.githubusercontent.com`.
//!
//! Host check (2026-11-07): `curl -sI https://github.com/slucheninov/lingopop-app/releases/download/v0.2.0/LingoPop-0.2.0-windows-x64-portable.exe`
//! answered `302` with `Location: https://release-assets.githubusercontent.com/...`.
//! `objects.githubusercontent.com` is kept because GitHub has served assets from it before.
//!
//! Failure mapping: transport error or refused redirect/URL = `Network`; non-2xx = `Download`;
//! more than `max_bytes` (announced or streamed) = `TooLarge`; file write/sync error =
//! `Download`; an existing `dest` = `Install("download destination exists")` and is left alone.
//! A `dest` this call created is removed on any error and when the future is dropped.
//!
//! Timeouts: connect 30 s, no total timeout, 60 s per chunk read. File writes use `std::fs`
//! from async code (core has no direct tokio dependency).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::redirect::Policy;
use url::Url;

use crate::update_sig::UpdateError;

const MAX_REDIRECTS: usize = 5;
const PRODUCTION_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

#[derive(Debug, Clone)]
struct Allowlist {
    /// Lowercase `host` (default port only) or, tests only, `host:port`.
    entries: Vec<String>,
    /// Tests only: plain http is accepted for 127.0.0.1.
    loopback_http: bool,
}

impl Allowlist {
    fn permits(&self, url: &Url) -> bool {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        match url.scheme() {
            "https" => {}
            "http" if self.loopback_http && host == "127.0.0.1" => {}
            _ => return false,
        }
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        // `Url` normalises a default port to `None`, so a bare entry matches only that.
        let with_port = url.port().map(|p| format!("{host}:{p}"));
        self.entries.iter().any(|e| match url.port() {
            None => *e == host,
            Some(_) => with_port.as_deref() == Some(e.as_str()),
        })
    }
}

/// Streams update files to disk under the allowlist above.
#[derive(Debug, Clone)]
pub struct UpdateDownloader {
    client: reqwest::Client,
    allowlist: Allowlist,
}

impl Default for UpdateDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateDownloader {
    /// Production allowlist, rustls, system proxy settings on Windows only (like the rest of core).
    pub fn new() -> Self {
        let list = Allowlist {
            entries: PRODUCTION_HOSTS.iter().map(|h| (*h).to_owned()).collect(),
            loopback_http: false,
        };
        Self::build(list, false)
    }

    /// Tests only: `allowed` entries are `127.0.0.1` or `127.0.0.1:<port>` (a bare host
    /// matches the default port only, so loopback tests list `host:port`); plain http is
    /// accepted for 127.0.0.1 hosts; the system proxy is never used.
    #[doc(hidden)]
    pub fn for_tests(allowed: &[&str]) -> Self {
        let list = Allowlist {
            entries: allowed.iter().map(|h| h.to_ascii_lowercase()).collect(),
            loopback_http: true,
        };
        Self::build(list, true)
    }

    fn build(list: Allowlist, no_proxy: bool) -> Self {
        let policy_list = list.clone();
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .user_agent(format!("LingoPop/{}", crate::VERSION))
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() > MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if policy_list.permits(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect target not allowed")
                }
            }));
        if no_proxy {
            builder = builder.no_proxy();
        }
        Self {
            client: builder.build().expect("static reqwest client settings"),
            allowlist: list,
        }
    }

    /// Downloads `url` into the NEW file `dest` and returns the bytes written. `progress`
    /// gets `(received, content_length)` after each chunk. Dropping the future removes the
    /// partial file.
    pub async fn download(
        &self,
        url: &str,
        max_bytes: u64,
        dest: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, UpdateError> {
        let parsed = Url::parse(url).map_err(|_| UpdateError::Network)?;
        if !self.allowlist.permits(&parsed) {
            return Err(UpdateError::Network);
        }
        let file = match OpenOptions::new().write(true).create_new(true).open(dest) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(UpdateError::Install("download destination exists"))
            }
            Err(_) => return Err(UpdateError::Download),
        };
        // Declared before `file`, so the handle closes before the guard removes the path.
        let mut guard = PartialFile {
            path: dest.to_owned(),
            keep: false,
        };
        let mut file = file;
        let written = self.stream(parsed, max_bytes, &mut file, progress).await?;
        guard.keep = true;
        Ok(written)
    }

    async fn stream(
        &self,
        url: Url,
        max_bytes: u64,
        file: &mut File,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, UpdateError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| UpdateError::Network)?;
        if !response.status().is_success() {
            return Err(UpdateError::Download);
        }
        let length = response.content_length();
        if length.is_some_and(|l| l > max_bytes) {
            return Err(UpdateError::TooLarge);
        }
        let mut received: u64 = 0;
        while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
            received += chunk.len() as u64;
            if received > max_bytes {
                return Err(UpdateError::TooLarge);
            }
            file.write_all(&chunk).map_err(|_| UpdateError::Download)?;
            progress(received, length);
        }
        file.flush().map_err(|_| UpdateError::Download)?;
        file.sync_all().map_err(|_| UpdateError::Download)?;
        Ok(received)
    }
}

/// Removes the file this call created unless `keep` was set.
struct PartialFile {
    path: PathBuf,
    keep: bool,
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod() -> Allowlist {
        Allowlist {
            entries: PRODUCTION_HOSTS.iter().map(|h| (*h).to_owned()).collect(),
            loopback_http: false,
        }
    }
    fn ok(l: &Allowlist, u: &str) -> bool {
        l.permits(&Url::parse(u).unwrap())
    }

    #[test]
    fn production_hosts_and_default_https_port() {
        let l = prod();
        assert!(ok(&l, "https://github.com/a"));
        assert!(ok(&l, "https://GitHub.COM:443/a"));
        assert!(ok(&l, "https://release-assets.githubusercontent.com/a"));
        assert!(!ok(&l, "https://github.com:8443/a"));
    }

    #[test]
    fn production_refuses_downgrade_userinfo_and_lookalikes() {
        let l = prod();
        assert!(!ok(&l, "http://github.com/a"));
        assert!(!ok(&l, "http://objects.githubusercontent.com/a"));
        assert!(!ok(&l, "https://user@github.com/a"));
        assert!(!ok(&l, "https://user:pw@github.com/a"));
        assert!(!ok(&l, "https://github.com.evil.example/a"));
        assert!(!ok(&l, "https://evilgithub.com/a"));
        assert!(!ok(&l, "https://evil.example/github.com"));
        assert!(!ok(&l, "ftp://github.com/a"));
    }

    #[test]
    fn test_list_http_only_for_loopback() {
        let l = Allowlist {
            entries: vec!["127.0.0.1:9".into(), "github.com".into()],
            loopback_http: true,
        };
        assert!(ok(&l, "http://127.0.0.1:9/a"));
        assert!(!ok(&l, "http://127.0.0.1:10/a"));
        assert!(!ok(&l, "http://github.com/a"));
        assert!(!ok(&l, "http://user@127.0.0.1:9/a"));
    }
}
