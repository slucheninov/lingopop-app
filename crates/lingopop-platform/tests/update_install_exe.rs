//! Install site, update journal, recovery and the Windows exe swap (spec sections 3 and 4).
//! Portable: temp dirs only, every OS. The renames are the real no-replace renames of the host
//! OS (`renamex_np(RENAME_EXCL)` / `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`), wrapped
//! by a scripted fake where a test needs a failure.

use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lingopop_core::update_sig::UpdateError;
use lingopop_platform::update_install::{
    bundle_identity, commit_journal, confirm, exe_support, file_sha256, install_exe,
    install_exe_with, intend, native_rename_noreplace, new_token, read_journal, recover,
    recover_with, stage_exe, stage_exe_with, write_journal, HookAction, InstallOps, InstallSite,
    Journal, Phase, RecoverOps, Recovery, SiteKind, Step, Support, JOURNAL_FILE, OWNER_MARKER,
};
use tempfile::TempDir;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const VERSION: &str = "0.2.2";

/// A minimal PE image: `MZ`, `e_lfanew` = 0x40, `PE\0\0`, the given machine, then `payload`.
fn pe_with_machine(machine: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 0x40];
    v[0] = b'M';
    v[1] = b'Z';
    v[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    v.extend_from_slice(b"PE\0\0");
    v.extend_from_slice(&machine.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

fn pe(payload: &[u8]) -> Vec<u8> {
    pe_with_machine(0x8664, payload)
}

fn old_bytes() -> Vec<u8> {
    pe(b"old version")
}

fn new_bytes() -> Vec<u8> {
    pe(b"new version")
}

struct Env {
    _tmp: TempDir,
    data: PathBuf,
    dir: PathBuf,
    site: InstallSite,
}

impl Env {
    fn new() -> Self {
        Self::named("LingoPop.exe")
    }

    fn named(name: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let dir = tmp.path().join("apps");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, old_bytes()).unwrap();
        Env {
            _tmp: tmp,
            data,
            dir,
            site: InstallSite {
                path,
                kind: SiteKind::WindowsExe,
            },
        }
    }

    fn journal_path(&self) -> PathBuf {
        self.data.join(JOURNAL_FILE)
    }

    fn journal(&self) -> Option<Journal> {
        read_journal(&self.data).unwrap()
    }

    fn folder(&self) -> PathBuf {
        self.dir.join(format!(".LingoPop-update-{TOKEN}"))
    }

    fn old(&self) -> PathBuf {
        self.dir.join(format!("LingoPop.old-{TOKEN}.exe"))
    }

    /// Stage with the fixed token and put the new exe at the staged path.
    fn stage(&self) -> Journal {
        let j = stage_exe_with(
            &self.data,
            &self.site,
            VERSION,
            TOKEN,
            &InstallOps::native(),
        )
        .expect("stage");
        fs::write(&j.staged, new_bytes()).unwrap();
        j
    }

    fn install(&self, ops: &InstallOps<'_>) -> Result<(), UpdateError> {
        let staged = self.journal().unwrap().staged;
        install_exe_with(&self.data, &self.site, &staged, ops)
    }

    fn recover(&self) -> Recovery {
        recover(&self.data, &self.site.path)
    }

    fn site_bytes(&self) -> Option<Vec<u8>> {
        fs::read(&self.site.path).ok()
    }
}

fn read(p: &Path) -> Vec<u8> {
    fs::read(p).unwrap()
}

fn crash_at(at: Step) -> impl Fn(Step) -> HookAction {
    move |s| {
        if s == at {
            HookAction::Crash
        } else {
            HookAction::Continue
        }
    }
}

// ---------------------------------------------------------------- install site

#[test]
fn stem_comes_from_running_name() {
    let env = Env::named("LingoPop-0.2.1-windows-x64-portable.exe");
    let j = stage_exe_with(&env.data, &env.site, VERSION, TOKEN, &InstallOps::native()).unwrap();
    assert_eq!(
        j.old,
        env.dir.join(format!(
            "LingoPop-0.2.1-windows-x64-portable.old-{TOKEN}.exe"
        ))
    );
    assert_eq!(
        j.staged,
        env.folder().join("LingoPop-0.2.1-windows-x64-portable.exe")
    );
    assert_eq!(j.scratch, vec![env.folder()]);
    assert_eq!(j.kind, SiteKind::WindowsExe);
}

#[test]
fn new_token_is_32_lowercase_hex_and_fresh() {
    let a = new_token().unwrap();
    let b = new_token().unwrap();
    assert_eq!(a.len(), 32);
    assert!(a
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_ne!(a, b);
}

#[test]
fn writable_folder_is_supported_and_probe_removed() {
    let env = Env::new();
    match exe_support(&env.site.path) {
        Support::Supported(site) => {
            assert_eq!(site.path, env.site.path);
            assert_eq!(site.kind, SiteKind::WindowsExe);
        }
        Support::Unsupported(r) => panic!("unsupported: {r}"),
    }
    assert!(!env.dir.join("LingoPop.update.probe").exists());
}

#[cfg(unix)]
#[test]
fn read_only_folder_is_unsupported() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    fs::set_permissions(&env.dir, fs::Permissions::from_mode(0o555)).unwrap();
    // root ignores the mode; the check is meaningless there.
    let probe = env.dir.join("root-check");
    let is_root = fs::write(&probe, b"x").is_ok();
    let _ = fs::remove_file(&probe);
    let result = exe_support(&env.site.path);
    fs::set_permissions(&env.dir, fs::Permissions::from_mode(0o755)).unwrap();
    if is_root {
        eprintln!("skipped: running as root");
        return;
    }
    assert!(matches!(
        result,
        Support::Unsupported("folder not writable")
    ));
    let names: Vec<_> = fs::read_dir(&env.dir).unwrap().collect();
    assert_eq!(names.len(), 1, "nothing was written next to the exe");
}

#[cfg(unix)]
#[test]
fn staging_folder_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    env.stage();
    let mode = fs::metadata(env.folder()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert_eq!(read(&env.folder().join(OWNER_MARKER)), TOKEN.as_bytes());
}

#[test]
fn stage_writes_the_journal_with_the_old_hash_first() {
    let env = Env::new();
    let j = env.stage();
    let disk = env.journal().unwrap();
    assert_eq!(disk, j);
    assert_eq!(disk.phase, Phase::Pending);
    assert_eq!(disk.old_hash, file_sha256(&env.site.path).unwrap());
    assert_eq!(disk.new_hash, None);
    assert!(!disk.old_restored);
    assert_eq!(disk.intended, vec![env.folder()]);
    assert_eq!(disk.created, vec![env.folder()]);
    assert_eq!(disk.token, TOKEN);
    assert_eq!(disk.version, VERSION);
    // camelCase on disk
    let text = fs::read_to_string(env.journal_path()).unwrap();
    for key in [
        "\"oldHash\"",
        "\"oldRestored\"",
        "\"newHash\"",
        "\"phase\": \"pending\"",
    ] {
        assert!(text.contains(key), "{key} in {text}");
    }
}

#[test]
fn an_open_journal_is_never_overwritten() {
    let env = Env::new();
    env.stage();
    let before = read(&env.journal_path());
    let again = stage_exe(&env.data, &env.site, VERSION);
    assert!(matches!(again, Err(UpdateError::Install(_))));
    assert_eq!(read(&env.journal_path()), before);
}

#[test]
fn existing_token_path_aborts_without_deleting() {
    let env = Env::new();
    fs::create_dir(env.folder()).unwrap();
    fs::write(env.folder().join("user.txt"), b"mine").unwrap();
    let r = stage_exe_with(&env.data, &env.site, VERSION, TOKEN, &InstallOps::native());
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(read(&env.folder().join("user.txt")), b"mine");
    assert!(!env.folder().join(OWNER_MARKER).exists());
    assert!(!env.journal_path().exists());
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
}

// ---------------------------------------------------------------- install

#[test]
fn swap_replaces_exe_and_keeps_name() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    assert_eq!(env.site_bytes().unwrap(), new_bytes());
    assert_eq!(read(&env.old()), old_bytes());
    let disk = env.journal().unwrap();
    assert_eq!(disk.phase, Phase::Pending);
    assert_eq!(disk.new_hash, file_sha256(&env.site.path));
    assert!(disk.created.contains(&env.old()));
}

#[test]
fn failed_stage_leaves_exe_untouched() {
    let env = Env::new();
    let j = env.stage();
    fs::write(&j.staged, b"not a PE file").unwrap();
    let r = install_exe(&env.data, &env.site, &j.staged);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn non_amd64_pe_is_refused() {
    let env = Env::new();
    let j = env.stage();
    fs::write(&j.staged, pe_with_machine(0xAA64, b"arm64")).unwrap();
    let r = install_exe(&env.data, &env.site, &j.staged);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn pre_move_identity_mismatch_aborts() {
    let env = Env::new();
    let j = env.stage();
    fs::write(&env.site.path, pe(b"changed by the user")).unwrap();
    let r = install_exe(&env.data, &env.site, &j.staged);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), pe(b"changed by the user"));
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn rollback_when_second_rename_fails() {
    let env = Env::new();
    let j = env.stage();
    let staged = j.staged.clone();
    let hook = move |s: Step| {
        if s == Step::BeforeSecondRename {
            fs::remove_file(&staged).unwrap();
        }
        HookAction::Continue
    };
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    let r = env.install(&ops);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

/// A rename that fails on the listed call numbers (1-based) and otherwise does the real one.
fn scripted_rename(fail_on: &'static [usize]) -> impl Fn(&Path, &Path) -> io::Result<()> {
    let calls = Cell::new(0usize);
    move |from, to| {
        calls.set(calls.get() + 1);
        if fail_on.contains(&calls.get()) {
            Err(io::Error::other("scripted rename failure"))
        } else {
            native_rename_noreplace(from, to)
        }
    }
}

#[test]
fn both_renames_fail_leaves_old_exe_and_journal() {
    let env = Env::new();
    env.stage();
    // call 1 = site -> old (real), 2 = staged -> site (fails), 3 = rollback old -> site (fails)
    let rename = scripted_rename(&[2, 3]);
    let ops = InstallOps {
        rename_noreplace: &rename,
        ..InstallOps::native()
    };
    let r = env.install(&ops);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert!(!env.site.path.exists());
    assert_eq!(
        read(&env.old()),
        old_bytes(),
        "the backup holds the original"
    );
    let j = env.journal().expect("journal stays");
    assert_eq!(j.old, env.old());
    assert!(!j.old_restored);
    assert!(
        env.folder().join("LingoPop.exe").exists(),
        "nothing deleted"
    );
    // the next start restores it
    assert_eq!(env.recover(), Recovery::Restored);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn failed_rollback_keeps_journal_and_old() {
    // Covered by both_renames_fail_leaves_old_exe_and_journal; kept as a name for the plan.
    let env = Env::new();
    env.stage();
    let rename = scripted_rename(&[2, 3]);
    let ops = InstallOps {
        rename_noreplace: &rename,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert!(env.journal_path().exists());
    assert_eq!(read(&env.old()), old_bytes());
}

#[test]
fn backup_rename_collision_never_replaces() {
    let env = Env::new();
    env.stage();
    let old = env.old();
    let hook = move |s: Step| {
        if s == Step::BeforeBackupRename {
            fs::write(&old, b"appeared").unwrap();
        }
        HookAction::Continue
    };
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    let r = env.install(&ops);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(read(&env.old()), b"appeared");
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn collision_cleans_attempt_files_but_not_the_colliding_one() {
    let env = Env::new();
    env.stage();
    fs::write(env.old(), old_bytes()).unwrap(); // even with the old bytes it is not ours
    let r = env.install(&InstallOps::native());
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(read(&env.old()), old_bytes());
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists(), "the staged file is deleted");
    assert!(!env.journal_path().exists());
}

fn collide_then(action: HookAction, old: PathBuf) -> impl Fn(Step) -> HookAction {
    move |s| match s {
        Step::BeforeBackupRename => {
            fs::write(&old, b"colliding").unwrap();
            HookAction::Continue
        }
        Step::AfterCollision => action,
        _ => HookAction::Continue,
    }
}

#[test]
fn crash_right_after_collision_keeps_colliding_file() {
    let env = Env::new();
    env.stage();
    let hook = collide_then(HookAction::Crash, env.old());
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    let j = env.journal().unwrap();
    assert!(j.intended.contains(&env.old()));
    assert!(!j.created.contains(&env.old()));
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.old()), b"colliding");
    assert!(!env.folder().exists(), "only the staged file is deleted");
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn failed_rewrite_after_collision_keeps_colliding_file() {
    let env = Env::new();
    env.stage();
    let hook = collide_then(HookAction::FailNextWrite, env.old());
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert_eq!(read(&env.old()), b"colliding");
    assert!(
        env.journal_path().exists(),
        "the failed rewrite leaves the journal"
    );
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.old()), b"colliding");
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn crash_during_collision_cleanup_is_finished_by_recover() {
    let env = Env::new();
    env.stage();
    let old = env.old();
    let hook = move |s: Step| match s {
        Step::BeforeBackupRename => {
            fs::write(&old, b"colliding").unwrap();
            HookAction::Continue
        }
        Step::AfterFolderRemoved => HookAction::Crash,
        _ => HookAction::Continue,
    };
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert!(!env.folder().exists());
    assert!(env.journal_path().exists());
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(!env.journal_path().exists());
    assert_eq!(read(&env.old()), b"colliding");
}

fn foreign_at_site_before_staged_rename(
    site: PathBuf,
    remove_before_rollback: bool,
) -> impl Fn(Step) -> HookAction {
    move |s| {
        match s {
            Step::BeforeStagedRename => fs::write(&site, b"foreign").unwrap(),
            Step::BeforeRollback if remove_before_rollback => fs::remove_file(&site).unwrap(),
            _ => {}
        }
        HookAction::Continue
    }
}

#[test]
fn site_collision_before_second_rename_with_failed_rollback_keeps_backup() {
    let env = Env::new();
    env.stage();
    let hook = foreign_at_site_before_staged_rename(env.site.path.clone(), false);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    let r = env.install(&ops);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), b"foreign");
    assert_eq!(read(&env.old()), old_bytes());
    assert!(env.folder().join("LingoPop.exe").exists());
    let journal_before = read(&env.journal_path());
    // the next start: the foreign file still at site
    assert_eq!(env.recover(), Recovery::Kept);
    assert_eq!(env.site_bytes().unwrap(), b"foreign");
    assert_eq!(read(&env.old()), old_bytes());
    assert_eq!(read(&env.journal_path()), journal_before);
}

#[test]
fn site_collision_before_second_rename_with_successful_rollback_cleans() {
    let env = Env::new();
    env.stage();
    let hook = foreign_at_site_before_staged_rename(env.site.path.clone(), true);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    let r = env.install(&ops);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

// ---------------------------------------------------------------- recovery

#[test]
fn no_journal_deletes_nothing() {
    let env = Env::new();
    let user = [
        env.dir.join("LingoPop.old.exe"),
        env.dir.join("LingoPop.update.exe"),
        env.dir
            .join("LingoPop.old-ffffffffffffffffffffffffffffffff.exe"),
        env.dir
            .join(".LingoPop-update-ffffffffffffffffffffffffffffffff"),
    ];
    for p in &user[..3] {
        fs::write(p, old_bytes()).unwrap();
    }
    fs::create_dir(&user[3]).unwrap();
    assert_eq!(env.recover(), Recovery::Nothing);
    for p in &user {
        assert!(p.exists());
    }
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
}

#[test]
fn crash_between_renames_is_restored() {
    let env = Env::new();
    env.stage();
    let hook = crash_at(Step::BeforeSecondRename);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert!(!env.site.path.exists());
    assert_eq!(env.recover(), Recovery::Restored);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn committed_site_missing_restores_old_and_closes() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    fs::remove_file(&env.site.path).unwrap();
    assert_eq!(env.recover(), Recovery::Restored);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn committed_old_version_at_site_without_a_restore_is_kept() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    fs::write(&env.site.path, old_bytes()).unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert_eq!(read(&env.old()), old_bytes());
    assert!(env.journal_path().exists());
}

#[test]
fn commit_is_refused_unless_a_finished_swap_is_pending() {
    let env = Env::new();
    assert!(matches!(
        commit_journal(&env.data),
        Err(UpdateError::Install(_))
    ));
    let j = env.stage();
    // No swap yet: `newHash` is missing.
    assert!(matches!(
        commit_journal(&env.data),
        Err(UpdateError::Install(_))
    ));
    assert_eq!(env.journal().unwrap().phase, Phase::Pending);
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    // The previous version was moved back.
    let mut restored = env.journal().unwrap();
    restored.old_restored = true;
    write_journal(&env.data, &restored).unwrap();
    assert!(matches!(
        commit_journal(&env.data),
        Err(UpdateError::Install(_))
    ));
    restored.old_restored = false;
    write_journal(&env.data, &restored).unwrap();
    commit_journal(&env.data).unwrap();
    // Already committed.
    assert!(matches!(
        commit_journal(&env.data),
        Err(UpdateError::Install(_))
    ));
    assert_eq!(env.journal().unwrap().phase, Phase::Committed);
}

#[test]
fn completed_swap_is_cleaned() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    assert_eq!(env.journal().unwrap().phase, Phase::Committed);
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(env.site_bytes().unwrap(), new_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn uncommitted_swap_is_restored() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    assert_eq!(env.recover(), Recovery::Restored);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn running_old_file_is_kept() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    assert_eq!(recover(&env.data, &env.old()), Recovery::Kept);
    assert_eq!(read(&env.old()), old_bytes());
    assert!(env.journal_path().exists());
}

#[test]
fn running_bundle_inside_scratch_is_kept() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    let exe = env.folder().join("old.app/Contents/MacOS");
    fs::create_dir_all(&exe).unwrap();
    fs::write(exe.join("LingoPop"), b"running").unwrap();
    let running = exe.join("LingoPop");
    assert_eq!(recover(&env.data, &running), Recovery::Kept);
    assert!(running.exists());
    assert!(env.folder().exists());
    assert!(env.journal_path().exists());
}

#[test]
fn crash_before_swap_never_installs_staged() {
    let env = Env::new();
    env.stage();
    assert_eq!(env.journal().unwrap().new_hash, None);
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn crash_after_hash_before_swap_never_installs_staged() {
    let env = Env::new();
    env.stage();
    let hook = crash_at(Step::BeforeFirstRename);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert!(env.journal().unwrap().new_hash.is_some());
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn committed_identity_mismatch_keeps_old() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    commit_journal(&env.data).unwrap();
    fs::write(&env.site.path, pe(b"changed after commit")).unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert_eq!(read(&env.old()), old_bytes());
    assert!(env.journal_path().exists());
    assert_eq!(env.site_bytes().unwrap(), pe(b"changed after commit"));
}

#[test]
fn unknown_item_at_site_keeps_everything() {
    // The backup exists (the crash came between the renames) and a foreign file took `site`.
    let env = Env::new();
    env.stage();
    let hook = crash_at(Step::BeforeSecondRename);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    fs::write(&env.site.path, b"unknown").unwrap();
    let before = read(&env.journal_path());
    assert_eq!(env.recover(), Recovery::Kept);
    assert_eq!(env.site_bytes().unwrap(), b"unknown");
    assert_eq!(read(&env.old()), old_bytes());
    assert!(env.folder().join("LingoPop.exe").exists());
    assert_eq!(read(&env.journal_path()), before);
}

#[test]
fn changed_backup_before_restore_keeps_journal() {
    let env = Env::new();
    env.stage();
    let hook = crash_at(Step::BeforeSecondRename);
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    fs::write(env.old(), b"changed backup").unwrap();
    fs::write(&env.site.path, b"foreign").unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert_eq!(read(&env.old()), b"changed backup");
    assert_eq!(env.site_bytes().unwrap(), b"foreign");
    assert!(env.journal_path().exists());
}

#[test]
fn change_between_check_and_rename_is_restored_and_closed() {
    let env = Env::new();
    env.stage();
    let site = env.site.path.clone();
    let hook = move |s: Step| match s {
        Step::BeforeFirstRename => {
            fs::write(&site, pe(b"changed in the gap")).unwrap();
            HookAction::Continue
        }
        Step::BeforeSecondRename => HookAction::Crash,
        _ => HookAction::Continue,
    };
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert!(!env.site.path.exists());
    assert_eq!(read(&env.old()), pe(b"changed in the gap"));
    let r = env.recover();
    assert_eq!(r, Recovery::Restored);
    assert_eq!(env.site_bytes().unwrap(), pe(b"changed in the gap"));
    assert!(!env.old().exists());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

/// Install with the staged file removed before the second rename (so the rollback runs) and a
/// crash at `at` inside the rollback.
fn rollback_then_crash(env: &Env, at: Step) {
    env.stage();
    let staged = env.journal().unwrap().staged;
    let hook = move |s: Step| {
        if s == Step::BeforeSecondRename {
            fs::remove_file(&staged).unwrap();
        }
        if s == at {
            HookAction::Crash
        } else {
            HookAction::Continue
        }
    };
    let ops = InstallOps {
        hook: &hook,
        ..InstallOps::native()
    };
    assert!(env.install(&ops).is_err());
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
}

#[test]
fn reused_vacated_old_path_is_kept() {
    let env = Env::new();
    rollback_then_crash(&env, Step::AfterRollback);
    assert!(env.journal().unwrap().old_restored);
    fs::write(env.old(), b"someone else's").unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.old()), b"someone else's");
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn restored_flag_blocks_restore_of_foreign_old() {
    let env = Env::new();
    rollback_then_crash(&env, Step::AfterRollback);
    fs::remove_file(&env.site.path).unwrap();
    fs::write(env.old(), b"foreign").unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert!(!env.site.path.exists());
    assert_eq!(read(&env.old()), b"foreign");
    assert!(env.folder().exists());
    assert!(env.journal_path().exists());
}

#[test]
fn restored_flag_keeps_foreign_old_with_old_bytes() {
    let env = Env::new();
    rollback_then_crash(&env, Step::AfterRollback);
    fs::write(env.old(), old_bytes()).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.old()), old_bytes());
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn crash_between_restore_and_flag_is_closed() {
    let env = Env::new();
    rollback_then_crash(&env, Step::AfterRollbackMove);
    let j = env.journal().unwrap();
    assert!(!j.old_restored);
    assert!(!env.old().exists());
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
}

#[test]
fn interrupted_recovery_after_swap_back_is_idempotent() {
    let env = Env::new();
    let j = env.stage();
    install_exe(&env.data, &env.site, &j.staged).unwrap();
    let hook = crash_at(Step::AfterSwapBack);
    let ops = RecoverOps {
        hook: &hook,
        ..RecoverOps::for_kind(SiteKind::WindowsExe)
    };
    assert_eq!(
        recover_with(&env.data, &env.site.path, &ops),
        Recovery::Kept
    );
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert_eq!(read(&j.staged), new_bytes());
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.folder().exists(), "the new copy is deleted");
    assert!(!env.old().exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn recover_ignores_paths_not_in_created() {
    let env = Env::new();
    env.stage();
    // A user file with the very bytes of the old version at the backup name: never created by us.
    fs::write(env.old(), old_bytes()).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.old()), old_bytes());
    assert!(!env.journal_path().exists());
}

fn hand_journal(env: &Env, staged: PathBuf) -> Journal {
    Journal {
        version: VERSION.into(),
        kind: SiteKind::WindowsExe,
        token: TOKEN.into(),
        site: env.site.path.clone(),
        staged,
        old: env.old(),
        scratch: vec![],
        phase: Phase::Pending,
        old_hash: file_sha256(&env.site.path).unwrap(),
        old_restored: false,
        new_hash: None,
        intended: vec![],
        created: vec![],
    }
}

#[test]
fn crash_after_create_before_confirm_leaves_leftover() {
    let env = Env::new();
    let staged = env.dir.join(format!("LingoPop.update-{TOKEN}.exe"));
    let mut j = hand_journal(&env, staged.clone());
    j.intended.push(staged.clone());
    write_journal(&env.data, &j).unwrap();
    fs::write(&staged, new_bytes()).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(
        read(&staged),
        new_bytes(),
        "uncertain path is left in place"
    );
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn created_path_is_cleaned() {
    let env = Env::new();
    let staged = env.dir.join(format!("LingoPop.update-{TOKEN}.exe"));
    let mut j = hand_journal(&env, staged.clone());
    write_journal(&env.data, &j).unwrap();
    intend(&env.data, &mut j, &staged).unwrap();
    fs::write(&staged, new_bytes()).unwrap();
    confirm(&env.data, &mut j, &staged).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(!staged.exists());
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn recorded_but_never_created_path_is_skipped() {
    let env = Env::new();
    let ghost = env.dir.join(format!("LingoPop.ghost-{TOKEN}.exe"));
    let mut j = hand_journal(&env, ghost.clone());
    j.created.push(ghost.clone());
    write_journal(&env.data, &j).unwrap();
    let neighbour = env.dir.join("notes.txt");
    fs::write(&neighbour, b"keep").unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&neighbour), b"keep");
    assert_eq!(env.site_bytes().unwrap(), old_bytes());
    assert!(!env.journal_path().exists());
}

#[test]
fn intend_refuses_an_existing_path() {
    let env = Env::new();
    let mut j = env.stage();
    let taken = env.folder().join("LingoPop.exe"); // written by Env::stage
    let r = intend(&env.data, &mut j, &taken);
    assert!(matches!(r, Err(UpdateError::Install(_))));
    assert!(!env.journal().unwrap().intended.contains(&taken));
}

#[test]
fn owned_folder_is_removed_whole() {
    let env = Env::new();
    let mut j = env.stage();
    let download = env.folder().join("download.zip");
    intend(&env.data, &mut j, &download).unwrap();
    fs::write(&download, b"zip").unwrap();
    confirm(&env.data, &mut j, &download).unwrap();
    let app = env.folder().join("LingoPop.app/Contents/MacOS");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("LingoPop"), b"exe").unwrap();
    fs::write(env.folder().join("untracked.tmp"), b"x").unwrap();
    let neighbour = env.dir.join("user-file.txt");
    fs::write(&neighbour, b"mine").unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(!env.folder().exists());
    assert_eq!(read(&neighbour), b"mine");
    assert!(!env.journal_path().exists());
}

#[test]
fn collision_inside_staging_folder_removes_the_folder() {
    let env = Env::new();
    let mut j = env.stage();
    let download = env.folder().join("LingoPop.exe.minisig");
    intend(&env.data, &mut j, &download).unwrap();
    // something else wrote into the private folder: the create_new of the download fails
    fs::write(&download, b"intruder").unwrap();
    let neighbour = env.dir.join("user-file.txt");
    fs::write(&neighbour, b"mine").unwrap();
    // the attempt fails and runs recovery (Task 7 calls recover on failure / cancel)
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(
        !env.folder().exists(),
        "colliding file removed with the folder"
    );
    assert_eq!(read(&neighbour), b"mine");
    assert!(!env.journal_path().exists());
}

#[test]
fn replacement_staging_folder_is_kept() {
    let env = Env::new();
    env.stage();
    let hook = crash_at(Step::AfterFolderRemoved);
    let ops = RecoverOps {
        hook: &hook,
        ..RecoverOps::for_kind(SiteKind::WindowsExe)
    };
    assert_eq!(
        recover_with(&env.data, &env.site.path, &ops),
        Recovery::Kept
    );
    assert!(!env.folder().exists());
    assert!(env.journal_path().exists());
    fs::create_dir(env.folder()).unwrap();
    fs::write(env.folder().join("theirs.txt"), b"theirs").unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(read(&env.folder().join("theirs.txt")), b"theirs");
    assert!(!env.journal_path().exists());
}

#[test]
fn staging_folder_with_foreign_marker_is_kept() {
    let env = Env::new();
    env.stage();
    fs::write(
        env.folder().join(OWNER_MARKER),
        b"ffffffffffffffffffffffffffffffff",
    )
    .unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(env.folder().join("LingoPop.exe").exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn staging_folder_without_marker_is_kept() {
    let env = Env::new();
    env.stage();
    fs::remove_file(env.folder().join(OWNER_MARKER)).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(env.folder().join("LingoPop.exe").exists());
    assert!(!env.journal_path().exists());
}

#[test]
fn site_missing_after_restore_touches_nothing() {
    let env = Env::new();
    rollback_then_crash(&env, Step::AfterRollback);
    fs::remove_file(&env.site.path).unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert!(env.journal_path().exists());
    assert!(env.folder().exists());
}

#[test]
fn unreadable_journal_is_kept() {
    let env = Env::new();
    fs::write(env.journal_path(), b"{ not json").unwrap();
    assert_eq!(env.recover(), Recovery::Kept);
    assert!(env.journal_path().exists());
}

// ---------------------------------------------------------------- macOS bundle identity

fn fake_bundle(root: &Path, version: &str, code_resources: &[u8], exe: &[u8]) {
    let contents = root.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::create_dir_all(contents.join("_CodeSignature")).unwrap();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>LingoPop</string>
	<key>CFBundleIdentifier</key>
	<string>app.lingopop.tauri.macos</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
</dict>
</plist>
"#
    );
    fs::write(contents.join("Info.plist"), plist).unwrap();
    fs::write(
        contents.join("_CodeSignature/CodeResources"),
        code_resources,
    )
    .unwrap();
    fs::write(contents.join("MacOS/LingoPop"), exe).unwrap();
}

#[test]
fn macos_identity_includes_the_executable() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a/LingoPop.app");
    let b = tmp.path().join("b/LingoPop.app");
    fake_bundle(&a, "0.2.1", b"resources", b"exe one");
    fake_bundle(&b, "0.2.1", b"resources", b"exe two");
    let ia = bundle_identity(&a).unwrap();
    let ib = bundle_identity(&b).unwrap();
    assert_ne!(ia, ib);
    assert!(ia.starts_with("0.2.1:"));
    assert_eq!(ia.split(':').count(), 3);
    // no Info.plist = no identity
    fs::remove_file(a.join("Contents/Info.plist")).unwrap();
    assert_eq!(bundle_identity(&a), None);
}

/// A portable stand-in for `renamex_np(RENAME_SWAP)` in tests (not atomic; fine on temp dirs).
fn test_swap(a: &Path, b: &Path) -> io::Result<()> {
    let tmp = a.with_extension("swap-tmp");
    fs::rename(a, &tmp)?;
    fs::rename(b, a)?;
    fs::rename(&tmp, b)
}

#[test]
fn bundles_with_identical_executables_are_told_apart() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let parent = tmp.path().join("Applications");
    let site = parent.join("LingoPop.app");
    let folder = parent.join(format!(".LingoPop-update-{VERSION}-{TOKEN}"));
    let staged = folder.join("LingoPop.app");
    fake_bundle(&site, "0.2.1", b"resources old", b"same executable");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(OWNER_MARKER), TOKEN).unwrap();
    fake_bundle(&staged, "0.2.2", b"resources new", b"same executable");
    let old_hash = bundle_identity(&site).unwrap();
    let new_hash = bundle_identity(&staged).unwrap();
    assert_ne!(old_hash, new_hash);
    // the swap happened (pending): new at site, old in the staging folder
    test_swap(&staged, &site).unwrap();
    let j = Journal {
        version: VERSION.into(),
        kind: SiteKind::MacBundle,
        token: TOKEN.into(),
        site: site.clone(),
        staged: staged.clone(),
        old: staged.clone(),
        scratch: vec![folder.clone()],
        phase: Phase::Pending,
        old_hash: old_hash.clone(),
        old_restored: false,
        new_hash: Some(new_hash),
        intended: vec![folder.clone()],
        created: vec![folder.clone()],
    };
    write_journal(&data, &j).unwrap();
    let swaps = RefCell::new(0);
    let swap = |a: &Path, b: &Path| {
        *swaps.borrow_mut() += 1;
        test_swap(a, b)
    };
    let ops = RecoverOps {
        swap: &swap,
        ..RecoverOps::for_kind(SiteKind::MacBundle)
    };
    let running = tmp.path().join("elsewhere/LingoPop");
    assert_eq!(recover_with(&data, &running, &ops), Recovery::Restored);
    assert_eq!(*swaps.borrow(), 1);
    assert_eq!(bundle_identity(&site).unwrap(), old_hash);
    assert!(!folder.exists());
    assert!(!data.join(JOURNAL_FILE).exists());
}

#[test]
fn macos_bundle_with_missing_site_is_never_restored_from_staged() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let parent = tmp.path().join("Applications");
    let site = parent.join("LingoPop.app");
    let folder = parent.join(format!(".LingoPop-update-{VERSION}-{TOKEN}"));
    let staged = folder.join("LingoPop.app");
    fs::create_dir_all(&folder).unwrap();
    fake_bundle(&staged, "0.2.2", b"r", b"e");
    let j = Journal {
        version: VERSION.into(),
        kind: SiteKind::MacBundle,
        token: TOKEN.into(),
        site: site.clone(),
        staged: staged.clone(),
        old: staged.clone(),
        scratch: vec![folder.clone()],
        phase: Phase::Pending,
        old_hash: "0.2.1:x:y".into(),
        old_restored: false,
        // Every other guard of the restore rule holds (a swap may have happened, `old` is
        // tracked and exists): only the bundle kind stops the rename from the staged path.
        new_hash: Some(bundle_identity(&staged).unwrap()),
        intended: vec![folder.clone()],
        created: vec![folder.clone()],
    };
    write_journal(&data, &j).unwrap();
    assert_eq!(recover(&data, &tmp.path().join("x")), Recovery::Kept);
    assert!(!site.exists());
    assert!(staged.exists());
    assert!(data.join(JOURNAL_FILE).exists());
}

// ---------------------------------------------------------------- Windows production rename

#[cfg(windows)]
#[test]
fn running_exe_is_swapped_by_the_production_rename() {
    use std::process::{Command, Stdio};
    let root = std::env::var_os("SystemRoot").expect("SystemRoot");
    let system32 = PathBuf::from(root).join("System32");
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let dir = tmp.path().join("app");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("LingoPop.exe");
    fs::copy(system32.join("PING.EXE"), &exe).unwrap();
    // System binaries keep their strings in `<lang>\<name>.mui` next to the image: copy them
    // under the new name so the copy behaves like the original.
    for entry in fs::read_dir(&system32).unwrap().flatten() {
        let mui = entry.path().join("PING.EXE.mui");
        if mui.is_file() {
            let lang = dir.join(entry.file_name());
            let _ = fs::create_dir_all(&lang);
            let _ = fs::copy(&mui, lang.join("LingoPop.exe.mui"));
        }
    }
    let ping = read(&exe);
    let whoami = read(&system32.join("whoami.exe"));
    let mut child = Command::new(&exe)
        .args(["-n", "30", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the copied exe keeps running"
    );
    let site = match exe_support(&exe) {
        Support::Supported(site) => site,
        Support::Unsupported(r) => panic!("unsupported: {r}"),
    };
    let j = stage_exe(&data, &site, VERSION).unwrap();
    fs::write(&j.staged, &whoami).unwrap();
    let result = install_exe(&data, &site, &j.staged);
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
    assert!(still_running, "the rename ran while the exe was running");
    assert_eq!(read(&exe), whoami);
    let old = dir.join(format!("LingoPop.old-{}.exe", j.token));
    assert_eq!(read(&old), ping);
    commit_journal(&data).unwrap();
    // the image may stay mapped briefly after the process exits
    let mut outcome = Recovery::Kept;
    for _ in 0..50 {
        outcome = recover(&data, &exe);
        if outcome != Recovery::Kept {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(outcome, Recovery::Cleaned);
    assert!(!old.exists());
    assert_eq!(read(&exe), whoami);
}

#[test]
fn unknown_site_before_any_swap_closes_the_journal() {
    // No `newHash`: no move can have happened, so a file at `old` is not a backup.
    let env = Env::new();
    env.stage();
    fs::write(env.old(), b"user file").unwrap();
    fs::write(&env.site.path, pe(b"user replaced the exe by hand")).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert_eq!(
        env.site_bytes().unwrap(),
        pe(b"user replaced the exe by hand")
    );
    assert_eq!(read(&env.old()), b"user file");
    assert!(!env.folder().exists());
    assert!(!env.journal_path().exists());
}

#[cfg(unix)]
#[test]
fn symlink_at_a_created_path_is_kept() {
    let env = Env::new();
    let target = env.dir.join("user-target.txt");
    fs::write(&target, b"mine").unwrap();
    let link = env.dir.join(format!("LingoPop.update-{TOKEN}.exe"));
    let mut j = hand_journal(&env, link.clone());
    j.created.push(link.clone());
    write_journal(&env.data, &j).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(env.recover(), Recovery::Cleaned);
    assert!(fs::symlink_metadata(&link).is_ok());
    assert_eq!(read(&target), b"mine");
}
