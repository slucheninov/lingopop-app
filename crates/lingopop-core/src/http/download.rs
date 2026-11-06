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
        let with_port = url.port().map(|p| format!("{host}:{p}"));
        self.entries
            .iter()
            .any(|e| *e == host || with_port.as_deref() == Some(e.as_str()))
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
        let mut guard = PartialFile {
            path: dest.to_owned(),
            file: Some(file),
            keep: false,
        };
        let written = self.stream(parsed, max_bytes, &mut guard, progress).await?;
        guard.keep = true;
        Ok(written)
    }

    async fn stream(
        &self,
        url: Url,
        max_bytes: u64,
        guard: &mut PartialFile,
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
            let file = guard.file.as_mut().ok_or(UpdateError::Download)?;
            file.write_all(&chunk).map_err(|_| UpdateError::Download)?;
            progress(received, length);
        }
        let file = guard.file.as_mut().ok_or(UpdateError::Download)?;
        file.flush().map_err(|_| UpdateError::Download)?;
        file.sync_all().map_err(|_| UpdateError::Download)?;
        Ok(received)
    }
}

/// Removes the file this call created unless `keep` was set.
struct PartialFile {
    path: PathBuf,
    file: Option<File>,
    keep: bool,
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        self.file.take();
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
