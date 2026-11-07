//! In-app update install: the install site, the update journal, startup recovery, the
//! Windows exe swap and the macOS bundle swap (spec "In-app updates" sections 3, 4 and 5). Portable: no `unsafe`, compiled
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
//! Durability: every journal state is made durable (the journal file and the data folder are
//! synced after the atomic rename) before the step it announces runs, and every install
//! rename, swap, rollback and extraction is synced (the parent folders involved) before the
//! next journal state is written; see [`native_sync`]. A barrier that fails before a
//! destructive step aborts the attempt; one that fails after a completed step is logged
//! (`update sync failed`) and the journal update that follows goes on.
//!
//! Log lines are fixed text, never paths.

mod bundle;
mod exe;
mod identity;
mod journal;
mod recover;

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use bundle::{
    bundle_staging_folder, bundle_support, bundle_zip_path, install_bundle, install_bundle_with,
    pinned_requirement, read_bundle_info, stage_bundle, stage_bundle_with, BundleTools, BUNDLE_ID,
    BUNDLE_NAME, BUNDLE_ZIP_FILE,
};
pub use exe::{
    exe_support, install_exe, install_exe_with, old_exe_path, stage_exe, stage_exe_with,
    staged_exe_path, staging_folder, InstallOps,
};
#[cfg(target_os = "macos")]
pub(crate) use identity::bundle_executable;
pub use identity::{bundle_identity, file_sha256, identity_for};
pub use journal::{
    commit_journal, confirm, intend, read_journal, remove_journal, write_journal, Journal, Phase,
    JOURNAL_FILE, OWNER_MARKER,
};
pub use lingopop_core::update_sig::UpdateError;
pub use recover::{
    recover, recover_after_install, recover_after_install_with, recover_with, RecoverOps, Recovery,
};

pub(crate) use journal::write_journal_with;

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

/// A fresh 128-bit random token: 32 lowercase hex characters from `getrandom`. An unavailable
/// OS random source is `Err(Install("random source unavailable"))`: no attempt starts.
pub fn new_token() -> Result<String, UpdateError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| UpdateError::Install("random source unavailable"))?;
    Ok(to_hex(&bytes))
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

/// The durability barrier of this OS for one path (a file or a folder), the one every journal
/// write, install move and extraction goes through:
/// - macOS: `File::sync_all`, which std implements as `fcntl(F_FULLFSYNC)` on Apple targets
///   (checked in the std source of the toolchain, `sys/fs/unix.rs` `os_fsync`), for a folder
///   too: the rename / swap entries reach the disk, not only its cache;
/// - Windows: a file is opened for writing and `sync_all`ed (`FlushFileBuffers`); a folder is
///   a no-op (NTFS journals its metadata, and every install rename is `MoveFileExW` with
///   `MOVEFILE_WRITE_THROUGH`);
/// - other Unix: `fsync` of the file or folder.
///
/// A symbolic link is not followed (its folder's barrier covers it).
pub fn native_sync(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        if meta.is_dir() {
            return Ok(());
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_all()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(path)?.sync_all()
    }
}

/// Syncs every file under `root`, then each folder after its contents, then `root` itself
/// (best effort over a whole extracted bundle; symbolic links are not followed).
pub(crate) fn sync_tree(sync: &dyn Fn(&Path) -> io::Result<()>, root: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(root)?;
    if meta.is_dir() {
        let mut entries: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(root)? {
            entries.push(entry?.path());
        }
        entries.sort();
        for entry in entries {
            sync_tree(sync, &entry)?;
        }
    }
    sync(root)
}

/// Process memory of where recovery moved THIS process's own running image (the folder of a
/// swapped-back install). Recovery and cleanup never delete such a path or a folder holding
/// it; the next start (a new process, the previous version) cleans it up. Only recovery adds
/// to it, and only when `running` lay inside `site` before its swap back.
#[derive(Debug, Default)]
pub struct RunningCopies(Mutex<Vec<PathBuf>>);

impl RunningCopies {
    /// An empty memory (tests model a new process with a fresh one).
    pub const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub(crate) fn add(&self, path: &Path) {
        let mut paths = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !paths.iter().any(|p| p == path) {
            paths.push(path.to_owned());
        }
    }

    /// True when deleting `path` could delete a relocated running copy: it is one, or holds one.
    pub(crate) fn guards(&self, path: &Path) -> bool {
        let paths = self.0.lock().unwrap_or_else(|e| e.into_inner());
        paths.iter().any(|p| p.starts_with(path))
    }
}

/// The one memory of the running process.
pub(crate) static PROCESS_RUNNING_COPIES: RunningCopies = RunningCopies::new();

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
    /// Bundle: after `newHash` and the pre-swap checks, right before the `RENAME_SWAP`.
    BeforeBundleSwap,
    /// Bundle: right after the swap succeeded, before the late check.
    AfterBundleSwap,
    /// Before the rollback (exe: rename of `old` onto `site`; bundle: the swap back).
    BeforeRollback,
    /// After the rollback rename succeeded, before `oldRestored` is written.
    AfterRollbackMove,
    /// After `oldRestored` of the rollback / bundle swap back was written, before the cleanup.
    AfterRollback,
    /// Recovery: after `old` was renamed onto a missing `site`, before `oldRestored` is written.
    AfterRestore,
    /// After the swap back of a pending swap (recovery) or after a failed late check (bundle
    /// install), before `oldRestored` is written.
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
    /// The durability barrier ([`native_sync`] in production).
    pub sync: &'a dyn Fn(&Path) -> io::Result<()>,
    /// Where recovery moved this process's running image ([`PROCESS_RUNNING_COPIES`]).
    pub copies: &'a RunningCopies,
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
            sync: &native_sync,
            copies: &PROCESS_RUNNING_COPIES,
            fail_next_write: Cell::new(false),
        }
    }

    /// Replaces the durability barrier (tests).
    pub fn with_sync(mut self, sync: &'a dyn Fn(&Path) -> io::Result<()>) -> Self {
        self.sync = sync;
        self
    }

    /// Replaces the running-copies memory (tests).
    pub fn with_copies(mut self, copies: &'a RunningCopies) -> Self {
        self.copies = copies;
        self
    }

    /// A barrier BEFORE a destructive step: every path is synced, the first failure returned.
    pub fn barrier(&self, paths: &[&Path]) -> io::Result<()> {
        paths.iter().try_for_each(|p| (self.sync)(p))
    }

    /// A barrier AFTER a completed step: every path is synced; a failure is logged and the
    /// caller goes on with the journal update.
    pub fn settle(&self, paths: &[&Path]) {
        for p in paths {
            if (self.sync)(p).is_err() {
                log("update sync failed");
            }
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
        write_journal_with(self.data_dir, j, self.sync)
    }
}

pub(crate) fn log(line: &'static str) {
    crate::error::platform_log(line);
}
