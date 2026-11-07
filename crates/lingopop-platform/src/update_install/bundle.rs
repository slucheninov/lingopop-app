//! The macOS app bundle install (spec section 5): the support check against the pinned
//! designated requirement, staging into `<parent>/.LingoPop-update-<version>-<token>/`, the
//! checks before the swap, the atomic `RENAME_SWAP` exchange and the swap back after a failed
//! late check. Portable: every OS tool (codesign, ditto, lipo, the swap) is behind
//! [`BundleTools`]; production is `macos::codesign::SystemBundleTools`.
//!
//! The pinned requirement is what keeps the macOS privacy permissions (TCC) across updates:
//! an update the system would treat as another app is refused before anything moves.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lingopop_core::updates::{stable_version, RELEASE_CERT_SHA1};

use super::exe::{abort, begin_attempt, create_private_dir};
use super::identity::{info_plist, plist_string};
use super::journal::{is_absent, read_journal, Journal, Phase};
use super::recover::{cleanup, Scope};
use super::{
    bundle_identity, is_token, log, native_rename_noreplace, native_sync, new_token, no_hook,
    sync_tree, Crashed, Ctx, HookAction, InstallSite, SiteKind, Step, Support, UpdateError,
};
use crate::macos_login::is_installed_bundle_path;

/// The only bundle id the updater ever replaces (the Swift app's `com.lingopop.app` never).
pub const BUNDLE_ID: &str = "app.lingopop.tauri.macos";

/// The bundle's file name inside the update zip.
pub const BUNDLE_NAME: &str = "LingoPop.app";

/// The download's file name inside the owned staging folder.
pub const BUNDLE_ZIP_FILE: &str = "download.zip";

/// The folder inside the owned staging folder the zip is extracted into.
const EXTRACT_DIR: &str = "extracted";

/// How often the swap back after a failed late check is tried before recovery takes over.
const SWAP_BACK_ATTEMPTS: u32 = 2;

/// The OS tools the bundle install needs. Output of the tools is parsed, never logged.
pub trait BundleTools: Send + Sync {
    /// `codesign -d -r- <app>`: the text after `designated => `; `None` when unsigned or
    /// unreadable.
    fn designated_requirement(&self, app: &Path) -> Option<String>;
    /// `codesign --verify --strict --deep -R=<requirement> <app>` succeeded.
    fn verify(&self, app: &Path, requirement: &str) -> bool;
    /// `ditto -x -k --noqtn <zip> <into>`.
    fn extract_zip(&self, zip: &Path, into: &Path) -> Result<(), UpdateError>;
    /// `(CFBundleIdentifier, CFBundleShortVersionString)` of `Contents/Info.plist`.
    fn bundle_info(&self, app: &Path) -> Option<(String, String)>;
    /// `lipo -archs` of the main executable (`arm64`, `x86_64`, ...); empty when unreadable.
    fn executable_archs(&self, app: &Path) -> Vec<String>;
    /// The atomic exchange of two paths on one volume (`renamex_np(RENAME_SWAP)`).
    fn swap(&self, a: &Path, b: &Path) -> Result<(), UpdateError>;
    /// The durability barrier of one file or folder ([`native_sync`]; tests record it).
    fn sync(&self, path: &Path) -> io::Result<()> {
        native_sync(path)
    }
}

/// The exact designated requirement every updatable bundle (the running one and the update)
/// must have / satisfy.
pub fn pinned_requirement() -> String {
    format!("identifier \"{BUNDLE_ID}\" and certificate leaf = H\"{RELEASE_CERT_SHA1}\"")
}

/// `(CFBundleIdentifier, CFBundleShortVersionString)` from the bundle's XML `Info.plist`
/// (the portable reader the identity uses; a binary plist gives `None`).
pub fn read_bundle_info(app: &Path) -> Option<(String, String)> {
    let plist = info_plist(app)?;
    Some((
        plist_string(&plist, "CFBundleIdentifier")?,
        plist_string(&plist, "CFBundleShortVersionString")?,
    ))
}

/// Whether the running bundle at `app` can update itself. `Unsupported` unless: the path passes
/// `is_installed_bundle_path` (no disk image, no App Translocation), the bundle id is
/// [`BUNDLE_ID`], its own designated requirement is exactly [`pinned_requirement`] (an ad-hoc
/// or otherwise signed build never self-updates) and the parent folder is writable (an
/// exclusive probe file is created and removed; nothing else is written).
pub fn bundle_support(app: &Path, tools: &dyn BundleTools) -> Support {
    let Some(text) = app.to_str() else {
        return Support::Unsupported("not an installed app bundle");
    };
    if !app.is_absolute() || !is_installed_bundle_path(text) {
        return Support::Unsupported("not an installed app bundle");
    }
    let Some(parent) = app.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Support::Unsupported("not an installed app bundle");
    };
    match tools.bundle_info(app) {
        Some((id, _)) if id == BUNDLE_ID => {}
        _ => return Support::Unsupported("not the LingoPop bundle"),
    }
    if tools.designated_requirement(app).as_deref() != Some(pinned_requirement().as_str()) {
        return Support::Unsupported("not signed with the release certificate");
    }
    let Ok(token) = new_token() else {
        return Support::Unsupported("folder not writable");
    };
    let probe = parent.join(format!(".LingoPop-update-probe-{token}"));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => drop(file),
        Err(_) => return Support::Unsupported("folder not writable"),
    }
    if fs::remove_file(&probe).is_err() {
        return Support::Unsupported("folder not writable");
    }
    Support::Supported(InstallSite {
        path: app.to_owned(),
        kind: SiteKind::MacBundle,
    })
}

/// `<parent>/.LingoPop-update-<version>-<token>/`: the owned staging folder. `None` unless the
/// version is a stable `X.Y.Z` and the token 32 lowercase hex.
pub fn bundle_staging_folder(site: &InstallSite, version: &str, token: &str) -> Option<PathBuf> {
    if site.kind != SiteKind::MacBundle
        || !is_token(token)
        || stable_version(version).as_deref() != Some(version)
    {
        return None;
    }
    let parent = site.path.parent().filter(|p| !p.as_os_str().is_empty())?;
    Some(parent.join(format!(".LingoPop-update-{version}-{token}")))
}

fn folder_of(j: &Journal) -> PathBuf {
    let parent = j.site.parent().unwrap_or(Path::new(""));
    parent.join(format!(".LingoPop-update-{}-{}", j.version, j.token))
}

/// Where the shell downloads the update zip: `<staging folder>/download.zip`.
pub fn bundle_zip_path(j: &Journal) -> PathBuf {
    folder_of(j).join(BUNDLE_ZIP_FILE)
}

/// Begins a bundle attempt with a fresh token: writes the `pending` journal (with `oldHash` =
/// the identity of the running bundle) before anything is created, then creates the owned
/// staging folder exclusively (0700) with its owner marker. `staged` = `old` =
/// `<staging>/extracted/LingoPop.app`. The download goes to [`bundle_zip_path`]. Refuses to run
/// while a journal is open.
pub fn stage_bundle(
    data_dir: &Path,
    site: &InstallSite,
    version: &str,
) -> Result<Journal, UpdateError> {
    let token = new_token()?;
    stage_bundle_with(data_dir, site, version, &token, &no_hook)
}

/// [`stage_bundle`] with a given token and hook (tests).
#[doc(hidden)]
pub fn stage_bundle_with(
    data_dir: &Path,
    site: &InstallSite,
    version: &str,
    token: &str,
    hook: &dyn Fn(Step) -> HookAction,
) -> Result<Journal, UpdateError> {
    let Some(folder) = bundle_staging_folder(site, version, token) else {
        return Err(UpdateError::Install("not a macOS bundle site"));
    };
    match read_journal(data_dir) {
        Ok(None) => {}
        _ => return Err(UpdateError::Install("an update journal is open")),
    }
    let old_hash =
        bundle_identity(&site.path).ok_or(UpdateError::Install("installed app unreadable"))?;
    let staged = folder.join(EXTRACT_DIR).join(BUNDLE_NAME);
    let j = Journal {
        version: version.to_owned(),
        kind: SiteKind::MacBundle,
        token: token.to_owned(),
        site: site.path.clone(),
        old: staged.clone(),
        staged,
        scratch: vec![folder.clone()],
        phase: Phase::Pending,
        old_hash,
        old_restored: false,
        new_hash: None,
        intended: Vec::new(),
        created: Vec::new(),
    };
    let identity = bundle_identity;
    let rename = native_rename_noreplace;
    let ctx = Ctx::new(data_dir, &site.path, &identity, &rename, hook);
    begin_attempt(&ctx, j, &folder)
}

/// Installs the downloaded, signature-verified update zip over the running bundle at
/// `site.path` (wherever it is; `/Applications` is never assumed). Expects the journal written
/// by [`stage_bundle`] for this `version`, and `zip` inside its staging folder.
///
/// Before the swap: extract with `ditto -x -k --noqtn`; exactly one `LingoPop.app`; its id is
/// [`BUNDLE_ID`] and its version `version`; it satisfies [`pinned_requirement`] under
/// `codesign --verify --strict --deep`; its executable contains `arch`; `newHash` is written;
/// the bundle at `site` still has `oldHash` and reports version `current` and id
/// [`BUNDLE_ID`]. Then `tools.swap(staged, site)`: the previous bundle now sits at the staged
/// path (= `old`) inside the owned folder, kept until the next start. A failed late check
/// (the installed identity, id, version and requirement again) swaps back.
///
/// After an `Err` where nothing moved or the swap back succeeded, the staging folder and the
/// journal are gone and `site` is unchanged; when the swap back fails NOTHING is deleted and
/// the `pending` journal stays for [`super::recover`]. On `Ok` the journal stays `pending`:
/// the caller commits it with [`super::commit_journal`].
pub fn install_bundle(
    data_dir: &Path,
    site: &InstallSite,
    zip: &Path,
    version: &str,
    current: &str,
    arch: &str,
    tools: &dyn BundleTools,
) -> Result<(), UpdateError> {
    install_bundle_with(data_dir, site, zip, version, current, arch, tools, &no_hook)
}

/// [`install_bundle`] with a hook (tests).
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn install_bundle_with(
    data_dir: &Path,
    site: &InstallSite,
    zip: &Path,
    version: &str,
    current: &str,
    arch: &str,
    tools: &dyn BundleTools,
    hook: &dyn Fn(Step) -> HookAction,
) -> Result<(), UpdateError> {
    let mut j = match read_journal(data_dir) {
        Ok(Some(j)) => j,
        _ => return Err(UpdateError::Install("no update journal")),
    };
    let folder = bundle_staging_folder(site, version, &j.token);
    let matches = folder.as_ref().is_some_and(|folder| {
        j.kind == SiteKind::MacBundle
            && j.site == site.path
            && j.version == version
            && j.phase == Phase::Pending
            && !j.old_restored
            && j.new_hash.is_none()
            && j.created.contains(folder)
            && j.staged == folder.join(EXTRACT_DIR).join(BUNDLE_NAME)
            && j.old == j.staged
            && zip.parent() == Some(folder.as_path())
    });
    if !matches {
        return Err(UpdateError::Install("update journal does not match"));
    }
    let identity = bundle_identity;
    let rename = native_rename_noreplace;
    let sync = |p: &Path| tools.sync(p);
    let ctx = Ctx::new(data_dir, &site.path, &identity, &rename, hook).with_sync(&sync);
    let check = Check {
        zip,
        version,
        current,
        arch,
        tools,
    };
    match install(&ctx, &mut j, &check) {
        Ok(result) => result,
        Err(Crashed) => Err(UpdateError::Install("update interrupted")),
    }
}

struct Check<'a> {
    zip: &'a Path,
    version: &'a str,
    current: &'a str,
    arch: &'a str,
    tools: &'a dyn BundleTools,
}

fn is_info(info: Option<(String, String)>, version: &str) -> bool {
    info.is_some_and(|(id, v)| id == BUNDLE_ID && v == version)
}

/// The extract folder holds exactly one entry, `LingoPop.app`, and it is a real folder.
fn exactly_one_app(extracted: &Path) -> bool {
    let Ok(entries) = fs::read_dir(extracted) else {
        return false;
    };
    let mut names = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => names.push(e.file_name()),
            Err(_) => return false,
        }
    }
    names.len() == 1
        && names[0] == BUNDLE_NAME
        && fs::symlink_metadata(extracted.join(BUNDLE_NAME)).is_ok_and(|m| m.is_dir())
}

fn install(
    ctx: &Ctx<'_>,
    j: &mut Journal,
    c: &Check<'_>,
) -> Result<Result<(), UpdateError>, Crashed> {
    let site = j.site.clone();
    let staged = j.staged.clone();
    let Some(extracted) = staged.parent().map(Path::to_owned) else {
        return Ok(abort(ctx, j, "update journal does not match"));
    };

    // Extract inside the owned folder (a collision there is an install error; the folder is
    // removed whole by the cleanup).
    if !is_absent(&extracted) || create_private_dir(&extracted).is_err() {
        return Ok(abort(ctx, j, "update could not be extracted"));
    }
    if c.tools.extract_zip(c.zip, &extracted).is_err() {
        return Ok(abort(ctx, j, "update could not be extracted"));
    }
    if !exactly_one_app(&extracted) {
        return Ok(abort(ctx, j, "update must hold exactly one LingoPop.app"));
    }
    // Barrier before anything moves: the extracted bundle (files, then folders) is on disk.
    if sync_tree(ctx.sync, &extracted).is_err() {
        return Ok(abort(ctx, j, "update could not be made durable"));
    }
    if !is_info(c.tools.bundle_info(&staged), c.version) {
        return Ok(abort(ctx, j, "update bundle id or version mismatch"));
    }
    if !c.tools.verify(&staged, &pinned_requirement()) {
        return Ok(abort(ctx, j, "update signature requirement mismatch"));
    }
    if !c
        .tools
        .executable_archs(&staged)
        .iter()
        .any(|a| a == c.arch)
    {
        return Ok(abort(ctx, j, "update lacks this architecture"));
    }
    let Some(new_hash) = (ctx.identity)(&staged) else {
        return Ok(abort(ctx, j, "staged update unreadable"));
    };
    if new_hash == j.old_hash {
        return Ok(abort(ctx, j, "update is the installed version"));
    }
    j.new_hash = Some(new_hash.clone());
    if ctx.write(j).is_err() {
        return Ok(abort(ctx, j, "update journal could not be written"));
    }
    // The installed bundle must still be the one recorded at the start, at the version
    // `update_plan` was given.
    if (ctx.identity)(&site).as_deref() != Some(j.old_hash.as_str())
        || !is_info(c.tools.bundle_info(&site), c.current)
    {
        return Ok(abort(ctx, j, "installed app changed"));
    }
    ctx.at(Step::BeforeBundleSwap)?;

    if c.tools.swap(&staged, &site).is_err() {
        // The exchange is atomic: a failure moved nothing. Clean up only when `site` provably
        // still holds the previous version; otherwise leave everything for recovery.
        if (ctx.identity)(&site).as_deref() == Some(j.old_hash.as_str()) {
            return Ok(abort(ctx, j, "bundle swap failed"));
        }
        log("update install failed");
        return Ok(Err(UpdateError::Install("bundle swap failed")));
    }
    // The caller commits only after install returns: the swap is durable first. Without the
    // barrier nothing else happens here (no swap back, no cleanup): the journal stays pending
    // and the next start, the new version at `site`, swaps the previous version back.
    if sync_swap(ctx, j).is_err() {
        log("update install failed");
        return Ok(Err(UpdateError::Install(
            "update could not be made durable",
        )));
    }
    ctx.at(Step::AfterBundleSwap)?;

    // Late check: the installed item is the checked bundle and still satisfies the pinned
    // requirement at its final path.
    let late_ok = (ctx.identity)(&site).as_deref() == Some(new_hash.as_str())
        && is_info(c.tools.bundle_info(&site), c.version)
        && c.tools.verify(&site, &pinned_requirement());
    if !late_ok {
        return swap_back(ctx, j, c.tools);
    }
    Ok(Ok(()))
}

/// After a completed swap: both parent folders of the exchange are synced.
fn sync_swap(ctx: &Ctx<'_>, j: &Journal) -> io::Result<()> {
    let parents: Vec<&Path> = [j.site.parent(), j.staged.parent()]
        .into_iter()
        .flatten()
        .collect();
    ctx.barrier(&parents)
}

/// The rollback: the same swap again. Only a successful swap back continues with the cleanup;
/// a failed one deletes nothing and leaves the `pending` journal for recovery.
fn swap_back(
    ctx: &Ctx<'_>,
    j: &mut Journal,
    tools: &dyn BundleTools,
) -> Result<Result<(), UpdateError>, Crashed> {
    log("update install failed");
    ctx.at(Step::BeforeRollback)?;
    let swapped = (0..SWAP_BACK_ATTEMPTS).any(|_| tools.swap(&j.staged, &j.site).is_ok());
    if !swapped {
        log("update rollback failed");
        return Ok(Err(UpdateError::Install("rollback failed")));
    }
    if sync_swap(ctx, j).is_err() {
        // `oldRestored` is not written over a swap that may not be on disk; nothing is
        // cleaned: the next start decides from what reached `site`.
        log("update rollback failed");
        return Ok(Err(UpdateError::Install(
            "rollback could not be made durable",
        )));
    }
    ctx.at(Step::AfterSwapBack)?;
    j.old_restored = true;
    if ctx.write(j).is_err() {
        // The previous version is back at `site`; recovery finds `oldHash` there and cleans.
        return Ok(Err(UpdateError::Install(
            "update journal could not be written",
        )));
    }
    ctx.at(Step::AfterRollback)?;
    let _ = cleanup(ctx, j, Scope::All)?;
    Ok(Err(UpdateError::Install(
        "update check failed after the swap",
    )))
}
