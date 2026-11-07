//! `update-journal.json`: the only thing recovery acts on (spec section 3, "Update journal").

use std::io;
use std::path::{Path, PathBuf};

use lingopop_core::fsio;
use serde::{Deserialize, Serialize};

use super::{SiteKind, UpdateError};

/// The journal's file name in the data folder.
pub const JOURNAL_FILE: &str = "update-journal.json";

/// The owner marker written with `create_new` inside every owned staging folder; it holds the
/// attempt token.
pub const OWNER_MARKER: &str = ".lingopop-owner";

/// `pending` when first written; `committed` only after the swap and every check after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Pending,
    Committed,
}

/// One update attempt. Every path it creates or moves is named here; recovery never matches
/// names alone.
///
/// Additions to the plan's field list: `kind` (which identity / swap recovery uses) and `token`
/// (what the owner marker of an owned folder must hold).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    /// The planned version.
    pub version: String,
    pub kind: SiteKind,
    /// The attempt's 128-bit token (32 lowercase hex).
    pub token: String,
    /// The install path recorded at startup.
    pub site: PathBuf,
    /// The new item before the swap (Windows: inside the owned staging folder).
    pub staged: PathBuf,
    /// Where the previous item is after the swap (Windows `<stem>.old-<token>.exe`, macOS the
    /// staged path).
    pub old: PathBuf,
    /// Download files and owned folders of this attempt.
    pub scratch: Vec<PathBuf>,
    pub phase: Phase,
    /// The identity of the item at `site` when the attempt began.
    pub old_hash: String,
    /// Set by an atomic rewrite right after every successful move of `old` back onto `site`;
    /// from then on whatever is at `old` is not the backup.
    #[serde(default)]
    pub old_restored: bool,
    /// The identity of the checked new item, written before the swap.
    #[serde(default)]
    pub new_hash: Option<String>,
    /// Paths recorded right before an exclusive creation or no-replace rename.
    #[serde(default)]
    pub intended: Vec<PathBuf>,
    /// Paths recorded only after that operation succeeded. Recovery deletes only these.
    #[serde(default)]
    pub created: Vec<PathBuf>,
}

fn journal_path(data_dir: &Path) -> PathBuf {
    data_dir.join(JOURNAL_FILE)
}

/// Writes the journal atomically (`fsio::atomic_write`, owner-only on Unix) and durably: after
/// the rename the journal file and the data folder are synced ([`super::native_sync`]), so the
/// state is on disk before the step it announces runs.
pub fn write_journal(data_dir: &Path, j: &Journal) -> Result<(), UpdateError> {
    write_journal_with(data_dir, j, &super::native_sync)
}

/// [`write_journal`] with an injected barrier. A failed barrier is a failed write: the caller
/// treats the new state as not written (the file on disk holds the old or the new state, like
/// after a crash).
pub(crate) fn write_journal_with(
    data_dir: &Path,
    j: &Journal,
    sync: &dyn Fn(&Path) -> io::Result<()>,
) -> Result<(), UpdateError> {
    let failed = || UpdateError::Install("update journal could not be written");
    let bytes = serde_json::to_vec_pretty(j).map_err(|_| failed())?;
    let path = journal_path(data_dir);
    fsio::atomic_write(&path, &bytes, true).map_err(|_| failed())?;
    sync(&path)
        .and_then(|()| sync(data_dir))
        .map_err(|_| failed())
}

/// `Ok(None)` when there is no journal; `Err` when it exists but cannot be read or parsed (it
/// is then left alone: an unreadable journal must keep blocking new attempts).
pub fn read_journal(data_dir: &Path) -> Result<Option<Journal>, UpdateError> {
    let bytes = fsio::read_optional(&journal_path(data_dir))
        .map_err(|_| UpdateError::Install("update journal could not be read"))?;
    match bytes {
        None => Ok(None),
        Some(bytes) => serde_json::from_slice(fsio::strip_bom(&bytes))
            .map(Some)
            .map_err(|_| UpdateError::Install("update journal could not be read")),
    }
}

pub(crate) fn remove_journal_checked(data_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(journal_path(data_dir)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Removes the journal, ignoring errors (a journal that stays is retried by the next start).
pub fn remove_journal(data_dir: &Path) {
    let _ = remove_journal_checked(data_dir);
}

/// Rewrites the journal as `committed`. Called only after the swap AND every check after it
/// succeeded.
pub fn commit_journal(data_dir: &Path) -> Result<(), UpdateError> {
    let mut j = read_journal(data_dir)?.ok_or(UpdateError::Install("no update journal"))?;
    // Only a finished swap of this attempt can be committed: pending, the new identity recorded
    // and the previous version not moved back.
    if j.phase != Phase::Pending || j.new_hash.is_none() || j.old_restored {
        return Err(UpdateError::Install("update journal cannot be committed"));
    }
    j.phase = Phase::Committed;
    write_journal(data_dir, &j)
}

/// True only when looking the path up fails with `NotFound`; anything else (present, or an
/// error reading the entry) counts as present, so it is never treated as free.
pub(crate) fn is_absent(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Before an exclusive creation of `path`: checks it is absent and records it in `intended`
/// (atomic rewrite). A present path is `Err(Install)` and is NOT recorded.
pub fn intend(data_dir: &Path, j: &mut Journal, path: &Path) -> Result<(), UpdateError> {
    if !is_absent(path) {
        return Err(UpdateError::Install("update path already exists"));
    }
    if j.intended.iter().any(|p| p == path) {
        return Ok(());
    }
    j.intended.push(path.to_owned());
    write_journal(data_dir, j).inspect_err(|_| {
        j.intended.pop();
    })
}

/// After the exclusive creation of `path` SUCCEEDED: records it in `created` (atomic rewrite).
pub fn confirm(data_dir: &Path, j: &mut Journal, path: &Path) -> Result<(), UpdateError> {
    if j.created.iter().any(|p| p == path) {
        return Ok(());
    }
    j.created.push(path.to_owned());
    write_journal(data_dir, j).inspect_err(|_| {
        j.created.pop();
    })
}
