//! In-app update state machine (spec "In-app updates" sections 3 and 6) over fakes: the release
//! JSON comes from `FakeTransport`, the downloads from a scripted [`FakeDownloader`], the
//! signature check uses the committed core test key, and the install is a [`FakeInstaller`].
//! The data folder and the "install site" are temp dirs; the journal, staging and recovery are
//! the real `lingopop_platform::update_install` functions (Windows exe kind, portable).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use lingopop_core::http::FakeTransport;
use lingopop_core::time::{ManualClock, Timestamp};
use lingopop_core::update_sig::{update_comment, verify_update_with, UpdateError};
use lingopop_core::updates::{UpdatePlan, UpdateTarget};
use lingopop_platform::update_install::{
    confirm, file_sha256, intend, read_journal, recover, write_journal, InstallSite, Journal,
    Phase, Recovery, SiteKind, Support, JOURNAL_FILE,
};
use tokio::sync::Notify;

use crate::updates::{
    relaunch_command, relaunch_target_for, startup_recovery, Download, Installer, RelaunchTarget,
    UpdateState, UpdateStatus, UpdateView, UpdaterDeps,
};

const FIXTURE_ASSET: &str = "LingoPop-0.2.1-test.zip";
const ASSET_BYTES: &[u8] =
    include_bytes!("../../../crates/lingopop-core/tests/fixtures/updates/LingoPop-0.2.1-test.zip");
const SIG: &str = include_str!(
    "../../../crates/lingopop-core/tests/fixtures/updates/LingoPop-0.2.1-test.zip.minisig"
);
const TEST_PUB: &str =
    include_str!("../../../crates/lingopop-core/tests/fixtures/updates/test.pub");
const NEW: &str = "9.9.9";
const NOW: f64 = 1_800_000_000.0;
const INSTALL_KEY: &str = "The update could not be installed. Your current version is unchanged.";
const DOWNLOAD_KEY: &str = "The update could not be downloaded.";
const VERIFY_KEY: &str = "The update could not be verified.";

fn key_line() -> &'static str {
    TEST_PUB.lines().nth(1).unwrap().trim()
}

fn asset_name() -> String {
    UpdateTarget::WindowsX64.asset_name(NEW)
}

fn url(name: &str) -> String {
    format!("https://github.com/slucheninov/lingopop-app/releases/download/v{NEW}/{name}")
}

fn release_json(size: u64) -> String {
    let asset = asset_name();
    format!(
        r#"{{"tag_name":"v{NEW}","assets":[
            {{"name":"{asset}","size":{size},"browser_download_url":"{}"}},
            {{"name":"{asset}.minisig","size":{},"browser_download_url":"{}"}}]}}"#,
        url(&asset),
        SIG.len(),
        url(&format!("{asset}.minisig")),
    )
}

/// What a scripted download does.
#[derive(Clone)]
enum Mode {
    /// Writes the bytes and returns their length.
    Serve,
    /// Writes the bytes but reports this many.
    Report(u64),
    /// Writes a few bytes, signals `started`, then never finishes (removed when dropped).
    Hang,
    /// Calls `progress` this many times, advancing the clock 5 ms each time.
    Chatty(usize),
    /// Reports progress 1, steps the clock back 10 s, then reports progress 2.
    Backwards,
    /// Panics inside the download (the async part of the attempt).
    Panic,
}

/// Holds a blocking step until the test releases it: `entered` is signalled first.
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl Gate {
    /// Arms the gate; dropping or sending on the returned sender releases it.
    fn arm(&self) -> std::sync::mpsc::Sender<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.release.lock().unwrap() = Some(rx);
        tx
    }

    /// Called on the blocking thread.
    fn pass(&self) {
        let rx = self.release.lock().unwrap().take();
        if let Some(rx) = rx {
            self.entered.notify_one();
            let _ = rx.recv();
        }
    }
}

/// What the fake install does.
#[derive(Clone, Copy, PartialEq)]
enum InstallMode {
    /// Copies the download onto `site` and records `newHash`.
    Copy,
    /// A real swap (`site` -> backup, download -> `site`), then the commit fails.
    SwapThenCommitFails,
    /// The same swap, then the commit reaches the journal file but reports an error (its
    /// durability barrier failed).
    SwapThenCommitOnRecordFails,
    /// Panics on the blocking thread.
    Panic,
}

#[derive(Debug, Clone)]
struct Call {
    url: String,
    dest: PathBuf,
    journal: Option<Journal>,
}

struct FakeDownloader {
    data_dir: PathBuf,
    asset: Mutex<Mode>,
    signature: Mutex<String>,
    calls: Mutex<Vec<Call>>,
    started: Notify,
    clock: Arc<ManualClock>,
}

/// Removes the partial file when the future is dropped, like the real downloader.
struct Partial(PathBuf, bool);
impl Drop for Partial {
    fn drop(&mut self) {
        if !self.1 {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

#[async_trait::async_trait]
impl Download for FakeDownloader {
    async fn download(
        &self,
        url: &str,
        _max_bytes: u64,
        dest: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, UpdateError> {
        self.calls.lock().unwrap().push(Call {
            url: url.to_owned(),
            dest: dest.to_owned(),
            journal: read_journal(&self.data_dir).unwrap(),
        });
        let bytes: Vec<u8> = if url.ends_with(".minisig") {
            self.signature.lock().unwrap().clone().into_bytes()
        } else {
            ASSET_BYTES.to_vec()
        };
        let mode = if url.ends_with(".minisig") {
            Mode::Serve
        } else {
            self.asset.lock().unwrap().clone()
        };
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)
            .map_err(|_| UpdateError::Download)?;
        let mut guard = Partial(dest.to_owned(), false);
        match mode {
            Mode::Panic => panic!("scripted download panic"),
            Mode::Hang => {
                std::fs::write(dest, &bytes[..4]).unwrap();
                self.started.notify_one();
                std::future::pending::<()>().await;
                unreachable!()
            }
            Mode::Serve | Mode::Report(_) | Mode::Chatty(_) | Mode::Backwards => {
                std::fs::write(dest, &bytes).unwrap();
                if let Mode::Chatty(n) = mode {
                    for i in 0..n {
                        self.clock.advance(0.005);
                        progress(i as u64, Some(bytes.len() as u64));
                    }
                }
                if let Mode::Backwards = mode {
                    progress(1, Some(bytes.len() as u64));
                    self.clock.advance(-10.0);
                    progress(2, Some(bytes.len() as u64));
                }
                guard.1 = true;
                Ok(match mode {
                    Mode::Report(n) => n,
                    _ => bytes.len() as u64,
                })
            }
        }
    }
}

struct FakeInstaller {
    data_dir: PathBuf,
    support: Support,
    installs: Mutex<Vec<(String, PathBuf)>>,
    relaunches: AtomicUsize,
    mode: Mutex<InstallMode>,
    gate: Gate,
}

impl Installer for FakeInstaller {
    fn support(&self) -> &Support {
        &self.support
    }

    /// Mirrors a successful swap: `newHash` recorded, the download now at `site`.
    fn install(&self, plan: &UpdatePlan, downloaded: &Path) -> Result<(), UpdateError> {
        self.installs
            .lock()
            .unwrap()
            .push((plan.version.clone(), downloaded.to_owned()));
        self.gate.pass();
        let Support::Supported(site) = &self.support else {
            return Err(UpdateError::Unsupported("test"));
        };
        let mode = *self.mode.lock().unwrap();
        if mode == InstallMode::Panic {
            panic!("scripted install panic");
        }
        let mut j = read_journal(&self.data_dir).unwrap().unwrap();
        j.new_hash = file_sha256(downloaded);
        write_journal(&self.data_dir, &j).unwrap();
        if matches!(
            mode,
            InstallMode::SwapThenCommitFails | InstallMode::SwapThenCommitOnRecordFails
        ) {
            // Like `install_exe`: the backup is tracked, then the two renames.
            let old = j.old.clone();
            intend(&self.data_dir, &mut j, &old).unwrap();
            std::fs::rename(&site.path, &old).unwrap();
            confirm(&self.data_dir, &mut j, &old).unwrap();
            std::fs::rename(downloaded, &site.path).unwrap();
        } else {
            std::fs::copy(downloaded, &site.path).unwrap();
        }
        Ok(())
    }

    fn commit(&self, data_dir: &Path) -> Result<(), UpdateError> {
        match *self.mode.lock().unwrap() {
            InstallMode::SwapThenCommitFails => {
                return Err(UpdateError::Install("scripted commit failure"));
            }
            InstallMode::SwapThenCommitOnRecordFails => {
                lingopop_platform::update_install::commit_journal(data_dir).unwrap();
                return Err(UpdateError::Install("scripted commit failure"));
            }
            _ => {}
        }
        lingopop_platform::update_install::commit_journal(data_dir)
    }

    fn relaunch(&self) -> std::io::Result<()> {
        self.relaunches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    data: PathBuf,
    site: PathBuf,
    state: Arc<UpdateState>,
    downloader: Arc<FakeDownloader>,
    installer: Arc<FakeInstaller>,
    verify_gate: Arc<Gate>,
    clock: Arc<ManualClock>,
    emitted: Arc<Mutex<Vec<UpdateView>>>,
    comments: Arc<Mutex<Vec<String>>>,
    transport: Arc<FakeTransport>,
    ui: Arc<crate::state::UiState>,
}

fn rig_with(release: FakeTransport, support: Option<&'static str>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let site_dir = dir.path().join("site");
    std::fs::create_dir_all(&site_dir).unwrap();
    let site = site_dir.join("LingoPop.exe");
    std::fs::write(&site, b"old exe").unwrap();
    let (_config, _stores, ui) = crate::task4_tests::rig(&dir.path().join("ui"));
    let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(NOW)));
    let downloader = Arc::new(FakeDownloader {
        data_dir: data.clone(),
        asset: Mutex::new(Mode::Serve),
        signature: Mutex::new(SIG.to_owned()),
        calls: Mutex::new(Vec::new()),
        started: Notify::new(),
        clock: clock.clone(),
    });
    let installer = Arc::new(FakeInstaller {
        data_dir: data.clone(),
        support: match support {
            None => Support::Supported(InstallSite {
                path: site.clone(),
                kind: SiteKind::WindowsExe,
            }),
            Some(why) => Support::Unsupported(why),
        },
        installs: Mutex::new(Vec::new()),
        relaunches: AtomicUsize::new(0),
        mode: Mutex::new(InstallMode::Copy),
        gate: Gate::default(),
    });
    let verify_gate = Arc::new(Gate::default());
    let held = verify_gate.clone();
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let comments = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let seen = comments.clone();
    let transport = Arc::new(release);
    let state = UpdateState::new(transport.clone(), ui.clone(), clock.clone())
        .with_deadline(Arc::new(|| Box::pin(std::future::pending())))
        .with_updater(UpdaterDeps {
            data_dir: data.clone(),
            running: site.clone(),
            target: Some(UpdateTarget::WindowsX64),
            installer: installer.clone(),
            downloader: downloader.clone(),
            // The fixture is signed for its own asset name; the comment the shell asks for is
            // recorded so the tests can check it is the plan's.
            verify: Arc::new(move |bytes, sig, comment| {
                seen.lock().unwrap().push(comment.to_owned());
                held.pass();
                verify_update_with(key_line(), bytes, sig, &update_comment(FIXTURE_ASSET))
            }),
            emit: Arc::new(move |view| sink.lock().unwrap().push(view.clone())),
        });
    Rig {
        _dir: dir,
        data,
        site,
        state: Arc::new(state),
        downloader,
        installer,
        verify_gate,
        clock,
        emitted,
        comments,
        transport,
        ui,
    }
}

fn rig() -> Rig {
    rig_with(
        FakeTransport::json(200, &release_json(ASSET_BYTES.len() as u64)),
        None,
    )
}

impl Rig {
    async fn available(&self) -> UpdateView {
        let view = self.state.check().await;
        assert_eq!(view.state, UpdateStatus::Available, "{view:?}");
        view
    }

    async fn install_and_settle(&self) -> UpdateView {
        self.state.install();
        self.state.settle().await;
        self.state.current()
    }

    fn journal_path(&self) -> PathBuf {
        self.data.join(JOURNAL_FILE)
    }

    /// Everything in the site folder besides the exe itself.
    fn site_leftovers(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.site.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "LingoPop.exe")
            .collect();
        names.sort();
        names
    }

    fn calls(&self) -> Vec<Call> {
        self.downloader.calls.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn check_available_then_install_reaches_ready() {
    let rig = rig();
    let view = rig.available().await;
    assert_eq!(view.version.as_deref(), Some(NEW));
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["state"], "available");
    assert_eq!(json["version"], NEW);
    assert!(json.get("revision").is_some());

    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Ready, "{view:?}");
    assert_eq!(view.version.as_deref(), Some(NEW));
    // Signature first, then the asset.
    let calls = rig.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].url.ends_with(".minisig"));
    assert_eq!(calls[1].url, url(&asset_name()));
    assert_eq!(
        rig.comments.lock().unwrap().as_slice(),
        [update_comment(&asset_name())]
    );
    assert_eq!(rig.installer.installs.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read(&rig.site).unwrap(), ASSET_BYTES);
    // The journal is committed; the next start cleans it up.
    let j = read_journal(&rig.data).unwrap().unwrap();
    assert_eq!(j.phase, Phase::Committed);
    assert_eq!(recover(&rig.data, &rig.site), Recovery::Cleaned);
    assert!(rig.site_leftovers().is_empty());
    // The check also feeds the existing banner cache.
    assert_eq!(
        rig.ui.prefs().latest_version.as_deref(),
        Some(NEW),
        "updates_check updates the cache like check_now"
    );
    // Events and the wire shape of a progress state.
    let emitted = rig.emitted.lock().unwrap().clone();
    let states: Vec<_> = emitted.iter().map(|v| v.state).collect();
    assert!(states.contains(&UpdateStatus::Checking));
    assert!(states.contains(&UpdateStatus::Downloading));
    assert!(states.contains(&UpdateStatus::Installing));
    assert_eq!(*states.last().unwrap(), UpdateStatus::Ready);
    let downloading = emitted
        .iter()
        .find(|v| v.state == UpdateStatus::Downloading)
        .unwrap();
    let json = serde_json::to_value(downloading).unwrap();
    assert_eq!(json["state"], "downloading");
    assert_eq!(json["total"], ASSET_BYTES.len() as u64);
    assert!(json.get("received").is_some());
    assert!(json.get("error").is_none());
}

#[tokio::test]
async fn install_is_single_flight() {
    let rig = rig();
    rig.available().await;
    *rig.downloader.asset.lock().unwrap() = Mode::Hang;
    let first = rig.state.install();
    assert_eq!(first.state, UpdateStatus::Downloading);
    rig.downloader.started.notified().await;
    let second = rig.state.install();
    assert_eq!(second.state, UpdateStatus::Downloading);
    assert_eq!(second.revision, rig.state.current().revision);
    // Only one attempt: one signature and one asset download, one journal.
    assert_eq!(rig.calls().len(), 2);
    let view = rig.state.cancel().await;
    assert_eq!(view.state, UpdateStatus::Available);
    assert_eq!(rig.calls().len(), 2);
}

#[tokio::test]
async fn cancel_returns_to_available_and_removes_files() {
    let rig = rig();
    rig.available().await;
    *rig.downloader.asset.lock().unwrap() = Mode::Hang;
    rig.state.install();
    rig.downloader.started.notified().await;
    let dest = rig.calls()[1].dest.clone();
    assert!(dest.exists(), "the partial download exists while it runs");
    let view = rig.state.cancel().await;
    assert_eq!(view.state, UpdateStatus::Available);
    assert_eq!(view.version.as_deref(), Some(NEW));
    assert!(!dest.exists());
    assert!(!rig.journal_path().exists());
    assert!(
        rig.site_leftovers().is_empty(),
        "{:?}",
        rig.site_leftovers()
    );
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
    assert!(rig.installer.installs.lock().unwrap().is_empty());
    // A new attempt can start afterwards.
    *rig.downloader.asset.lock().unwrap() = Mode::Serve;
    assert_eq!(rig.install_and_settle().await.state, UpdateStatus::Ready);
}

#[tokio::test]
async fn signature_failure_is_failed_and_installer_not_called() {
    let rig = rig();
    rig.available().await;
    let tampered = SIG.replace("lingopop-update", "lingopop-updatX");
    *rig.downloader.signature.lock().unwrap() = tampered;
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed);
    assert_eq!(view.error.as_deref(), Some(VERIFY_KEY));
    assert_eq!(view.version.as_deref(), Some(NEW));
    assert!(rig.installer.installs.lock().unwrap().is_empty());
    assert!(!rig.journal_path().exists());
    assert!(rig.site_leftovers().is_empty());
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
}

/// A journal that recovery leaves open: an unknown item at `site` while the backup exists.
fn unresolvable_journal(rig: &Rig) -> Vec<u8> {
    std::fs::write(&rig.site, b"foreign").unwrap();
    let token = "0123456789abcdef".repeat(2);
    let dir = rig.site.parent().unwrap();
    let old = dir.join(format!("LingoPop.old-{token}.exe"));
    std::fs::write(&old, b"backup").unwrap();
    let j = Journal {
        version: NEW.into(),
        kind: SiteKind::WindowsExe,
        token: token.clone(),
        site: rig.site.clone(),
        staged: dir
            .join(format!(".LingoPop-update-{token}"))
            .join("LingoPop.exe"),
        old: old.clone(),
        scratch: vec![dir.join(format!(".LingoPop-update-{token}"))],
        phase: Phase::Pending,
        old_hash: "a".repeat(64),
        old_restored: false,
        new_hash: Some("b".repeat(64)),
        intended: vec![old.clone()],
        created: vec![old],
    };
    write_journal(&rig.data, &j).unwrap();
    assert_eq!(recover(&rig.data, &rig.site), Recovery::Kept);
    std::fs::read(rig.journal_path()).unwrap()
}

#[tokio::test]
async fn retry_after_failed_rollback_is_blocked() {
    let rig = rig();
    rig.available().await;
    let before = unresolvable_journal(&rig);
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed);
    assert_eq!(view.error.as_deref(), Some(INSTALL_KEY));
    assert!(rig.calls().is_empty(), "no download over an open journal");
    assert_eq!(std::fs::read(rig.journal_path()).unwrap(), before);
    let view = rig.state.cancel().await;
    assert_eq!(view.state, UpdateStatus::Failed);
    assert_eq!(std::fs::read(rig.journal_path()).unwrap(), before);
    // Try again is blocked the same way.
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed);
    assert!(rig.calls().is_empty());
    assert_eq!(std::fs::read(rig.journal_path()).unwrap(), before);
}

#[tokio::test]
async fn journal_written_before_first_download() {
    let rig = rig();
    rig.available().await;
    rig.install_and_settle().await;
    let calls = rig.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        let j = call
            .journal
            .as_ref()
            .expect("journal on disk before the download");
        assert_eq!(j.phase, Phase::Pending);
        assert_eq!(j.site, rig.site);
        assert!(
            j.scratch.iter().any(|s| call.dest.starts_with(s)),
            "{:?} not under {:?}",
            call.dest,
            j.scratch
        );
        assert!(j.intended.contains(&call.dest));
    }
}

#[tokio::test]
async fn cancel_after_download_removes_scratch_and_journal() {
    let rig = rig();
    rig.available().await;
    // The signature is complete on disk; the asset hangs.
    *rig.downloader.asset.lock().unwrap() = Mode::Hang;
    rig.state.install();
    rig.downloader.started.notified().await;
    let sig_dest = rig.calls()[0].dest.clone();
    assert!(sig_dest.exists());
    let scratch = read_journal(&rig.data).unwrap().unwrap().scratch;
    assert!(scratch.iter().all(|s| s.exists()));
    rig.state.cancel().await;
    assert!(!sig_dest.exists());
    assert!(scratch.iter().all(|s| !s.exists()));
    assert!(!rig.journal_path().exists());
    assert!(rig.site_leftovers().is_empty());
}

#[tokio::test]
async fn size_mismatch_is_failed() {
    let rig = rig();
    rig.available().await;
    *rig.downloader.asset.lock().unwrap() = Mode::Report(ASSET_BYTES.len() as u64 - 1);
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed);
    assert_eq!(view.error.as_deref(), Some(DOWNLOAD_KEY));
    assert!(rig.installer.installs.lock().unwrap().is_empty());
    assert!(rig.comments.lock().unwrap().is_empty(), "not even verified");
    assert!(!rig.journal_path().exists());
    assert!(rig.site_leftovers().is_empty());
}

#[tokio::test]
async fn unsupported_site_shows_download_only() {
    let rig = rig_with(
        FakeTransport::json(200, &release_json(ASSET_BYTES.len() as u64)),
        Some("folder not writable"),
    );
    let view = rig.state.check().await;
    assert_eq!(view.state, UpdateStatus::Unsupported);
    assert_eq!(view.version.as_deref(), Some(NEW));
    let view = rig.state.install();
    assert_eq!(view.state, UpdateStatus::Unsupported);
    rig.state.settle().await;
    assert!(rig.calls().is_empty());
    assert!(!rig.journal_path().exists());

    // Newer but without the target's assets: Download only as well.
    let rig = rig_with(
        FakeTransport::json(200, &format!(r#"{{"tag_name":"v{NEW}","assets":[]}}"#)),
        None,
    );
    let view = rig.state.check().await;
    assert_eq!(view.state, UpdateStatus::Unsupported);
    assert_eq!(view.version.as_deref(), Some(NEW));

    // Up to date: idle.
    let version = lingopop_core::VERSION;
    let rig = rig_with(
        FakeTransport::json(200, &format!(r#"{{"tag_name":"v{version}"}}"#)),
        None,
    );
    let view = rig.state.check().await;
    assert_eq!(view.state, UpdateStatus::Idle);
    assert_eq!(view.version, None);
}

#[tokio::test]
async fn relaunch_after_swap_uses_install_path() {
    // macOS: the enclosing bundle, through LaunchServices.
    let exe = Path::new("/Applications/LingoPop.app/Contents/MacOS/lingopop");
    let target = relaunch_target_for(exe, None, true);
    assert_eq!(
        target,
        RelaunchTarget::App(PathBuf::from("/Applications/LingoPop.app"))
    );
    let (program, args) = relaunch_command(&target, 42);
    assert_eq!(program, PathBuf::from("/usr/bin/open"));
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "-n",
            "/Applications/LingoPop.app",
            "--args",
            "--wait-for-pid",
            "42"
        ]
    );
    // A dev binary outside a bundle relaunches itself.
    let dev = Path::new("/tmp/target/debug/lingopop");
    assert_eq!(
        relaunch_target_for(dev, None, true),
        RelaunchTarget::Exe(dev.to_owned())
    );
    // Windows: the exe itself, never a bundle rule.
    let win = Path::new("C:/Apps/LingoPop.app/Contents/MacOS/lingopop");
    assert_eq!(
        relaunch_target_for(win, None, false),
        RelaunchTarget::Exe(win.to_owned())
    );
    let (program, args) = relaunch_command(&RelaunchTarget::Exe(win.to_owned()), 7);
    assert_eq!(program, win);
    assert_eq!(args, ["--wait-for-pid", "7"]);

    // A start from the backup whose recovery restored it: the target is the journal's site.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let token = "0123456789abcdef".repeat(2);
    let site = dir.path().join("LingoPop.exe");
    let old = dir.path().join(format!("LingoPop.old-{token}.exe"));
    std::fs::write(&old, b"previous").unwrap();
    let folder = dir.path().join(format!(".LingoPop-update-{token}"));
    let j = Journal {
        version: NEW.into(),
        kind: SiteKind::WindowsExe,
        token: token.clone(),
        site: site.clone(),
        staged: folder.join("LingoPop.exe"),
        old: old.clone(),
        scratch: vec![folder],
        phase: Phase::Pending,
        old_hash: file_sha256(&old).unwrap(),
        old_restored: false,
        new_hash: Some("b".repeat(64)),
        intended: vec![old.clone(), site.clone()],
        created: vec![old.clone()],
    };
    write_journal(&data, &j).unwrap();
    let (recovery, target) = startup_recovery(&data, &old);
    assert_eq!(recovery, Recovery::Restored);
    assert_eq!(target, RelaunchTarget::Exe(site.clone()));
    assert_eq!(std::fs::read(&site).unwrap(), b"previous");

    // The state machine's restart goes through the installer, never `current_exe()`.
    let rig = rig();
    rig.available().await;
    rig.install_and_settle().await;
    rig.state.restart().unwrap();
    assert_eq!(rig.installer.relaunches.load(Ordering::SeqCst), 1);
}

#[test]
fn startup_by_the_new_binary_restores_and_relaunches_the_install_site() {
    // The previous process quit after the swap, before the commit; the NEW exe starts at
    // `site`. Recovery moves its own image into the staging folder, so the journal and the
    // folder stay for the next start, and every relaunch targets the install site.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let token = "0123456789abcdef".repeat(2);
    let site = dir.path().join("LingoPop.exe");
    let old = dir.path().join(format!("LingoPop.old-{token}.exe"));
    let folder = dir.path().join(format!(".LingoPop-update-{token}"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join(lingopop_platform::update_install::OWNER_MARKER),
        &token,
    )
    .unwrap();
    std::fs::write(&site, b"new").unwrap();
    std::fs::write(&old, b"previous").unwrap();
    let staged = folder.join("LingoPop.exe");
    let j = Journal {
        version: NEW.into(),
        kind: SiteKind::WindowsExe,
        token: token.clone(),
        site: site.clone(),
        staged: staged.clone(),
        old: old.clone(),
        scratch: vec![folder.clone()],
        phase: Phase::Pending,
        old_hash: file_sha256(&old).unwrap(),
        old_restored: false,
        new_hash: Some(file_sha256(&site).unwrap()),
        intended: vec![folder.clone(), old.clone(), site.clone()],
        created: vec![folder.clone(), old.clone()],
    };
    write_journal(&data, &j).unwrap();
    let (recovery, target) = startup_recovery(&data, &site);
    assert_eq!(recovery, Recovery::RestoredKept);
    assert_eq!(target, RelaunchTarget::Exe(site.clone()));
    assert_eq!(std::fs::read(&site).unwrap(), b"previous");
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        b"new",
        "the running image is kept"
    );
    assert!(read_journal(&data).unwrap().is_some());
}

#[tokio::test]
async fn state_read_returns_ready_after_settings_reopen() {
    let rig = rig();
    rig.available().await;
    let ready = rig.install_and_settle().await;
    assert_eq!(ready.state, UpdateStatus::Ready);
    let emitted = rig.emitted.lock().unwrap().len();
    // A reopened Settings window reads the state: unchanged, nothing emitted.
    for _ in 0..3 {
        assert_eq!(rig.state.current(), ready);
    }
    assert_eq!(rig.emitted.lock().unwrap().len(), emitted);
    // A check while Ready keeps Ready and asks nothing.
    let requests_before = rig.transport.requests().len();
    assert_eq!(rig.state.check().await.state, UpdateStatus::Ready);
    assert_eq!(rig.transport.requests().len(), requests_before);
    // Neither install nor cancel leaves Ready.
    assert_eq!(rig.state.install().state, UpdateStatus::Ready);
    assert_eq!(rig.state.cancel().await.state, UpdateStatus::Ready);
}

#[tokio::test]
async fn revision_grows_on_every_change() {
    let rig = rig();
    let start = rig.state.current().revision;
    rig.available().await;
    rig.install_and_settle().await;
    let emitted = rig.emitted.lock().unwrap().clone();
    assert!(emitted.len() >= 4);
    let mut last = start;
    for view in &emitted {
        assert!(view.revision > last, "{emitted:?}");
        last = view.revision;
    }
    assert_eq!(rig.state.current().revision, last);
    // A read alone changes nothing.
    assert_eq!(rig.state.current().revision, last);
}

#[tokio::test]
async fn progress_events_are_throttled() {
    let rig = rig();
    rig.available().await;
    // 200 progress reports over one virtual second (5 ms apart).
    *rig.downloader.asset.lock().unwrap() = Mode::Chatty(200);
    let start = rig.clock_seconds();
    rig.install_and_settle().await;
    let elapsed = rig.clock_seconds() - start;
    let progress = rig
        .emitted
        .lock()
        .unwrap()
        .iter()
        .filter(|v| v.state == UpdateStatus::Downloading && v.received.unwrap_or(0) > 0)
        .count();
    assert!((0.99..1.01).contains(&elapsed), "{elapsed}");
    assert!(progress >= 5, "progress is still reported: {progress}");
    assert!(progress <= 11, "at most 10 per second: {progress}");
}

impl Rig {
    fn clock_seconds(&self) -> f64 {
        use lingopop_core::time::Clock;
        self.clock.now().unix_seconds()
    }
}

// Fix round 1.

#[tokio::test]
async fn commit_failure_restores_the_previous_version_at_once() {
    let rig = rig();
    rig.available().await;
    *rig.installer.mode.lock().unwrap() = InstallMode::SwapThenCommitFails;
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed, "{view:?}");
    assert_eq!(view.error.as_deref(), Some(INSTALL_KEY));
    assert_eq!(rig.installer.installs.lock().unwrap().len(), 1);
    // The swap happened and was undone in the same job: the old version is back now.
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
    assert!(!rig.journal_path().exists());
    assert!(
        rig.site_leftovers().is_empty(),
        "{:?}",
        rig.site_leftovers()
    );
}

#[tokio::test]
async fn commit_error_with_the_commit_on_record_is_ready_and_cleans_nothing() {
    let rig = rig();
    rig.available().await;
    *rig.installer.mode.lock().unwrap() = InstallMode::SwapThenCommitOnRecordFails;
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Ready, "{view:?}");
    // The new version is installed; this process's own image (the backup) is kept and the
    // committed journal is closed by the next start.
    assert_ne!(std::fs::read(&rig.site).unwrap(), b"old exe");
    assert!(rig.journal_path().exists());
    let journal = read_journal(&rig.data).unwrap().unwrap();
    assert_eq!(std::fs::read(&journal.old).unwrap(), b"old exe");
}

#[tokio::test]
async fn panic_in_the_install_job_is_failed_and_cleaned() {
    let rig = rig();
    rig.available().await;
    *rig.installer.mode.lock().unwrap() = InstallMode::Panic;
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed, "{view:?}");
    assert_eq!(view.error.as_deref(), Some(INSTALL_KEY));
    assert!(!rig.journal_path().exists());
    assert!(
        rig.site_leftovers().is_empty(),
        "{:?}",
        rig.site_leftovers()
    );
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
    // Not stuck: a retry from Failed works.
    *rig.installer.mode.lock().unwrap() = InstallMode::Copy;
    assert_eq!(rig.install_and_settle().await.state, UpdateStatus::Ready);
}

#[tokio::test]
async fn panic_in_the_attempt_task_is_failed_not_stuck_downloading() {
    let rig = rig();
    rig.available().await;
    *rig.downloader.asset.lock().unwrap() = Mode::Panic;
    let view = rig.install_and_settle().await;
    assert_eq!(view.state, UpdateStatus::Failed, "{view:?}");
    assert_eq!(view.error.as_deref(), Some(INSTALL_KEY));
    assert!(rig.installer.installs.lock().unwrap().is_empty());
    assert!(!rig.journal_path().exists());
    assert!(
        rig.site_leftovers().is_empty(),
        "{:?}",
        rig.site_leftovers()
    );
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
    // The machine accepts a new attempt.
    *rig.downloader.asset.lock().unwrap() = Mode::Serve;
    assert_eq!(rig.install_and_settle().await.state, UpdateStatus::Ready);
}

#[tokio::test]
async fn cancel_during_installing_is_ignored() {
    let rig = rig();
    rig.available().await;
    let release = rig.installer.gate.arm();
    rig.state.install();
    rig.installer.gate.entered.notified().await;
    assert_eq!(rig.state.current().state, UpdateStatus::Installing);
    let view = rig.state.cancel().await;
    assert_eq!(view.state, UpdateStatus::Installing, "{view:?}");
    release.send(()).unwrap();
    rig.state.settle().await;
    let view = rig.state.current();
    assert_eq!(view.state, UpdateStatus::Ready, "{view:?}");
    assert_eq!(std::fs::read(&rig.site).unwrap(), ASSET_BYTES);
}

#[tokio::test]
async fn cancel_during_verify_cleans_up_and_never_installs() {
    let rig = rig();
    rig.available().await;
    let release = rig.verify_gate.arm();
    rig.state.install();
    rig.verify_gate.entered.notified().await;
    assert_eq!(rig.state.current().state, UpdateStatus::Downloading);
    // `join!` polls the cancel first: its first poll sets the flag, then the verify passes.
    let (view, ()) = tokio::join!(rig.state.cancel(), async {
        release.send(()).unwrap();
    });
    assert_eq!(view.state, UpdateStatus::Available, "{view:?}");
    assert!(rig.installer.installs.lock().unwrap().is_empty());
    assert!(!rig.journal_path().exists());
    assert!(
        rig.site_leftovers().is_empty(),
        "{:?}",
        rig.site_leftovers()
    );
    assert_eq!(std::fs::read(&rig.site).unwrap(), b"old exe");
}

#[tokio::test]
async fn progress_after_the_clock_steps_back_is_still_reported() {
    let rig = rig();
    rig.available().await;
    *rig.downloader.asset.lock().unwrap() = Mode::Backwards;
    rig.install_and_settle().await;
    let received: Vec<u64> = rig
        .emitted
        .lock()
        .unwrap()
        .iter()
        .filter(|v| v.state == UpdateStatus::Downloading)
        .filter_map(|v| v.received)
        .filter(|r| *r > 0)
        .collect();
    assert_eq!(received, vec![1, 2]);
}
