//! Startup recovery and the shared cleanup (spec section 3, "Recovery"). Every step is
//! idempotent: an interrupted recovery is simply run again.

use std::io;
use std::path::{Path, PathBuf};

use super::journal::{is_absent, read_journal, remove_journal_checked, Journal, Phase};
use super::{
    bundle_identity, file_sha256, log, native_rename_noreplace, native_rename_swap, no_hook,
    Crashed, Ctx, HookAction, SiteKind, Step, OWNER_MARKER,
};

/// The outcome of [`recover`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// No journal: nothing was touched.
    Nothing,
    /// The journal was closed; nothing had to be put back.
    Cleaned,
    /// The previous version was put back at `site` in this run, and the journal was closed.
    Restored,
    /// The journal stays (an unknown state, an error, a protected path): the next start retries
    /// and no new attempt may start.
    Kept,
}

/// The operations recovery uses, injectable for tests.
pub struct RecoverOps<'a> {
    /// The identity of the item at a path (see `identity.rs`).
    pub identity: &'a dyn Fn(&Path) -> Option<String>,
    /// The atomic exchange of two paths (macOS `RENAME_SWAP`), used for a bundle swap-back.
    pub swap: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    /// The no-replace rename (fails when the destination exists).
    pub rename_noreplace: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    #[doc(hidden)]
    pub hook: &'a dyn Fn(Step) -> HookAction,
}

impl RecoverOps<'static> {
    /// The native operations for a site kind: Windows = SHA-256 of the exe, macOS = the bundle
    /// identity; the OS no-replace rename and swap.
    pub fn for_kind(kind: SiteKind) -> Self {
        let identity: &'static dyn Fn(&Path) -> Option<String> = match kind {
            SiteKind::WindowsExe => &file_sha256,
            SiteKind::MacBundle => &bundle_identity,
        };
        RecoverOps {
            identity,
            swap: &native_rename_swap,
            rename_noreplace: &native_rename_noreplace,
            hook: &no_hook,
        }
    }
}

/// Recovery with the native operations chosen by the journal's site kind. Run at startup and
/// before every new attempt; `running` is the running exe or a path inside the running bundle.
pub fn recover(data_dir: &Path, running: &Path) -> Recovery {
    match read_journal(data_dir) {
        Ok(None) => Recovery::Nothing,
        Ok(Some(j)) => run_with(data_dir, running, &RecoverOps::for_kind(j.kind), j),
        Err(_) => {
            log("update recovery failed");
            Recovery::Kept
        }
    }
}

/// Recovery with injected operations (tests, and Task 6 reuse).
pub fn recover_with(data_dir: &Path, running: &Path, ops: &RecoverOps<'_>) -> Recovery {
    match read_journal(data_dir) {
        Ok(None) => Recovery::Nothing,
        Ok(Some(j)) => run_with(data_dir, running, ops, j),
        Err(_) => {
            log("update recovery failed");
            Recovery::Kept
        }
    }
}

fn run_with(data_dir: &Path, running: &Path, ops: &RecoverOps<'_>, j: Journal) -> Recovery {
    let ctx = Ctx::new(
        data_dir,
        running,
        ops.identity,
        ops.rename_noreplace,
        ops.hook,
    );
    run(&ctx, ops.swap, j).unwrap_or(Recovery::Kept)
}

fn failed() -> Result<Recovery, Crashed> {
    log("update recovery failed");
    Ok(Recovery::Kept)
}

fn run(
    ctx: &Ctx<'_>,
    swap: &dyn Fn(&Path, &Path) -> io::Result<()>,
    mut j: Journal,
) -> Result<Recovery, Crashed> {
    let mut restored = false;

    // `site` missing (Windows only, between the two renames): put the backup back.
    match std::fs::symlink_metadata(&j.site) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // `oldRestored` takes precedence; without `newHash` no swap can have happened, so
            // a file at `old` cannot be our backup; a bundle `old` is the staged path and never
            // restored by a rename.
            let backup = j.kind == SiteKind::WindowsExe
                && !j.old_restored
                && j.new_hash.is_some()
                && tracked(&j, &j.old)
                && !is_absent(&j.old);
            if !backup || (ctx.rename)(&j.old, &j.site).is_err() {
                return failed();
            }
            ctx.at(Step::AfterRestore)?;
            j.old_restored = true;
            if ctx.write(&j).is_err() {
                return failed();
            }
            restored = true;
        }
        Err(_) => return failed(),
    }

    let mut id = (ctx.identity)(&j.site);
    let is_new = |id: &Option<String>, j: &Journal| id.is_some() && *id == j.new_hash;

    if is_new(&id, &j) {
        if j.phase == Phase::Committed {
            return Ok(finish(cleanup(ctx, &mut j, Scope::All)?, restored));
        }
        // Pending: the swap happened, a later check or its swap-back failed. Put the previous
        // version back, only when the item at the other path exists and is not the new one.
        let other_ok = !j.old_restored
            && tracked(&j, &j.old)
            && !is_absent(&j.old)
            && (ctx.identity)(&j.old) != j.new_hash;
        if !other_ok {
            return failed();
        }
        match j.kind {
            SiteKind::WindowsExe => {
                if (ctx.rename)(&j.site, &j.staged).is_err() {
                    return failed();
                }
                if (ctx.rename)(&j.old, &j.site).is_err() {
                    // `site` is missing now: the next run restores `old` by the rule above.
                    return failed();
                }
            }
            SiteKind::MacBundle => {
                if swap(&j.old, &j.site).is_err() {
                    return failed();
                }
            }
        }
        ctx.at(Step::AfterSwapBack)?;
        j.old_restored = true;
        if ctx.write(&j).is_err() {
            return failed();
        }
        restored = true;
        id = (ctx.identity)(&j.site);
    }

    if j.phase == Phase::Committed {
        // Committed but `site` is not the checked new version: keep `old` and the journal.
        return failed();
    }

    if id.is_some() && id.as_deref() == Some(j.old_hash.as_str()) {
        // The previous version is at `site`: the remaining copies are the new or unchecked
        // download, never installed by recovery.
        return Ok(finish(cleanup(ctx, &mut j, Scope::All)?, restored));
    }

    // Unknown item at `site`.
    log("update recovery failed");
    // While `oldRestored` is false, whatever is at `old` is the backup, whatever its identity.
    // Without `newHash` no rename or swap can have happened (it is written before the first
    // move), so nothing at `old` can be a backup then.
    if !j.old_restored && j.new_hash.is_some() && !is_absent(&j.old) {
        return Ok(Recovery::Kept);
    }
    Ok(finish(cleanup(ctx, &mut j, Scope::FoldersOnly)?, restored))
}

fn finish(closed: bool, restored: bool) -> Recovery {
    match (closed, restored) {
        (false, _) => Recovery::Kept,
        (true, true) => Recovery::Restored,
        (true, false) => Recovery::Cleaned,
    }
}

/// `path` is named by the journal (`intended` or `created`) or lies inside a created folder.
fn tracked(j: &Journal, path: &Path) -> bool {
    j.intended.iter().any(|p| p == path)
        || j.created.iter().any(|p| p == path || path.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Created files, the backup (under the identity rule) and owned folders.
    All,
    /// Only owned folders (an unknown item at `site` with no backup left).
    FoldersOnly,
}

/// True when deleting `path` could delete the running app: `running` equals or lies inside it.
fn protected(ctx: &Ctx<'_>, j: &Journal, path: &Path) -> bool {
    if ctx.running.starts_with(path) || j.site.starts_with(path) {
        return true;
    }
    match (
        std::fs::canonicalize(ctx.running),
        std::fs::canonicalize(path),
    ) {
        (Ok(r), Ok(p)) => r.starts_with(p),
        _ => false,
    }
}

fn marker_matches(folder: &Path, token: &str) -> bool {
    let marker = folder.join(OWNER_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(m) if m.is_file() && m.len() <= 64 => {}
        _ => return false,
    }
    std::fs::read(&marker).is_ok_and(|b| b == token.as_bytes())
}

fn unique(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

fn inside(path: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| path != f && path.starts_with(f))
}

/// Deletes this attempt's own paths, then the journal. Returns `Ok(true)` when the journal
/// was removed, `Ok(false)` when it stays (a protected path, a delete error). Rules:
/// - only `created` paths that exist are deleted; `site` never;
/// - a `created` folder is removed whole only with a matching owner marker, else kept;
/// - the backup `old` (outside an owned folder) only while `oldRestored` is false and its
///   identity equals `oldHash`;
/// - `intended`-only paths outside an owned folder are left in place.
pub(crate) fn cleanup(ctx: &Ctx<'_>, j: &mut Journal, scope: Scope) -> Result<bool, Crashed> {
    let created = unique(&j.created);
    let dirs: Vec<PathBuf> = created
        .iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
        .cloned()
        .collect();
    let (owned, unowned): (Vec<PathBuf>, Vec<PathBuf>) = dirs
        .iter()
        .cloned()
        .partition(|d| marker_matches(d, &j.token));
    for _ in &unowned {
        log("update leftover kept");
    }
    let mut keep = false;

    if scope == Scope::All {
        for p in created.iter().filter(|p| !dirs.contains(p)) {
            if *p == j.site || *p == j.old || is_absent(p) || inside(p, &owned) {
                continue;
            }
            if inside(p, &unowned) {
                log("update leftover kept");
                continue;
            }
            if protected(ctx, j, p) {
                keep = true;
                continue;
            }
            if !std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) {
                // We only create regular files: anything else at that path is not ours.
                log("update leftover kept");
                continue;
            }
            if std::fs::remove_file(p).is_err() {
                log("update recovery failed");
                keep = true;
                continue;
            }
            ctx.at(Step::AfterDelete)?;
        }
        if j.old != j.site && !is_absent(&j.old) && !inside(&j.old, &owned) {
            let is_backup = j.created.contains(&j.old) && !j.old_restored;
            let is_file = std::fs::symlink_metadata(&j.old).is_ok_and(|m| m.is_file());
            if is_backup && protected(ctx, j, &j.old) {
                keep = true;
            } else if is_backup
                && is_file
                && (ctx.identity)(&j.old).as_deref() == Some(j.old_hash.as_str())
            {
                if std::fs::remove_file(&j.old).is_err() {
                    log("update recovery failed");
                    keep = true;
                } else {
                    ctx.at(Step::AfterDelete)?;
                }
            } else {
                log("update leftover kept");
            }
        }
    }

    for f in &owned {
        if protected(ctx, j, f) {
            keep = true;
            continue;
        }
        if std::fs::remove_dir_all(f).is_err() {
            log("update recovery failed");
            keep = true;
            continue;
        }
        ctx.at(Step::AfterFolderRemoved)?;
    }

    for p in unique(&j.intended) {
        if !j.created.contains(&p)
            && p != j.site
            && p != j.old
            && !is_absent(&p)
            && !inside(&p, &owned)
        {
            log("update leftover kept");
        }
    }

    if keep {
        return Ok(false);
    }
    if remove_journal_checked(ctx.data_dir).is_err() {
        log("update recovery failed");
        return Ok(false);
    }
    Ok(true)
}
