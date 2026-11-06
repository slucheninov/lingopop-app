//! In-app update install: the install site, the update journal, startup recovery and the
//! Windows exe swap (spec "In-app updates" sections 3 and 4). Portable: no `unsafe`, compiled
//! on every OS; the no-replace / swap renames come from `src/windows/fs.rs` and
//! `src/macos/fs.rs` through [`native_rename_noreplace`] / [`native_rename_swap`] and are
//! injected everywhere, so tests can script failures.
//!
//! Safety rules (the reason this module exists):
//! - a failed or interrupted update never leaves the user without a working app: the backup is
//!   only ever moved, never deleted, until the journal proves which version is at the site;
//! - recovery never deletes a path that is not the updater's: only paths in the journal's
//!   `created` list, folders only with a matching owner marker, the backup only while it is
//!   still the backup and has the old identity, and never `site` or a path holding `running`.
//!
//! Log lines are fixed text, never paths.

mod exe;
mod identity;
mod journal;
mod recover;

use std::cell::Cell;
use std::io;
use std::path::Path;

pub use exe::{
    exe_support, install_exe, install_exe_with, old_exe_path, stage_exe, stage_exe_with,
    staged_exe_path, staging_folder, InstallOps,
};
pub use identity::{bundle_identity, file_sha256, identity_for};
pub use journal::{
    commit_journal, confirm, intend, read_journal, remove_journal, write_journal, Journal, Phase,
    JOURNAL_FILE, OWNER_MARKER,
};
pub use lingopop_core::update_sig::UpdateError;
pub use recover::{recover, recover_with, RecoverOps, Recovery};

use serde::{Deserialize, Serialize};

/// What kind of item the running app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SiteKind {
    /// The Windows portable exe (`<stem>.exe`).
    WindowsExe,
    /// The macOS `.app` bundle.
    MacBundle,
}

/// The install path recorded at startup: the exe or `.app` path the update replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSite {
    pub path: std::path::PathBuf,
    pub kind: SiteKind,
}

/// Whether the running app can update itself. The reason is a fixed log text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Supported(InstallSite),
    Unsupported(&'static str),
}

/// A fresh 128-bit random token: 32 lowercase hex characters from `getrandom`. Panics only
/// when the OS random source is unavailable, in which case nothing could be done safely.
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS random source");
    to_hex(&bytes)
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xF) as usize] as char);
    }
    s
}

pub(crate) fn is_token(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The no-replace rename of this OS: fails (`AlreadyExists`) when `to` exists. macOS
/// `renamex_np(RENAME_EXCL)`, Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`;
/// other OSes (not targets) report `Unsupported`.
pub fn native_rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::fs::rename_noreplace(from, to)
    }
    #[cfg(windows)]
    {
        crate::windows::fs::rename_noreplace(from, to)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no-replace rename is not available on this OS",
        ))
    }
}

/// The atomic exchange of two paths on this OS: macOS `renamex_np(RENAME_SWAP)`; elsewhere
/// `Unsupported` (the Windows recovery does its swap-back with two no-replace renames).
pub fn native_rename_swap(a: &Path, b: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::fs::rename_swap(a, b)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (a, b);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "rename swap is not available on this OS",
        ))
    }
}

/// Test hook points of the install and the recovery. Production passes a hook that always
/// returns [`HookAction::Continue`].
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// After the pre-move identity check of `site`, before the backup name is checked.
    BeforeFirstRename,
    /// After the backup name was checked and recorded in `intended`, right before the rename.
    BeforeBackupRename,
    /// Right after a failed no-replace rename, before any journal rewrite.
    AfterCollision,
    /// After the backup rename was recorded, before `site` is checked for the staged rename.
    BeforeSecondRename,
    /// After `site` was checked and recorded, right before the staged exe is renamed onto it.
    BeforeStagedRename,
    /// Before the rollback rename of `old` onto `site`.
    BeforeRollback,
    /// After the rollback rename succeeded, before `oldRestored` is written.
    AfterRollbackMove,
    /// After `oldRestored` of the rollback was written, before the cleanup.
    AfterRollback,
    /// Recovery: after `old` was renamed onto a missing `site`, before `oldRestored` is written.
    AfterRestore,
    /// Recovery: after the swap-back of a pending swap, before `oldRestored` is written.
    AfterSwapBack,
    /// Cleanup: after a file was deleted.
    AfterDelete,
    /// Cleanup: after an owned folder was removed.
    AfterFolderRemoved,
}

/// What a test hook asks for.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    Continue,
    /// Stop at once as if the process died: nothing more is written or deleted.
    Crash,
    /// The next journal write fails.
    FailNextWrite,
}

#[doc(hidden)]
pub fn no_hook(_: Step) -> HookAction {
    HookAction::Continue
}

/// A simulated crash from a hook.
pub(crate) struct Crashed;

/// Shared state of one install or recovery run: the injected operations and the hook.
pub(crate) struct Ctx<'a> {
    pub data_dir: &'a Path,
    pub running: &'a Path,
    pub identity: &'a dyn Fn(&Path) -> Option<String>,
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    pub hook: &'a dyn Fn(Step) -> HookAction,
    fail_next_write: Cell<bool>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        data_dir: &'a Path,
        running: &'a Path,
        identity: &'a dyn Fn(&Path) -> Option<String>,
        rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
        hook: &'a dyn Fn(Step) -> HookAction,
    ) -> Self {
        Self {
            data_dir,
            running,
            identity,
            rename,
            hook,
            fail_next_write: Cell::new(false),
        }
    }

    pub fn at(&self, step: Step) -> Result<(), Crashed> {
        match (self.hook)(step) {
            HookAction::Continue => Ok(()),
            HookAction::Crash => Err(Crashed),
            HookAction::FailNextWrite => {
                self.fail_next_write.set(true);
                Ok(())
            }
        }
    }

    pub fn write(&self, j: &Journal) -> Result<(), UpdateError> {
        if self.fail_next_write.replace(false) {
            return Err(UpdateError::Install("update journal could not be written"));
        }
        write_journal(self.data_dir, j)
    }
}

pub(crate) fn log(line: &'static str) {
    crate::error::platform_log(line);
}
