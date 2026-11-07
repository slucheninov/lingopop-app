//! The update renames over `MoveFileExW`, all with `MOVEFILE_WRITE_THROUGH` so they return only
//! once the move is on disk (the update journal's durability order), never with
//! `MOVEFILE_COPY_ALLOWED` (same volume: a rename): the no-replace rename of the install steps,
//! and the replace that publishes the update journal from its flushed temp file.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use ::windows::core::PCWSTR;
use ::windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use super::util::win32_code;

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains a NUL character",
        ));
    }
    wide.push(0);
    Ok(wide)
}

/// Renames `from` to `to`, failing (`ERROR_ALREADY_EXISTS` / `ERROR_FILE_EXISTS`, i.e.
/// `AlreadyExists`) when `to` exists. Renaming a running exe is allowed by Windows.
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from = wide_path(from)?;
    let to = wide_path(to)?;
    // SAFETY: both buffers are NUL-terminated UTF-16 strings without interior NULs that outlive
    // the call; MoveFileExW only reads them. Only MOVEFILE_WRITE_THROUGH: no replace, no copy,
    // no reboot delay.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(win32_code(&e)))
}

/// Replaces `to` with `from` (`MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`): the
/// durable publication of the update journal from its flushed temp file.
pub(crate) fn replace_write_through(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(test)]
    REPLACE_CALLS.with(|c| c.set(c.get() + 1));
    let from = wide_path(from)?;
    let to = wide_path(to)?;
    // SAFETY: both buffers are NUL-terminated UTF-16 strings without interior NULs that outlive
    // the call; MoveFileExW only reads them. Replace + write-through only: no copy, no reboot
    // delay.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(win32_code(&e)))
}

#[cfg(test)]
thread_local! {
    static REPLACE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many journal publications this test thread made through [`replace_write_through`].
#[cfg(test)]
pub(crate) fn replace_calls() -> usize {
    REPLACE_CALLS.with(|c| c.get())
}
