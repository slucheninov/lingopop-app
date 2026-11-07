//! The macOS bundle install (spec section 5) over a fake `BundleTools`: temp dirs only, every
//! OS. The fake extracts scripted fake bundles, answers the codesign / lipo questions from its
//! script and swaps with a portable (non-atomic) stand-in for `RENAME_SWAP`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lingopop_core::update_sig::UpdateError;
use lingopop_core::updates::RELEASE_CERT_SHA1;
use lingopop_platform::update_install::{
    bundle_identity, bundle_staging_folder, bundle_support, bundle_zip_path, commit_journal,
    install_bundle, install_bundle_with, pinned_requirement, read_bundle_info, read_journal,
    recover, recover_after_install_with, recover_with, stage_bundle, stage_bundle_with,
    BundleTools, HookAction, InstallSite, Journal, Phase, RecoverOps, Recovery, RunningCopies,
    SiteKind, Step, Support, BUNDLE_ID, JOURNAL_FILE, OWNER_MARKER,
};
use tempfile::TempDir;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const CURRENT: &str = "0.2.1";
const VERSION: &str = "0.2.2";
const ARCH: &str = "arm64";

fn info_plist(id: &str, version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\
         \t<key>CFBundleExecutable</key>\n\t<string>LingoPop</string>\n\
         \t<key>CFBundleIdentifier</key>\n\t<string>{id}</string>\n\
         \t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n\
         </dict>\n</plist>\n"
    )
}

fn fake_bundle(app: &Path, id: &str, version: &str, exe: &[u8]) {
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::create_dir_all(contents.join("_CodeSignature")).unwrap();
    fs::write(contents.join("Info.plist"), info_plist(id, version)).unwrap();
    fs::write(
        contents.join("_CodeSignature/CodeResources"),
        format!("resources {version}"),
    )
    .unwrap();
    fs::write(contents.join("MacOS/LingoPop"), exe).unwrap();
}

fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_owned();
            if p.is_dir() {
                out.push((rel, Vec::new()));
                walk(root, &p, out);
            } else {
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// A portable stand-in for `renamex_np(RENAME_SWAP)` (not atomic; fine on temp dirs).
fn test_swap(a: &Path, b: &Path) -> io::Result<()> {
    let tmp = a.with_extension("swap-tmp");
    fs::rename(a, &tmp)?;
    fs::rename(b, a)?;
    fs::rename(&tmp, b)
}

/// One entry the fake "zip" extracts: `(file name, bundle id, version)`.
type Payload = (&'static str, &'static str, &'static str);

struct Fake {
    requirement: Option<String>,
    payload: Vec<Payload>,
    archs: Vec<String>,
    /// Paths `verify` rejects (on top of any requirement that is not the pinned one).
    reject_verify: Mutex<Vec<PathBuf>>,
    verify_calls: Mutex<Vec<(PathBuf, String)>>,
    /// 1-based numbers of the swap calls that fail.
    fail_swaps: Vec<u32>,
    swaps: Mutex<Vec<(PathBuf, PathBuf)>>,
    extracts: Mutex<u32>,
    /// Every sync and swap in order.
    events: Mutex<Vec<Ev>>,
    /// A sync of any path at or under this one fails.
    fail_sync_under: Option<PathBuf>,
    /// A sync of exactly this path fails.
    fail_sync_at: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Sync(PathBuf),
    Swap(PathBuf, PathBuf),
}

impl Fake {
    fn new() -> Self {
        Fake {
            requirement: Some(pinned_requirement()),
            payload: vec![("LingoPop.app", BUNDLE_ID, VERSION)],
            archs: vec!["x86_64".into(), "arm64".into()],
            reject_verify: Mutex::new(Vec::new()),
            verify_calls: Mutex::new(Vec::new()),
            fail_swaps: Vec::new(),
            swaps: Mutex::new(Vec::new()),
            extracts: Mutex::new(0),
            events: Mutex::new(Vec::new()),
            fail_sync_under: None,
            fail_sync_at: None,
        }
    }

    fn events(&self) -> Vec<Ev> {
        self.events.lock().unwrap().clone()
    }

    fn swap_count(&self) -> usize {
        self.swaps.lock().unwrap().len()
    }
}

impl BundleTools for Fake {
    fn designated_requirement(&self, _app: &Path) -> Option<String> {
        self.requirement.clone()
    }

    fn verify(&self, app: &Path, requirement: &str) -> bool {
        self.verify_calls
            .lock()
            .unwrap()
            .push((app.to_owned(), requirement.to_owned()));
        requirement == pinned_requirement()
            && !self.reject_verify.lock().unwrap().iter().any(|p| p == app)
    }

    fn extract_zip(&self, zip: &Path, into: &Path) -> Result<(), UpdateError> {
        *self.extracts.lock().unwrap() += 1;
        assert!(zip.is_file(), "the zip exists");
        for (name, id, version) in &self.payload {
            fake_bundle(&into.join(name), id, version, b"new executable");
        }
        Ok(())
    }

    fn bundle_info(&self, app: &Path) -> Option<(String, String)> {
        read_bundle_info(app)
    }

    fn executable_archs(&self, _app: &Path) -> Vec<String> {
        self.archs.clone()
    }

    fn swap(&self, a: &Path, b: &Path) -> Result<(), UpdateError> {
        self.events
            .lock()
            .unwrap()
            .push(Ev::Swap(a.to_owned(), b.to_owned()));
        let mut swaps = self.swaps.lock().unwrap();
        swaps.push((a.to_owned(), b.to_owned()));
        let n = swaps.len() as u32;
        if self.fail_swaps.contains(&n) {
            return Err(UpdateError::Install("swap failed"));
        }
        test_swap(a, b).map_err(|_| UpdateError::Install("swap failed"))
    }

    fn sync(&self, path: &Path) -> io::Result<()> {
        self.events.lock().unwrap().push(Ev::Sync(path.to_owned()));
        if self
            .fail_sync_under
            .as_ref()
            .is_some_and(|f| path.starts_with(f))
            || self.fail_sync_at.as_deref() == Some(path)
        {
            return Err(io::Error::other("scripted sync failure"));
        }
        lingopop_platform::update_install::native_sync(path)
    }
}

struct Env {
    tmp: TempDir,
    data: PathBuf,
    parent: PathBuf,
    site: InstallSite,
}

impl Env {
    fn new() -> Self {
        Self::at("Applications", CURRENT)
    }

    fn at(parent: &str, installed: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let parent = tmp.path().join(parent);
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join("LingoPop.app");
        fake_bundle(&path, BUNDLE_ID, installed, b"old executable");
        Env {
            tmp,
            data,
            parent,
            site: InstallSite {
                path,
                kind: SiteKind::MacBundle,
            },
        }
    }

    fn folder(&self) -> PathBuf {
        self.parent
            .join(format!(".LingoPop-update-{VERSION}-{TOKEN}"))
    }

    fn journal(&self) -> Option<Journal> {
        read_journal(&self.data).unwrap()
    }

    /// Stage with the fixed token and put a fake download at the zip path.
    fn stage(&self) -> Journal {
        let j = stage_bundle_with(
            &self.data,
            &self.site,
            VERSION,
            TOKEN,
            &lingopop_platform::update_install::no_hook,
        )
        .expect("stage");
        fs::write(bundle_zip_path(&j), b"zip bytes").unwrap();
        j
    }

    fn install(&self, tools: &Fake) -> Result<(), UpdateError> {
        let j = self.journal().unwrap();
        install_bundle(
            &self.data,
            &self.site,
            &bundle_zip_path(&j),
            VERSION,
            CURRENT,
            ARCH,
            tools,
        )
    }

    fn install_with(&self, tools: &dyn BundleTools) -> Result<(), UpdateError> {
        let j = self.journal().unwrap();
        install_bundle(
            &self.data,
            &self.site,
            &bundle_zip_path(&j),
            VERSION,
            CURRENT,
            ARCH,
            tools,
        )
    }

    fn site_version(&self) -> Option<String> {
        read_bundle_info(&self.site.path).map(|(_, v)| v)
    }

    fn running(&self) -> PathBuf {
        self.site.path.join("Contents/MacOS/LingoPop")
    }

    /// Recovery as a new start runs it: the running executable inside `site`, a fresh process
    /// memory of relocated running copies.
    fn recover_with_test_swap(&self) -> Recovery {
        self.recover_in(&RunningCopies::new(), &self.running())
    }

    /// Recovery inside the process that owns `copies`, running `running`.
    fn recover_in(&self, copies: &RunningCopies, running: &Path) -> Recovery {
        let swap = |a: &Path, b: &Path| test_swap(a, b);
        let ops = RecoverOps {
            swap: &swap,
            running_copies: copies,
            ..RecoverOps::for_kind(SiteKind::MacBundle)
        };
        recover_with(&self.data, running, &ops)
    }

    /// The previous process (its image outside `site`) runs recovery.
    fn recover_from_elsewhere(&self) -> Recovery {
        let elsewhere = self.tmp.path().join("elsewhere/LingoPop");
        self.recover_in(&RunningCopies::new(), &elsewhere)
    }
}

fn assert_refused_untouched(
    env: &Env,
    before: &[(PathBuf, Vec<u8>)],
    result: Result<(), UpdateError>,
) {
    assert!(matches!(result, Err(UpdateError::Install(_))), "{result:?}");
    assert_eq!(
        tree(&env.site.path),
        before,
        "the installed app is untouched"
    );
    assert!(!env.folder().exists(), "the attempt's own folder is gone");
    assert!(
        !env.data.join(JOURNAL_FILE).exists(),
        "the journal is closed"
    );
}

// ---------------------------------------------------------------- requirement and support

#[test]
fn pinned_requirement_is_exact() {
    assert_eq!(
        pinned_requirement(),
        format!(
            "identifier \"app.lingopop.tauri.macos\" and certificate leaf = H\"{RELEASE_CERT_SHA1}\""
        )
    );
}

#[test]
fn adhoc_running_app_is_unsupported() {
    let env = Env::new();
    let tools = Fake {
        requirement: Some("cdhash H\"a2060af7ad6b115902aad8cbfeb1e95b643bc514\"".into()),
        ..Fake::new()
    };
    assert!(matches!(
        bundle_support(&env.site.path, &tools),
        Support::Unsupported(_)
    ));
    let unsigned = Fake {
        requirement: None,
        ..Fake::new()
    };
    assert!(matches!(
        bundle_support(&env.site.path, &unsigned),
        Support::Unsupported(_)
    ));
}

#[test]
fn pinned_running_app_is_supported() {
    let env = Env::new();
    match bundle_support(&env.site.path, &Fake::new()) {
        Support::Supported(site) => assert_eq!(site, env.site),
        Support::Unsupported(r) => panic!("unsupported: {r}"),
    }
    // the writability probe leaves nothing behind
    let names: Vec<_> = fs::read_dir(&env.parent)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("LingoPop.app")]);
}

#[test]
fn disk_image_and_translocated_paths_are_unsupported() {
    let tools = Fake::new();
    for p in [
        "/Volumes/LingoPop/LingoPop.app",
        "/private/var/folders/x/AppTranslocation/y/d/LingoPop.app",
        "/Applications/LingoPop",
    ] {
        assert!(
            matches!(
                bundle_support(Path::new(p), &tools),
                Support::Unsupported(_)
            ),
            "{p}"
        );
    }
}

#[test]
fn foreign_bundle_id_at_the_running_path_is_unsupported() {
    let env = Env::new();
    fake_bundle(&env.site.path, "com.lingopop.app", CURRENT, b"swift app");
    assert!(matches!(
        bundle_support(&env.site.path, &Fake::new()),
        Support::Unsupported(_)
    ));
}

#[cfg(unix)]
#[test]
fn read_only_parent_is_unsupported() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    fs::set_permissions(&env.parent, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = env.parent.join("root-check");
    let is_root = fs::write(&probe, b"x").is_ok();
    let _ = fs::remove_file(&probe);
    let result = bundle_support(&env.site.path, &Fake::new());
    fs::set_permissions(&env.parent, fs::Permissions::from_mode(0o755)).unwrap();
    if is_root {
        eprintln!("skipped: running as root");
        return;
    }
    assert!(matches!(result, Support::Unsupported(_)));
}

// ---------------------------------------------------------------- staging

#[test]
fn stage_writes_the_journal_and_the_owned_folder() {
    let env = Env::new();
    let j = env.stage();
    let folder = env.folder();
    assert_eq!(
        bundle_staging_folder(&env.site, VERSION, TOKEN),
        Some(folder.clone())
    );
    assert_eq!(j.kind, SiteKind::MacBundle);
    assert_eq!(j.site, env.site.path);
    assert!(j.staged.starts_with(&folder));
    assert_eq!(j.staged.file_name().unwrap(), "LingoPop.app");
    assert_eq!(
        j.old, j.staged,
        "on macOS the old bundle ends up at the staged path"
    );
    assert_eq!(j.scratch, vec![folder.clone()]);
    assert_eq!(j.created, vec![folder.clone()]);
    assert_eq!(j.phase, Phase::Pending);
    assert_eq!(j.new_hash, None);
    assert_eq!(Some(j.old_hash.clone()), bundle_identity(&env.site.path));
    assert_eq!(
        fs::read(folder.join(OWNER_MARKER)).unwrap(),
        TOKEN.as_bytes()
    );
    assert!(bundle_zip_path(&j).starts_with(&folder));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
    assert_eq!(env.journal().unwrap(), j);
}

#[test]
fn stage_refuses_an_open_journal_and_a_windows_site() {
    let env = Env::new();
    let first = env.stage();
    let other = "fedcba9876543210fedcba9876543210";
    assert!(stage_bundle_with(
        &env.data,
        &env.site,
        VERSION,
        other,
        &lingopop_platform::update_install::no_hook
    )
    .is_err());
    assert_eq!(
        env.journal().unwrap(),
        first,
        "the open journal is never overwritten"
    );
    let exe = InstallSite {
        path: env.parent.join("LingoPop.exe"),
        kind: SiteKind::WindowsExe,
    };
    let env2 = Env::new();
    assert!(stage_bundle(&env2.data, &exe, VERSION).is_err());
    assert!(env2.journal().is_none());
}

#[test]
fn stage_refuses_a_version_that_is_not_a_plain_name() {
    let env = Env::new();
    for v in ["../x", "0.2/2", "", "0.2.2 "] {
        assert!(stage_bundle_with(
            &env.data,
            &env.site,
            v,
            TOKEN,
            &lingopop_platform::update_install::no_hook
        )
        .is_err());
    }
    assert!(env.journal().is_none());
}

// ---------------------------------------------------------------- checks before the swap

#[test]
fn wrong_bundle_id_is_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        payload: vec![("LingoPop.app", "com.lingopop.app", VERSION)],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn wrong_version_is_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        payload: vec![("LingoPop.app", BUNDLE_ID, "0.2.3")],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn requirement_mismatch_is_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    let j = env.stage();
    let tools = Fake::new();
    tools.reject_verify.lock().unwrap().push(j.staged.clone());
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
    // the staged bundle was checked against exactly the pinned requirement
    let calls = tools.verify_calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        &[(j.staged.clone(), pinned_requirement())]
    );
}

#[test]
fn two_apps_in_zip_are_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        payload: vec![
            ("LingoPop.app", BUNDLE_ID, VERSION),
            ("Other.app", BUNDLE_ID, VERSION),
        ],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn zip_without_the_app_is_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        payload: vec![("Other.app", BUNDLE_ID, VERSION)],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
}

#[test]
fn missing_architecture_is_refused() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        archs: vec!["x86_64".into(), "arm64e".into()],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn installed_version_mismatch_aborts_before_swap() {
    let env = Env::at("Applications", "0.2.3");
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0, "no swap call");
    assert_eq!(env.site_version().as_deref(), Some("0.2.3"));
}

#[test]
fn installed_app_changed_since_staging_aborts_before_swap() {
    let env = Env::new();
    env.stage();
    // same version, other executable: the identity no longer equals oldHash
    fake_bundle(&env.site.path, BUNDLE_ID, CURRENT, b"replaced by hand");
    let before = tree(&env.site.path);
    let tools = Fake::new();
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn install_without_a_matching_journal_writes_nothing() {
    let env = Env::new();
    let before = tree(&env.site.path);
    let zip = env.tmp.path().join("download.zip");
    fs::write(&zip, b"zip").unwrap();
    let tools = Fake::new();
    let r = install_bundle(&env.data, &env.site, &zip, VERSION, CURRENT, ARCH, &tools);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(*tools.extracts.lock().unwrap(), 0);
    assert_eq!(tree(&env.site.path), before);
    assert!(env.journal().is_none());
}

// ---------------------------------------------------------------- the swap

#[test]
fn swap_keeps_old_app_until_cleanup() {
    let env = Env::new();
    let j = env.stage();
    let old_hash = j.old_hash.clone();
    let tools = Fake::new();
    env.install(&tools).expect("install");
    assert_eq!(tools.swap_count(), 1);
    assert_eq!(
        tools.swaps.lock().unwrap()[0],
        (j.staged.clone(), env.site.path.clone())
    );
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    // the old bundle sits at the staged path inside the owned folder
    assert_eq!(
        bundle_identity(&j.staged).as_deref(),
        Some(old_hash.as_str())
    );
    let after = env.journal().unwrap();
    assert_eq!(after.phase, Phase::Pending, "the caller commits");
    assert_eq!(after.new_hash, bundle_identity(&env.site.path));
    assert!(!after.old_restored);
    // the late check ran against the installed path with the pinned requirement
    assert!(tools
        .verify_calls
        .lock()
        .unwrap()
        .contains(&(env.site.path.clone(), pinned_requirement())));

    commit_journal(&env.data).unwrap();
    // the next start (the new version, running from site) removes the old bundle
    assert_eq!(recover(&env.data, &env.running()), Recovery::Cleaned);
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
}

#[test]
fn failed_swap_leaves_app_untouched() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        fail_swaps: vec![1],
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 1);
}

#[test]
fn bundle_outside_applications_swaps_at_its_own_path() {
    let env = Env::at("Users/me/Apps", CURRENT);
    let unrelated = env.tmp.path().join("x/Applications/LingoPop.app");
    fake_bundle(&unrelated, BUNDLE_ID, CURRENT, b"unrelated");
    let unrelated_before = tree(&unrelated);
    let j = env.stage();
    assert!(j.staged.starts_with(&env.parent));
    let tools = Fake::new();
    env.install(&tools).expect("install");
    assert_eq!(tools.swaps.lock().unwrap()[0].1, env.site.path);
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    assert_eq!(tree(&unrelated), unrelated_before);
    commit_journal(&env.data).unwrap();
    assert_eq!(recover(&env.data, &env.running()), Recovery::Cleaned);
    assert_eq!(tree(&unrelated), unrelated_before);
}

#[test]
fn failed_late_check_swaps_back() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    tools
        .reject_verify
        .lock()
        .unwrap()
        .push(env.site.path.clone());
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 2, "the swap and the swap back");
}

#[test]
fn failed_late_check_and_failed_swap_back_is_restored_by_recover() {
    let env = Env::new();
    let before = tree(&env.site.path);
    let j = env.stage();
    let tools = Fake {
        fail_swaps: vec![2, 3],
        ..Fake::new()
    };
    tools
        .reject_verify
        .lock()
        .unwrap()
        .push(env.site.path.clone());
    let r = env.install(&tools);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(tools.swap_count(), 3, "the swap and two swap-back attempts");
    // nothing deleted: the new bundle at site, the old one at the staged path, journal pending
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    assert_eq!(
        bundle_identity(&j.staged).as_deref(),
        Some(j.old_hash.as_str())
    );
    let open = env.journal().unwrap();
    assert_eq!(open.phase, Phase::Pending);
    assert!(open.new_hash.is_some());
    assert!(!open.old_restored);

    // the same process (its image now at the staged path) runs recovery
    assert_eq!(env.recover_from_elsewhere(), Recovery::Restored);
    assert_eq!(tree(&env.site.path), before, "the previous version is back");
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
}

// ---------------------------------------------------------------- crashes and recovery

fn crash_at(at: Step) -> impl Fn(Step) -> HookAction {
    move |s| {
        if s == at {
            HookAction::Crash
        } else {
            HookAction::Continue
        }
    }
}

fn install_with_hook(
    env: &Env,
    tools: &Fake,
    hook: &dyn Fn(Step) -> HookAction,
) -> Result<(), UpdateError> {
    let j = env.journal().unwrap();
    install_bundle_with(
        &env.data,
        &env.site,
        &bundle_zip_path(&j),
        VERSION,
        CURRENT,
        ARCH,
        tools,
        hook,
    )
}

#[test]
fn crash_after_staging_is_cleaned_by_recover() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert_eq!(tree(&env.site.path), before);
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
}

#[test]
fn crash_before_swap_never_installs_the_checked_bundle() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    let hook = crash_at(Step::BeforeBundleSwap);
    assert!(install_with_hook(&env, &tools, &hook).is_err());
    let open = env.journal().unwrap();
    assert!(
        open.new_hash.is_some(),
        "newHash is written before the swap"
    );
    assert_eq!(tools.swap_count(), 0);
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert_eq!(tree(&env.site.path), before);
    assert!(!env.folder().exists());
}

#[test]
fn crash_after_swap_is_restored_by_recover() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    let hook = crash_at(Step::AfterBundleSwap);
    assert!(install_with_hook(&env, &tools, &hook).is_err());
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    // the next start is the NEW binary at site: restored, its own bundle kept
    assert_eq!(env.recover_with_test_swap(), Recovery::RestoredKept);
    assert_eq!(tree(&env.site.path), before);
    assert!(env.folder().exists());
    // the start after that (the previous version) closes it
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
}

#[test]
fn crash_after_swap_back_before_the_flag_is_closed_by_recover() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    tools
        .reject_verify
        .lock()
        .unwrap()
        .push(env.site.path.clone());
    let hook = crash_at(Step::AfterSwapBack);
    assert!(install_with_hook(&env, &tools, &hook).is_err());
    assert_eq!(tree(&env.site.path), before, "already swapped back");
    assert!(!env.journal().unwrap().old_restored);
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert_eq!(tree(&env.site.path), before);
    assert!(!env.folder().exists());
}

#[test]
fn an_uncommitted_install_is_swapped_back_at_the_next_start() {
    // install succeeded, but the app quit before the caller committed
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    env.install(&Fake::new()).unwrap();
    assert_eq!(env.recover_with_test_swap(), Recovery::RestoredKept);
    assert_eq!(tree(&env.site.path), before);
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(!env.folder().exists());
}

#[test]
fn new_binary_rolling_back_keeps_its_relocated_bundle() {
    // The previous process died after the swap and before the commit. The user starts the app
    // at its install path: the NEW binary, `running = site/Contents/MacOS/LingoPop`. Recovery
    // swaps the previous version back, which moves the running bundle into the owned folder.
    let env = Env::new();
    let before = tree(&env.site.path);
    let j = env.stage();
    env.install(&Fake::new()).unwrap();
    let new_hash = env.journal().unwrap().new_hash.unwrap();
    let process = RunningCopies::new();
    assert_eq!(
        env.recover_in(&process, &env.running()),
        Recovery::RestoredKept
    );
    assert_eq!(tree(&env.site.path), before, "the previous version is back");
    assert_eq!(
        bundle_identity(&j.staged).as_deref(),
        Some(new_hash.as_str()),
        "the relocated running bundle survives"
    );
    let open = env.journal().expect("journal kept");
    assert!(open.old_restored);
    assert_eq!(open.phase, Phase::Pending);
    // any later recovery of this process (an update attempt) keeps it too
    assert_eq!(env.recover_in(&process, &env.running()), Recovery::Kept);
    assert!(j.staged.exists());
    // the next start runs the restored previous version at site: journal and folder closed
    assert_eq!(
        env.recover_in(&RunningCopies::new(), &env.running()),
        Recovery::Cleaned
    );
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
    assert_eq!(tree(&env.site.path), before);
}

#[cfg(unix)]
#[test]
fn running_inside_site_through_a_symlinked_parent_is_protected() {
    let env = Env::new();
    let j = env.stage();
    env.install(&Fake::new()).unwrap();
    let link = env.tmp.path().join("Apps-link");
    std::os::unix::fs::symlink(&env.parent, &link).unwrap();
    let running = link.join("LingoPop.app/Contents/MacOS/LingoPop");
    assert_eq!(
        env.recover_in(&RunningCopies::new(), &running),
        Recovery::RestoredKept
    );
    assert!(j.staged.exists());
    assert!(env.journal().is_some());
}

#[test]
fn committed_journal_with_the_previous_bundle_at_site_is_closed() {
    // A committed journal but `site` holds the previous version (e.g. the swap did not reach
    // the disk before a power cut): `site` is a full copy of the previous version, so the
    // folder holds no only copy and the journal must not block updates forever.
    let env = Env::new();
    let before = tree(&env.site.path);
    let j = env.stage();
    env.install(&Fake::new()).unwrap();
    commit_journal(&env.data).unwrap();
    // put the namespace back as if the swap was lost
    test_swap(&j.staged, &env.site.path).unwrap();
    assert_eq!(tree(&env.site.path), before);
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert_eq!(tree(&env.site.path), before, "site untouched");
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
}

#[test]
fn extracted_bundle_and_swap_are_durable_in_order() {
    let env = Env::new();
    let j = env.stage();
    let tools = Fake::new();
    env.install(&tools).unwrap();
    let events = tools.events();
    let swap = events
        .iter()
        .position(|e| *e == Ev::Swap(j.staged.clone(), env.site.path.clone()))
        .expect("the swap");
    let extracted = j.staged.parent().unwrap().to_owned();
    let exe = j.staged.join("Contents/MacOS/LingoPop");
    let pos = |ev: Ev| events.iter().position(|e| *e == ev).unwrap();
    // files before their folders, all before the swap
    assert!(pos(Ev::Sync(exe.clone())) < pos(Ev::Sync(j.staged.join("Contents/MacOS"))));
    assert!(pos(Ev::Sync(j.staged.clone())) < pos(Ev::Sync(extracted.clone())));
    assert!(pos(Ev::Sync(extracted.clone())) < swap);
    // the swap is durable (both parents) before install returns and the caller commits
    assert_eq!(events[swap + 1], Ev::Sync(env.parent.clone()));
    assert_eq!(events[swap + 2], Ev::Sync(extracted));
    commit_journal(&env.data).unwrap();
}

#[test]
fn a_barrier_failure_before_the_swap_aborts_with_nothing_moved() {
    let env = Env::new();
    let before = tree(&env.site.path);
    let j = env.stage();
    let tools = Fake {
        fail_sync_under: Some(j.staged.clone()),
        ..Fake::new()
    };
    assert_refused_untouched(&env, &before, env.install(&tools));
    assert_eq!(tools.swap_count(), 0);
}

#[test]
fn recovery_keeps_a_staging_folder_without_the_marker() {
    let env = Env::new();
    env.stage();
    fs::remove_file(env.folder().join(OWNER_MARKER)).unwrap();
    // the folder is not provably ours any more: kept, journal closed (nothing was moved)
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(env.folder().exists());
    assert!(env.journal().is_none());
}

#[test]
fn the_installing_process_restores_and_closes_after_a_failed_commit() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    env.install(&Fake::new()).unwrap();
    let swap = |a: &Path, b: &Path| test_swap(a, b);
    let ops = RecoverOps {
        swap: &swap,
        running_copies: &RunningCopies::new(),
        ..RecoverOps::for_kind(SiteKind::MacBundle)
    };
    assert_eq!(
        recover_after_install_with(&env.data, &env.running(), &ops),
        Recovery::Restored
    );
    assert_eq!(tree(&env.site.path), before);
    assert!(!env.folder().exists());
    assert!(env.journal().is_none());
}

// ---------------------------------------------------------------- round 2: failed barriers

#[test]
fn a_barrier_failure_after_the_swap_leaves_the_install_pending() {
    let env = Env::new();
    let j = env.stage();
    let tools = Fake {
        fail_sync_at: Some(env.parent.clone()),
        ..Fake::new()
    };
    assert!(matches!(env.install(&tools), Err(UpdateError::Install(_))));
    assert_eq!(tools.swap_count(), 1, "no swap back in this process");
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    let pending = env.journal().expect("the journal stays");
    assert_eq!(pending.phase, Phase::Pending, "not committed");
    assert!(!pending.old_restored);
    assert!(
        j.staged.exists(),
        "the previous bundle stays in the owned folder"
    );
}

#[test]
fn a_pending_install_after_a_failed_swap_barrier_is_recovered_at_the_next_start() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake {
        fail_sync_at: Some(env.parent.clone()),
        ..Fake::new()
    };
    assert!(env.install(&tools).is_err());
    // the next start runs the new version from `site`: swapped back and kept, then cleaned
    assert_eq!(env.recover_with_test_swap(), Recovery::RestoredKept);
    assert_eq!(tree(&env.site.path), before);
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(env.journal().is_none());
    assert!(!env.folder().exists());
}

#[test]
fn a_swap_back_that_cannot_be_synced_is_not_recorded() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    let tools = Fake::new();
    tools
        .reject_verify
        .lock()
        .unwrap()
        .push(env.site.path.clone());
    // the 1st sync of the parent follows the swap, the 2nd the swap back: only the 2nd fails
    let tools = FailNth {
        inner: tools,
        path: env.parent.clone(),
        n: 2,
        seen: Mutex::new(0),
    };
    assert!(env.install_with(&tools).is_err());
    assert_eq!(tree(&env.site.path), before, "the previous version is back");
    let kept = env.journal().expect("the journal stays");
    assert!(
        !kept.old_restored,
        "oldRestored is not written without the barrier"
    );
    assert!(env.folder().exists(), "no in-process cleanup");
    assert_eq!(env.recover_from_elsewhere(), Recovery::Cleaned);
    assert!(env.journal().is_none());
}

#[test]
fn recovery_swap_back_that_cannot_be_synced_is_not_recorded() {
    let env = Env::new();
    let before = tree(&env.site.path);
    env.stage();
    env.install(&Fake::new()).unwrap();
    let swap = |a: &Path, b: &Path| test_swap(a, b);
    let parent = env.parent.clone();
    let sync = |p: &Path| {
        if p == parent {
            return Err(io::Error::other("scripted sync failure"));
        }
        lingopop_platform::update_install::native_sync(p)
    };
    let copies = RunningCopies::new();
    let ops = RecoverOps {
        swap: &swap,
        sync: &sync,
        running_copies: &copies,
        ..RecoverOps::for_kind(SiteKind::MacBundle)
    };
    assert_eq!(
        recover_with(&env.data, &env.running(), &ops),
        Recovery::Kept
    );
    assert_eq!(tree(&env.site.path), before);
    let kept = env.journal().expect("the journal stays");
    assert!(!kept.old_restored);
    assert!(env.folder().exists(), "the moved bundle is not deleted");
    // the next start (the previous version) closes it
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(env.journal().is_none());
}

#[test]
fn commit_error_with_the_commit_on_record_keeps_the_running_old_bundle() {
    // The reviewer's regression: the commit reached the journal file but `commit_journal`
    // reported an error. New version at `site`, the old (running) bundle in the owned folder,
    // `running` = the original path inside `site`: nothing is deleted.
    let env = Env::new();
    let j = env.stage();
    env.install(&Fake::new()).unwrap();
    commit_journal(&env.data).unwrap();
    let swap = |a: &Path, b: &Path| test_swap(a, b);
    let ops = RecoverOps {
        swap: &swap,
        running_copies: &RunningCopies::new(),
        ..RecoverOps::for_kind(SiteKind::MacBundle)
    };
    let outcome = recover_after_install_with(&env.data, &env.running(), &ops);
    assert_eq!(outcome, Recovery::Installed);
    assert_ne!(outcome, Recovery::Cleaned);
    assert_eq!(
        read_bundle_info(&j.staged).map(|(_, v)| v).as_deref(),
        Some(CURRENT),
        "the running old bundle survives"
    );
    assert_eq!(env.journal().unwrap().phase, Phase::Committed);
    assert_eq!(env.site_version().as_deref(), Some(VERSION));
    // the next start (the new version at `site`) closes it
    assert_eq!(env.recover_with_test_swap(), Recovery::Cleaned);
    assert!(!env.folder().exists());
}

/// A `Fake` whose n-th sync of exactly `path` fails.
struct FailNth {
    inner: Fake,
    path: PathBuf,
    n: u32,
    seen: Mutex<u32>,
}

impl BundleTools for FailNth {
    fn designated_requirement(&self, app: &Path) -> Option<String> {
        self.inner.designated_requirement(app)
    }
    fn verify(&self, app: &Path, requirement: &str) -> bool {
        self.inner.verify(app, requirement)
    }
    fn extract_zip(&self, zip: &Path, into: &Path) -> Result<(), UpdateError> {
        self.inner.extract_zip(zip, into)
    }
    fn bundle_info(&self, app: &Path) -> Option<(String, String)> {
        self.inner.bundle_info(app)
    }
    fn executable_archs(&self, app: &Path) -> Vec<String> {
        self.inner.executable_archs(app)
    }
    fn swap(&self, a: &Path, b: &Path) -> Result<(), UpdateError> {
        self.inner.swap(a, b)
    }
    fn sync(&self, path: &Path) -> io::Result<()> {
        if path == self.path {
            let mut seen = self.seen.lock().unwrap();
            *seen += 1;
            if *seen == self.n {
                return Err(io::Error::other("scripted sync failure"));
            }
        }
        self.inner.sync(path)
    }
}
