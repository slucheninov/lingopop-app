//! The no-replace rename over `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` (same volume:
//! no `MOVEFILE_COPY_ALLOWED` either, so the move is a rename).

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use ::windows::core::PCWSTR;
use ::windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};

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
    // the call; MoveFileExW only reads them. Flags 0 = no replace, no copy, no reboot delay.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(win32_code(&e)))
}
