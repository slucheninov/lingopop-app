//! The Windows portable exe (spec section 4): install site, staging and the two-rename swap
//! with rollback. Portable code; only the no-replace rename is OS specific (injected).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::journal::{is_absent, read_journal, Journal, Phase, OWNER_MARKER};
use super::recover::{cleanup, Scope};
use super::{
    file_sha256, is_token, log, native_rename_noreplace, new_token, no_hook, Crashed, Ctx,
    HookAction, InstallSite, SiteKind, Step, Support, UpdateError,
};

/// `IMAGE_FILE_MACHINE_AMD64`, the same rule as `scripts/verify-release-assets.py`.
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// The operations the exe install uses, injectable for tests.
pub struct InstallOps<'a> {
    /// The identity of an exe (production: SHA-256).
    pub identity: &'a dyn Fn(&Path) -> Option<String>,
    /// The no-replace rename (fails when the destination exists).
    pub rename_noreplace: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    #[doc(hidden)]
    pub hook: &'a dyn Fn(Step) -> HookAction,
}

impl InstallOps<'static> {
    /// SHA-256 and the OS no-replace rename, no hook.
    pub fn native() -> Self {
        InstallOps {
            identity: &file_sha256,
            rename_noreplace: &native_rename_noreplace,
            hook: &no_hook,
        }
    }
}

/// `<stem>` = the exe's file name without a case-insensitive `.exe`; `None` for any other name.
fn exe_stem(exe: &Path) -> Option<&str> {
    let name = exe.file_name()?.to_str()?;
    let cut = name.len().checked_sub(4)?;
    if !name.is_char_boundary(cut) || !name[cut..].eq_ignore_ascii_case(".exe") || cut == 0 {
        return None;
    }
    Some(&name[..cut])
}

fn exe_dir(exe: &Path) -> Option<&Path> {
    exe.parent().filter(|p| !p.as_os_str().is_empty())
}

/// `<exe dir>/.LingoPop-update-<token>/`: the owned staging folder.
pub fn staging_folder(site: &InstallSite, token: &str) -> Option<PathBuf> {
    exe_stem(&site.path)?;
    Some(exe_dir(&site.path)?.join(format!(".LingoPop-update-{token}")))
}

/// `<exe dir>/.LingoPop-update-<token>/<stem>.exe`: the download destination and staged exe.
pub fn staged_exe_path(site: &InstallSite, token: &str) -> Option<PathBuf> {
    let stem = exe_stem(&site.path)?;
    Some(staging_folder(site, token)?.join(format!("{stem}.exe")))
}

/// `<exe dir>/<stem>.old-<token>.exe`: the backup after the first rename.
pub fn old_exe_path(site: &InstallSite, token: &str) -> Option<PathBuf> {
    let stem = exe_stem(&site.path)?;
    Some(exe_dir(&site.path)?.join(format!("{stem}.old-{token}.exe")))
}

/// Whether the exe at `exe` can update itself: the name ends in `.exe` and its folder is
/// writable (a probe `<stem>.update.probe` can be created exclusively and removed). Nothing
/// else is written.
pub fn exe_support(exe: &Path) -> Support {
    let (Some(stem), Some(dir)) = (exe_stem(exe), exe_dir(exe)) else {
        return Support::Unsupported("not an exe file");
    };
    let probe = dir.join(format!("{stem}.update.probe"));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => drop(file),
        Err(_) => return Support::Unsupported("folder not writable"),
    }
    if fs::remove_file(&probe).is_err() {
        return Support::Unsupported("folder not writable");
    }
    Support::Supported(InstallSite {
        path: exe.to_owned(),
        kind: SiteKind::WindowsExe,
    })
}

/// Begins an attempt with a fresh token: writes the `pending` journal (with `oldHash`, before
/// anything is created), then creates the owned staging folder exclusively (0700 on Unix) with
/// its owner marker. The returned journal's `staged` is the download destination. Refuses to
/// run while a journal is open.
pub fn stage_exe(
    data_dir: &Path,
    site: &InstallSite,
    version: &str,
) -> Result<Journal, UpdateError> {
    let token = new_token()?;
    stage_exe_with(data_dir, site, version, &token, &InstallOps::native())
}

/// [`stage_exe`] with a given token and operations (tests).
#[doc(hidden)]
pub fn stage_exe_with(
    data_dir: &Path,
    site: &InstallSite,
    version: &str,
    token: &str,
    ops: &InstallOps<'_>,
) -> Result<Journal, UpdateError> {
    if site.kind != SiteKind::WindowsExe || !is_token(token) {
        return Err(UpdateError::Install("not a Windows exe site"));
    }
    let (Some(folder), Some(staged), Some(old)) = (
        staging_folder(site, token),
        staged_exe_path(site, token),
        old_exe_path(site, token),
    ) else {
        return Err(UpdateError::Install("not a Windows exe site"));
    };
    match read_journal(data_dir) {
        Ok(None) => {}
        _ => return Err(UpdateError::Install("an update journal is open")),
    }
    let old_hash =
        (ops.identity)(&site.path).ok_or(UpdateError::Install("installed app unreadable"))?;
    let j = Journal {
        version: version.to_owned(),
        kind: SiteKind::WindowsExe,
        token: token.to_owned(),
        site: site.path.clone(),
        staged,
        old,
        scratch: vec![folder.clone()],
        phase: Phase::Pending,
        old_hash,
        old_restored: false,
        new_hash: None,
        intended: Vec::new(),
        created: Vec::new(),
    };
    let ctx = Ctx::new(
        data_dir,
        &site.path,
        ops.identity,
        ops.rename_noreplace,
        ops.hook,
    );
    begin_attempt(&ctx, j, &folder)
}

/// Writes the first `pending` journal, then creates the owned staging folder `folder`
/// exclusively (0700 on Unix) with its owner marker holding `j.token`, recording it in
/// `intended` before and in `created` after. Any failure removes what this attempt created and
/// the journal (nothing has moved). Shared by the exe and the bundle staging.
pub(crate) fn begin_attempt(
    ctx: &Ctx<'_>,
    mut j: Journal,
    folder: &Path,
) -> Result<Journal, UpdateError> {
    let folder = folder.to_owned();
    let token = j.token.clone();
    let token = token.as_str();
    ctx.write(&j)?;

    if !is_absent(&folder) {
        return abort(ctx, &mut j, "staging folder exists");
    }
    j.intended.push(folder.clone());
    if ctx.write(&j).is_err() {
        return abort(ctx, &mut j, "update journal could not be written");
    }
    match create_private_dir(&folder) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            j.intended.retain(|p| *p != folder);
            if ctx.write(&j).is_err() {
                log("update install failed");
                return Err(UpdateError::Install("update journal could not be written"));
            }
            return abort(ctx, &mut j, "staging folder exists");
        }
        Err(_) => return abort(ctx, &mut j, "staging folder could not be created"),
    }
    if write_marker(&folder, token).is_err() {
        // The folder is ours (create_dir succeeded) but not yet owned: remove it only if empty.
        let _ = fs::remove_dir(&folder);
        return abort(ctx, &mut j, "staging folder could not be created");
    }
    j.created.push(folder);
    if ctx.write(&j).is_err() {
        return abort(ctx, &mut j, "update journal could not be written");
    }
    Ok(j)
}

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    // `mode` exists on Unix only, so the builder is mutated only there.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_marker(folder: &Path, token: &str) -> io::Result<()> {
    let marker = folder.join(OWNER_MARKER);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)?;
    let written = file
        .write_all(token.as_bytes())
        .and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(&marker);
    }
    written
}

/// Nothing moved: delete this attempt's created paths and the journal.
pub(crate) fn abort<T>(
    ctx: &Ctx<'_>,
    j: &mut Journal,
    why: &'static str,
) -> Result<T, UpdateError> {
    log("update install failed");
    let _ = cleanup(ctx, j, Scope::All);
    Err(UpdateError::Install(why))
}

/// True when `path` is a PE image whose machine is AMD64.
fn is_amd64_pe(path: &Path) -> bool {
    fn machine(path: &Path) -> io::Result<Option<u16>> {
        let mut file = File::open(path)?;
        let mut header = [0u8; 0x40];
        if file.read_exact(&mut header).is_err() || &header[..2] != b"MZ" {
            return Ok(None);
        }
        let offset = u32::from_le_bytes([header[0x3C], header[0x3D], header[0x3E], header[0x3F]]);
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut pe = [0u8; 6];
        if file.read_exact(&mut pe).is_err() || &pe[..4] != b"PE\0\0" {
            return Ok(None);
        }
        Ok(Some(u16::from_le_bytes([pe[4], pe[5]])))
    }
    fs::metadata(path).is_ok_and(|m| m.is_file())
        && matches!(machine(path), Ok(Some(IMAGE_FILE_MACHINE_AMD64)))
}

/// Installs the verified download `staged` over the running exe. Expects the journal written
/// by [`stage_exe`]. Steps: PE machine AMD64; `newHash` written; `site` hashed again and
/// required to equal `oldHash`; rename `site` -> `old`; rename `staged` -> `site`; if that
/// fails, rename `old` back. After an `Err` where nothing moved or the rollback succeeded, the
/// attempt's files and the journal are gone and `site` is byte-identical; when the rollback
/// fails NOTHING is deleted and the journal plus `old` stay for [`super::recover`].
/// On `Ok` the journal stays `pending`: the caller commits it with
/// [`super::commit_journal`].
pub fn install_exe(data_dir: &Path, site: &InstallSite, staged: &Path) -> Result<(), UpdateError> {
    install_exe_with(data_dir, site, staged, &InstallOps::native())
}

/// [`install_exe`] with injected operations and hooks (tests).
#[doc(hidden)]
pub fn install_exe_with(
    data_dir: &Path,
    site: &InstallSite,
    staged: &Path,
    ops: &InstallOps<'_>,
) -> Result<(), UpdateError> {
    let mut j = match read_journal(data_dir) {
        Ok(Some(j)) => j,
        _ => return Err(UpdateError::Install("no update journal")),
    };
    if j.kind != SiteKind::WindowsExe
        || site.kind != SiteKind::WindowsExe
        || j.site != site.path
        || j.staged != staged
        || j.phase != Phase::Pending
        || j.old_restored
        || old_exe_path(site, &j.token).as_ref() != Some(&j.old)
    {
        return Err(UpdateError::Install("update journal does not match"));
    }
    let ctx = Ctx::new(
        data_dir,
        &site.path,
        ops.identity,
        ops.rename_noreplace,
        ops.hook,
    );
    match swap(&ctx, &mut j) {
        Ok(result) => result,
        Err(Crashed) => Err(UpdateError::Install("update interrupted")),
    }
}

fn swap(ctx: &Ctx<'_>, j: &mut Journal) -> Result<Result<(), UpdateError>, Crashed> {
    let site = j.site.clone();
    let staged = j.staged.clone();
    let old = j.old.clone();

    if !is_amd64_pe(&staged) {
        return Ok(abort(ctx, j, "not an x64 Windows executable"));
    }
    let Some(new_hash) = (ctx.identity)(&staged) else {
        return Ok(abort(ctx, j, "staged update unreadable"));
    };
    j.new_hash = Some(new_hash);
    if ctx.write(j).is_err() {
        return Ok(abort(ctx, j, "update journal could not be written"));
    }
    // Step 0: the running exe must still be the one recorded at the start.
    if (ctx.identity)(&site).as_deref() != Some(j.old_hash.as_str()) {
        return Ok(abort(ctx, j, "installed app changed"));
    }
    ctx.at(Step::BeforeFirstRename)?;

    // Step 1: site -> old (no-replace).
    if !is_absent(&old) {
        return Ok(abort(ctx, j, "backup name taken"));
    }
    j.intended.push(old.clone());
    if ctx.write(j).is_err() {
        return Ok(abort(ctx, j, "update journal could not be written"));
    }
    ctx.at(Step::BeforeBackupRename)?;
    if let Err(e) = (ctx.rename)(&site, &old) {
        ctx.at(Step::AfterCollision)?;
        if e.kind() == io::ErrorKind::AlreadyExists {
            // Someone else's file: it never reaches `created`, and is no longer ours to name.
            j.intended.retain(|p| *p != old);
            if ctx.write(j).is_err() {
                log("update install failed");
                return Ok(Err(UpdateError::Install(
                    "update journal could not be written",
                )));
            }
        }
        return Ok(abort(ctx, j, "backup rename failed"));
    }
    j.created.push(old.clone());
    if ctx.write(j).is_err() {
        return rollback(ctx, j);
    }
    ctx.at(Step::BeforeSecondRename)?;

    // Step 2: staged -> site (no-replace).
    if !is_absent(&site) {
        return rollback(ctx, j);
    }
    if !j.intended.contains(&site) {
        j.intended.push(site.clone());
        if ctx.write(j).is_err() {
            return rollback(ctx, j);
        }
    }
    ctx.at(Step::BeforeStagedRename)?;
    if (ctx.rename)(&staged, &site).is_err() {
        return rollback(ctx, j);
    }
    Ok(Ok(()))
}

/// Step 3: rename `old` back onto `site`. Only a successful rollback continues with the
/// cleanup; a failed one deletes nothing.
fn rollback(ctx: &Ctx<'_>, j: &mut Journal) -> Result<Result<(), UpdateError>, Crashed> {
    log("update install failed");
    ctx.at(Step::BeforeRollback)?;
    if (ctx.rename)(&j.old, &j.site).is_err() {
        log("update rollback failed");
        return Ok(Err(UpdateError::Install("rollback failed")));
    }
    ctx.at(Step::AfterRollbackMove)?;
    j.old_restored = true;
    if ctx.write(j).is_err() {
        // `old` is gone and `oldRestored` false on disk: recovery's ordinary "no backup" case.
        return Ok(Err(UpdateError::Install(
            "update journal could not be written",
        )));
    }
    ctx.at(Step::AfterRollback)?;
    let _ = cleanup(ctx, j, Scope::All)?;
    Ok(Err(UpdateError::Install("staged exe could not be moved")))
}
