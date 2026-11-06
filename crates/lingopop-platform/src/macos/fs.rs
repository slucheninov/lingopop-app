//! No-replace and swap renames over `renamex_np` (macOS 10.12+, same volume).

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
}

fn renamex(from: &Path, to: &Path, flags: libc::c_uint) -> io::Result<()> {
    let from = c_path(from)?;
    let to = c_path(to)?;
    // SAFETY: both pointers come from `CString`s that are NUL-terminated and stay alive for the
    // duration of the call; `renamex_np` only reads them and reports failure through errno.
    let rc = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Renames `from` to `to`, failing with `AlreadyExists` (EEXIST) when `to` exists.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    renamex(from, to, libc::RENAME_EXCL)
}

/// Atomically exchanges `a` and `b` (both must exist, same volume).
pub(crate) fn rename_swap(a: &Path, b: &Path) -> io::Result<()> {
    renamex(a, b, libc::RENAME_SWAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noreplace_fails_on_an_existing_destination_and_swap_exchanges() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"A").unwrap();
        std::fs::write(&b, b"B").unwrap();
        let e = rename_noreplace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"B");
        rename_swap(&a, &b).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"B");
        assert_eq!(std::fs::read(&b).unwrap(), b"A");
        let c = dir.path().join("c");
        rename_noreplace(&a, &c).unwrap();
        assert_eq!(std::fs::read(&c).unwrap(), b"B");
        assert!(!a.exists());
    }
}
