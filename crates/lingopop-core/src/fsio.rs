//! File helpers shared by the stores: BOM rule, atomic write, optional read,
//! quarantine of unreadable files, and the load / error types.
//!
//! Windows note: std's `rename` replaces an existing file on Windows; transient
//! sharing/permission errors caused by antivirus or indexers are retried briefly.
//! Any error that remains is returned and the temp file removed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::time::Timestamp;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Distinguishes temp files created by the same process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How a store file was found on `open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No file, or an empty / whitespace-only / BOM-only one.
    Missing,
    /// A current envelope was read.
    Loaded,
    /// A legacy bare array was read; it is upgraded on the next write.
    Legacy,
    /// The file could not be decoded and was moved to this path; the store starts empty.
    CorruptBackedUp(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("file access failed: {0}")]
    Io(#[from] io::Error),
    #[error("could not encode the store file: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Removes a leading UTF-8 BOM (Notepad adds one when the user edits JSON).
/// This is the single BOM rule of the crate; a BOM anywhere else is kept.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes)
}

/// True for an empty file, or one holding only ASCII whitespace (after a BOM).
pub fn is_blank(bytes: &[u8]) -> bool {
    strip_bom(bytes).iter().all(u8::is_ascii_whitespace)
}

/// Reads a file; a missing file is `Ok(None)`, any other error is returned.
pub fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn file_name_of(path: &Path) -> io::Result<&std::ffi::OsStr> {
    path.file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))
}

/// Creates `<name>.tmp-<pid>-<n>` next to the target; never reuses an existing file.
fn create_temp(dir: &Path, name: &str, private: bool) -> io::Result<(File, PathBuf)> {
    let mut last_error = None;
    for _ in 0..16 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("{name}.tmp-{}-{n}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private; // no POSIX modes; the per-user data directory is the boundary
        match options.open(&tmp) {
            Ok(file) => return Ok((file, tmp)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last_error = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("could not create a temp file")))
}

/// Rename attempts: Windows retries briefly (antivirus / indexers can hold the file
/// for a few milliseconds); elsewhere a failure is final.
const RENAME_ATTEMPTS: u32 = if cfg!(windows) { 5 } else { 1 };

/// Backoff unit: the pause before attempt `n + 1` is `n` units (10, 20, 30, 40 ms).
const RENAME_BACKOFF_UNIT: Duration = Duration::from_millis(10);

/// True for a rename error that a short wait can clear: a permission error
/// everywhere, and on Windows also `ERROR_SHARING_VIOLATION` (32) and
/// `ERROR_LOCK_VIOLATION` (33).
fn is_transient_rename_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        return true;
    }
    false
}

/// Runs `rename` up to `attempts` times, sleeping `unit * n` after the `n`-th
/// transient failure; any other error, or the last failure, is returned.
fn retry_rename(
    mut rename: impl FnMut() -> io::Result<()>,
    attempts: u32,
    unit: Duration,
) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match rename() {
            Err(e) if attempt < attempts && is_transient_rename_error(&e) => {
                std::thread::sleep(unit * attempt);
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Writes `bytes` to a temp file in the target's directory, syncs it and renames it
/// over `path`; creates missing parent directories. On Unix `private` makes the
/// file mode 0600. On any error the temp file is removed and `path` is untouched.
pub fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let name = file_name_of(path)?.to_string_lossy().into_owned();
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let (mut file, tmp) = create_temp(dir, &name, private)?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file); // must be closed before the rename on Windows
            retry_rename(
                || fs::rename(&tmp, path),
                RENAME_ATTEMPTS,
                RENAME_BACKOFF_UNIT,
            )
        });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Reads the result of probing a candidate name: only `NotFound` means the name is
/// free, a successful probe means it is taken, and any other error is returned
/// (the caller must not rename anything then).
fn is_free<T>(probe: io::Result<T>) -> io::Result<bool> {
    match probe {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

/// Moves an unreadable file aside to `<file name>.bak-<unix seconds>` (with `-1`,
/// `-2`, ... appended if that name is taken) and returns the new path. If probing
/// a candidate name fails with anything but `NotFound`, the error is returned and
/// the file is not moved.
pub fn quarantine(path: &Path, now: Timestamp) -> io::Result<PathBuf> {
    let name = file_name_of(path)?.to_string_lossy().into_owned();
    let seconds = now.unix_seconds().floor() as i64;
    let base = format!("{name}.bak-{seconds}");
    let mut candidate = path.with_file_name(&base);
    let mut n = 0_u32;
    while !is_free(candidate.symlink_metadata())? {
        n += 1;
        candidate = path.with_file_name(format!("{base}-{n}"));
    }
    fs::rename(path, &candidate)?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bom_removes_only_a_leading_bom() {
        assert_eq!(strip_bom(&[0xEF, 0xBB, 0xBF, b'{', b'}']), b"{}");
        assert_eq!(strip_bom(b"{}"), b"{}");
        let middle = [b'{', 0xEF, 0xBB, 0xBF, b'}'];
        assert_eq!(strip_bom(&middle), &middle);
        assert_eq!(strip_bom(&[]), b"");
        assert_eq!(strip_bom(&[0xEF, 0xBB]), &[0xEF, 0xBB]);
        assert_eq!(strip_bom(&[0xEF, 0xBB, 0xBF]), b"");
    }

    use std::fs;
    use std::path::PathBuf;

    use crate::time::Timestamp;

    fn names_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn atomic_write_creates_parent_dirs_and_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("file.json");
        atomic_write(&path, b"one", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(names_in(path.parent().unwrap()), ["file.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_private_sets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.dat");
        atomic_write(&path, b"x", true).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn atomic_write_failure_parent_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"i am a file").unwrap();
        let result = atomic_write(&blocker.join("x.json"), b"data", false);
        assert!(result.is_err());
        assert_eq!(names_in(dir.path()), ["blocker"]);
        assert_eq!(fs::read(&blocker).unwrap(), b"i am a file");
    }

    #[test]
    fn atomic_write_failure_target_is_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("inner.txt"), b"keep").unwrap();
        assert!(atomic_write(&target, b"data", false).is_err());
        assert_eq!(fs::read(target.join("inner.txt")).unwrap(), b"keep");
        assert_eq!(names_in(dir.path()), ["target"]);
        assert_eq!(names_in(&target), ["inner.txt"]);
    }

    #[test]
    fn read_optional_maps_not_found_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        assert_eq!(read_optional(&path).unwrap(), None);
        fs::write(&path, b"{}").unwrap();
        assert_eq!(read_optional(&path).unwrap(), Some(b"{}".to_vec()));
    }

    #[test]
    fn is_blank_covers_empty_whitespace_and_bom_only() {
        assert!(is_blank(b""));
        assert!(is_blank(b"  \n\t\r\n"));
        assert!(is_blank(&[0xEF, 0xBB, 0xBF]));
        assert!(is_blank(&[0xEF, 0xBB, 0xBF, b' ', b'\n']));
        assert!(!is_blank(b"{}"));
        assert!(!is_blank(&[0xEF, 0xBB, 0xBF, b'[', b']']));
    }

    #[test]
    fn quarantine_names_file_bak_unix_seconds_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let now = Timestamp::from_unix_seconds(1_793_366_956.0);

        fs::write(&path, b"first").unwrap();
        let first = quarantine(&path, now).unwrap();
        assert_eq!(first, dir.path().join("history.json.bak-1793366956"));
        assert!(!path.exists());

        fs::write(&path, b"second").unwrap();
        let second = quarantine(&path, now).unwrap();
        let expected: PathBuf = dir.path().join("history.json.bak-1793366956-1");
        assert_eq!(second, expected);

        fs::write(&path, b"third").unwrap();
        let third = quarantine(&path, now).unwrap();
        assert_eq!(third, dir.path().join("history.json.bak-1793366956-2"));

        assert_eq!(fs::read(&first).unwrap(), b"first");
        assert_eq!(fs::read(&second).unwrap(), b"second");
        assert_eq!(fs::read(&third).unwrap(), b"third");
    }

    #[test]
    fn is_free_only_treats_not_found_as_free() {
        assert!(is_free::<()>(Err(io::Error::from(io::ErrorKind::NotFound))).unwrap());
        assert!(!is_free(Ok(())).unwrap());
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::Other,
        ] {
            let err = is_free::<()>(Err(io::Error::from(kind))).unwrap_err();
            assert_eq!(err.kind(), kind);
        }
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_returns_probe_errors_and_renames_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"i am a file").unwrap();
        // "blocker/x.json.bak-N" probes with ENOTDIR (not NotFound).
        let path = blocker.join("x.json");
        let err = quarantine(&path, Timestamp::from_unix_seconds(1_793_366_956.0)).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(names_in(dir.path()), ["blocker"]);
        assert_eq!(fs::read(&blocker).unwrap(), b"i am a file");
    }

    #[test]
    fn transient_rename_errors_are_recognised() {
        assert!(is_transient_rename_error(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_transient_rename_error(&io::Error::from(
            io::ErrorKind::NotFound
        )));
        assert!(!is_transient_rename_error(&io::Error::from(
            io::ErrorKind::AlreadyExists
        )));
        #[cfg(windows)]
        {
            assert!(is_transient_rename_error(&io::Error::from_raw_os_error(32)));
            assert!(is_transient_rename_error(&io::Error::from_raw_os_error(33)));
            assert!(!is_transient_rename_error(&io::Error::from_raw_os_error(2)));
        }
        #[cfg(unix)]
        {
            // 32 / 33 are EPIPE / EDOM on Unix, not sharing violations.
            assert!(!is_transient_rename_error(&io::Error::from_raw_os_error(
                32
            )));
            assert!(!is_transient_rename_error(&io::Error::from_raw_os_error(
                33
            )));
        }
    }

    #[test]
    fn retry_rename_retries_transient_errors_then_succeeds() {
        let mut calls = 0;
        let result = retry_rename(
            || {
                calls += 1;
                if calls < 3 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            },
            5,
            Duration::ZERO,
        );
        assert!(result.is_ok());
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_rename_is_bounded_and_returns_the_last_error() {
        let mut calls = 0;
        let result = retry_rename(
            || {
                calls += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            5,
            Duration::ZERO,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 5);
    }

    #[test]
    fn retry_rename_does_not_retry_other_errors_or_with_one_attempt() {
        let mut calls = 0;
        let result = retry_rename(
            || {
                calls += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            5,
            Duration::ZERO,
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);

        let mut calls = 0;
        let result = retry_rename(
            || {
                calls += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            1,
            Duration::ZERO,
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
}
