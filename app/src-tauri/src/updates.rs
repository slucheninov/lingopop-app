//! GitHub release check and the in-app update (spec "In-app updates" sections 3, 5 and 6).
//!
//! The check is silent: everything is injected (transport, clock, deadline), so tests use
//! `FakeTransport` and `ManualClock` and never the network or a runtime timer. The update state
//! machine (`Idle -> Checking -> Available -> Downloading -> Installing -> Ready | Failed`, plus
//! `Unsupported` = Download only) runs one attempt at a time; the downloader, the signature
//! check, the installer and the event sink are injected too ([`UpdaterDeps`]).
use std::ffi::OsString;
use std::future::{poll_fn, Future};
use std::path::{Path, PathBuf};
use std::pin::{pin, Pin};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::task::Poll;

use lingopop_core::http::{Transport, UpdateDownloader};
use lingopop_core::time::Clock;
use lingopop_core::update_sig::UpdateError;
use lingopop_core::updates::{
    fetch_release_with, is_newer, should_check, update_plan, Release, UpdatePlan, UpdateTarget,
    UPDATE_REQUEST_TIMEOUT,
};
use lingopop_core::VERSION;
use lingopop_platform::update_install::{
    bundle_zip_path, commit_journal, confirm, intend, read_journal, recover, stage_bundle,
    stage_exe, InstallSite, Journal, Recovery, SiteKind, Support,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::UiState;

type Deadline = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Receives every state change (production: the `updates://state` event).
pub type Emit = Arc<dyn Fn(&UpdateView) + Send + Sync>;
/// `verify_update(bytes, signature text, expected trusted comment)`; tests inject the test key.
pub type Verify = Arc<dyn Fn(&[u8], &str, &str) -> Result<(), UpdateError> + Send + Sync>;

/// The event carrying every [`UpdateView`].
pub const STATE_EVENT: &str = "updates://state";
/// At most this many progress events per second.
const PROGRESS_INTERVAL_SECONDS: f64 = 0.1;
/// The largest `.minisig` the updater downloads (the plan rule).
const MAX_SIGNATURE_BYTES: u64 = 4096;
/// The relaunch error, the mac catalog key.
const RELAUNCH_FAILED: &str = "Could not relaunch LingoPop";

/// Streams one update file into a NEW file (`create_new`); dropping the future removes it.
#[async_trait::async_trait]
pub trait Download: Send + Sync {
    async fn download(
        &self,
        url: &str,
        max_bytes: u64,
        dest: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, UpdateError>;
}

#[async_trait::async_trait]
impl Download for UpdateDownloader {
    async fn download(
        &self,
        url: &str,
        max_bytes: u64,
        dest: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, UpdateError> {
        UpdateDownloader::download(self, url, max_bytes, dest, progress).await
    }
}

/// The OS side of an update: whether the running app may update itself, the install of a
/// verified download, and the relaunch of the install path recorded at startup.
pub trait Installer: Send + Sync {
    /// Evaluated once (it may run `codesign`); call it off the async threads.
    fn support(&self) -> &Support;
    /// Swaps the verified download in. On `Ok` the journal is still `pending`; the caller
    /// commits it.
    fn install(&self, plan: &UpdatePlan, downloaded: &Path) -> Result<(), UpdateError>;
    /// Commits the journal after a successful install (`commit_journal`; a seam for tests).
    fn commit(&self, data_dir: &Path) -> Result<(), UpdateError> {
        commit_journal(data_dir)
    }
    /// Starts the installed app with `--wait-for-pid <this pid>`; the caller then exits.
    fn relaunch(&self) -> std::io::Result<()>;
}

/// What the update machine needs besides the release check.
pub struct UpdaterDeps {
    pub data_dir: PathBuf,
    /// The running exe (a path inside the running bundle on macOS): recovery never deletes it.
    pub running: PathBuf,
    pub target: Option<UpdateTarget>,
    pub installer: Arc<dyn Installer>,
    pub downloader: Arc<dyn Download>,
    pub verify: Verify,
    pub emit: Emit,
}

/// The wire state name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
    Ready,
    Failed,
    Unsupported,
}

/// `updates://state` payload and the answer of every update command. `error` is an English
/// catalog key; `revision` grows with every state change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    pub state: UpdateStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone)]
enum Phase {
    Idle,
    Checking,
    Available(UpdatePlan),
    /// The signature and asset downloads (and the staging before them).
    Downloading {
        plan: UpdatePlan,
        received: u64,
    },
    /// Verifying and installing: a cancel is ignored from here on.
    Installing(UpdatePlan),
    Ready(String),
    Failed {
        plan: UpdatePlan,
        error: &'static str,
    },
    Unsupported(String),
}

struct Machine {
    phase: Phase,
    revision: u64,
    /// Set by `cancel` under this lock while Downloading; checked before Installing.
    cancelled: bool,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    last_progress: Option<f64>,
}

impl Machine {
    fn view(&self) -> UpdateView {
        let mut view = UpdateView {
            state: UpdateStatus::Idle,
            version: None,
            received: None,
            total: None,
            error: None,
            revision: self.revision,
        };
        match &self.phase {
            Phase::Idle => {}
            Phase::Checking => view.state = UpdateStatus::Checking,
            Phase::Available(plan) => {
                view.state = UpdateStatus::Available;
                view.version = Some(plan.version.clone());
            }
            Phase::Downloading { plan, received } => {
                view.state = UpdateStatus::Downloading;
                view.version = Some(plan.version.clone());
                view.received = Some(*received);
                view.total = Some(plan.asset_size);
            }
            Phase::Installing(plan) => {
                view.state = UpdateStatus::Installing;
                view.version = Some(plan.version.clone());
            }
            Phase::Ready(version) => {
                view.state = UpdateStatus::Ready;
                view.version = Some(version.clone());
            }
            Phase::Failed { plan, error } => {
                view.state = UpdateStatus::Failed;
                view.version = Some(plan.version.clone());
                view.error = Some((*error).to_owned());
            }
            Phase::Unsupported(version) => {
                view.state = UpdateStatus::Unsupported;
                view.version = Some(version.clone());
            }
        }
        view
    }
}

/// Why an attempt stopped.
enum Stop {
    Cancelled,
    Failed(UpdateError),
}

impl From<UpdateError> for Stop {
    fn from(e: UpdateError) -> Self {
        Stop::Failed(e)
    }
}

pub struct UpdateState {
    transport: Arc<dyn Transport>,
    ui: Arc<UiState>,
    clock: Arc<dyn Clock>,
    deadline: Deadline,
    updater: Option<UpdaterDeps>,
    machine: Mutex<Machine>,
}

impl UpdateState {
    pub fn new(transport: Arc<dyn Transport>, ui: Arc<UiState>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            ui,
            clock,
            deadline: Arc::new(|| Box::pin(tokio::time::sleep(UPDATE_REQUEST_TIMEOUT))),
            updater: None,
            machine: Mutex::new(Machine {
                phase: Phase::Idle,
                revision: 0,
                cancelled: false,
                cancel: None,
                task: None,
                last_progress: None,
            }),
        }
    }

    /// Enables the in-app update. Without it a newer release is always Download only.
    pub fn with_updater(mut self, deps: UpdaterDeps) -> Self {
        self.updater = Some(deps);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_deadline(mut self, deadline: Deadline) -> Self {
        self.deadline = deadline;
        self
    }

    /// One GitHub lookup; a successful one is cached in `ui.json` (latest tag, check time).
    async fn fetch_and_cache(&self) -> Option<Release> {
        let release = fetch_release_with(self.transport.as_ref(), (self.deadline)()).await?;
        let checked = self.clock.now().unix_seconds();
        let cached = release.version.clone();
        if self
            .ui
            .update(|p| {
                p.latest_version = Some(cached);
                p.last_update_check = Some(checked);
            })
            .is_err()
        {
            log::warn!("update check cache could not be saved");
        }
        Some(release)
    }

    /// Always asks GitHub (the cache rule is for startup only). Silent on every failure.
    /// Returns the tag only when it is newer than this build; a successful lookup is cached
    /// either way, so the next startup within three hours needs no request.
    pub async fn check_now(&self) -> Option<String> {
        let latest = self.fetch_and_cache().await?.version;
        is_newer(&latest, VERSION).then_some(latest)
    }

    /// Within the check interval of the last lookup, answers from the cache without a request.
    pub async fn check_at_startup(&self) -> Option<String> {
        let prefs = self.ui.prefs();
        if should_check(prefs.last_update_check, self.clock.now().unix_seconds()) {
            return self.check_now().await;
        }
        prefs.latest_version.filter(|v| is_newer(v, VERSION))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Machine> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, view: &UpdateView) {
        if let Some(deps) = &self.updater {
            (deps.emit)(view);
        }
    }

    /// Applies `f` when it returns true: one state change, one revision, one event (sent after
    /// the lock is released; the revision orders events that race).
    fn change_if(&self, f: impl FnOnce(&mut Machine) -> bool) -> UpdateView {
        let (view, changed) = {
            let mut m = self.lock();
            let changed = f(&mut m);
            if changed {
                m.revision += 1;
            }
            (m.view(), changed)
        };
        if changed {
            self.emit(&view);
        }
        view
    }

    fn set(&self, phase: Phase) -> UpdateView {
        self.change_if(|m| {
            m.phase = phase;
            true
        })
    }

    /// The current state; changes nothing.
    pub fn current(&self) -> UpdateView {
        self.lock().view()
    }

    /// `updates_check`: one lookup (cached like [`Self::check_now`]), then Available (a plan
    /// and a supported install site), Unsupported (newer, Download only) or Idle (up to date
    /// or the lookup failed, silently). Busy, Ready: the current state, no request.
    pub async fn check(self: &Arc<Self>) -> UpdateView {
        let mut busy = None;
        self.change_if(|m| match m.phase {
            Phase::Checking
            | Phase::Downloading { .. }
            | Phase::Installing(_)
            | Phase::Ready(_) => {
                busy = Some(m.view());
                false
            }
            _ => {
                m.phase = Phase::Checking;
                true
            }
        });
        if let Some(view) = busy {
            return view;
        }
        let phase = match self.fetch_and_cache().await {
            Some(release) if is_newer(&release.version, VERSION) => {
                let plan = self
                    .updater
                    .as_ref()
                    .and_then(|deps| deps.target)
                    .and_then(|target| update_plan(&release, VERSION, target));
                match plan {
                    Some(plan) if self.supported().await => Phase::Available(plan),
                    _ => Phase::Unsupported(release.version),
                }
            }
            _ => Phase::Idle,
        };
        self.set(phase)
    }

    async fn supported(&self) -> bool {
        let Some(deps) = &self.updater else {
            return false;
        };
        let installer = deps.installer.clone();
        blocking(move || matches!(installer.support(), Support::Supported(_)))
            .await
            .unwrap_or(false)
    }

    /// `updates_install`: from Available (or Try again from Failed) starts one attempt in the
    /// background and returns Downloading; otherwise (already running, Ready, ...) returns the
    /// current state and starts nothing.
    pub fn install(self: &Arc<Self>) -> UpdateView {
        if self.updater.is_none() {
            return self.current();
        }
        let mut started = None;
        let view = self.change_if(|m| {
            let plan = match &m.phase {
                Phase::Available(plan) | Phase::Failed { plan, .. } => plan.clone(),
                _ => return false,
            };
            let (tx, rx) = tokio::sync::watch::channel(false);
            m.phase = Phase::Downloading {
                plan: plan.clone(),
                received: 0,
            };
            m.cancelled = false;
            m.cancel = Some(tx);
            m.last_progress = None;
            started = Some((plan, rx));
            true
        });
        if let Some((plan, rx)) = started {
            let this = Arc::clone(self);
            let handle = tauri::async_runtime::spawn(async move { this.run(plan, rx).await });
            let mut m = self.lock();
            // The task may already be done (it then left Downloading); its handle is harmless.
            m.task = Some(handle);
        }
        view
    }

    /// `updates_cancel`: while Downloading, drops the download, removes the attempt's files
    /// and journal and returns Available. Anything else (installing included) is ignored.
    pub async fn cancel(self: &Arc<Self>) -> UpdateView {
        let handle = {
            let mut m = self.lock();
            if !matches!(m.phase, Phase::Downloading { .. }) {
                return m.view();
            }
            m.cancelled = true;
            if let Some(tx) = &m.cancel {
                let _ = tx.send(true);
            }
            m.task.take()
        };
        if let Some(handle) = handle {
            let _ = handle.await;
        }
        self.current()
    }

    /// Waits for a running attempt (tests).
    #[cfg(test)]
    pub(crate) async fn settle(&self) {
        let handle = self.lock().task.take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    /// `updates_restart` / `app_relaunch`: starts the install path recorded at startup.
    pub fn restart(&self) -> Result<(), String> {
        let deps = self.updater.as_ref().ok_or(RELAUNCH_FAILED.to_owned())?;
        deps.installer.relaunch().map_err(|_| {
            log::warn!("relaunch failed");
            RELAUNCH_FAILED.to_owned()
        })
    }

    async fn run(self: Arc<Self>, plan: UpdatePlan, cancel: tokio::sync::watch::Receiver<bool>) {
        // The attempt runs as its own task so a panic in it ends as Failed (install key) with
        // the attempt cleaned up through recovery, never as a phase stuck in Downloading.
        let this = Arc::clone(&self);
        let attempted = plan.clone();
        let outcome = match tauri::async_runtime::spawn(async move {
            this.attempt(&attempted, cancel).await
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                log::warn!("update attempt panicked");
                self.cleanup().await;
                Err(UpdateError::Install("update worker failed").into())
            }
        };
        let phase = match outcome {
            Ok(()) => {
                log::info!("update installed");
                Phase::Ready(plan.version)
            }
            Err(Stop::Cancelled) => {
                log::info!("update cancelled");
                Phase::Available(plan)
            }
            Err(Stop::Failed(error)) => {
                log::warn!("update failed: {error}");
                Phase::Failed {
                    plan,
                    error: error.catalog_key(),
                }
            }
        };
        self.change_if(|m| {
            m.phase = phase;
            m.cancel = None;
            m.cancelled = false;
            true
        });
    }

    fn deps(&self) -> &UpdaterDeps {
        self.updater
            .as_ref()
            .expect("an attempt starts only with an updater")
    }

    fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// One attempt. Spec section 3: recovery first and no new attempt over an open journal;
    /// the journal is written (by the staging) before the first download; signature, then
    /// asset, both inside the owned staging folder; size and signature checks; install;
    /// commit. A failure or cancel before the install removes the attempt's files and journal
    /// through recovery (the item at `site` still has `oldHash`), never by name.
    async fn attempt(
        &self,
        plan: &UpdatePlan,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), Stop> {
        let deps = self.deps();
        let (data, running) = (deps.data_dir.clone(), deps.running.clone());
        let installer = deps.installer.clone();
        let (open, support) = blocking(move || {
            let _ = recover(&data, &running);
            let open = !matches!(read_journal(&data), Ok(None));
            (open, installer.support().clone())
        })
        .await?;
        if open {
            // Nothing is written and the open journal is left for the next start.
            return Err(UpdateError::Install("an update journal is open").into());
        }
        let site = match support {
            Support::Supported(site) => site,
            Support::Unsupported(why) => return Err(UpdateError::Unsupported(why).into()),
        };
        if self.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let data = deps.data_dir.clone();
        let version = plan.version.clone();
        let staged_site = site.clone();
        let journal = blocking(move || stage(&data, &staged_site, &version)).await??;
        let asset = match self.fetch_and_verify(plan, journal, &mut cancel).await {
            Ok(asset) => asset,
            Err(stop) => {
                self.cleanup().await;
                return Err(stop);
            }
        };
        let entered = self.change_if(|m| {
            if m.cancelled {
                return false;
            }
            m.phase = Phase::Installing(plan.clone());
            true
        });
        if entered.state != UpdateStatus::Installing {
            self.cleanup().await;
            return Err(Stop::Cancelled);
        }
        let (installer, data, plan) = (deps.installer.clone(), deps.data_dir.clone(), plan.clone());
        let running = deps.running.clone();
        // After an install error the attempt is either cleaned already or the journal stays
        // for the next start's recovery (a failed rollback): nothing more to do here. A commit
        // failure after a successful swap is recovered at once (the pending new version is
        // swapped back), so "Your current version is unchanged" is true when it is shown.
        let installed = blocking(move || {
            installer.install(&plan, &asset)?;
            if let Err(error) = installer.commit(&data) {
                log::warn!("update commit failed, restoring the previous version");
                if !matches!(recover(&data, &running), Recovery::Restored) {
                    log::warn!("update restore after a failed commit did not complete");
                }
                return Err(error);
            }
            Ok(())
        })
        .await;
        match installed {
            Ok(result) => result?,
            Err(error) => {
                // The install job panicked: its state is unknown, recovery decides.
                log::warn!("update install panicked");
                self.cleanup().await;
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// Downloads the signature, then the asset, into the owned staging folder (each path in
    /// `intended` before and `created` after), checks the size and verifies the signature.
    /// Returns the asset path.
    async fn fetch_and_verify(
        &self,
        plan: &UpdatePlan,
        mut journal: Journal,
        cancel: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let deps = self.deps();
        let data = deps.data_dir.as_path();
        let (sig_path, asset_path) = download_paths(&journal, plan)?;

        intend(data, &mut journal, &sig_path)?;
        let none = |_: u64, _: Option<u64>| {};
        race(
            deps.downloader
                .download(&plan.signature_url, MAX_SIGNATURE_BYTES, &sig_path, &none),
            cancel,
        )
        .await??;
        confirm(data, &mut journal, &sig_path)?;
        if self.is_cancelled() {
            return Err(Stop::Cancelled);
        }

        intend(data, &mut journal, &asset_path)?;
        let progress = |received: u64, _: Option<u64>| self.progress(received);
        let received = race(
            deps.downloader
                .download(&plan.asset_url, plan.asset_size, &asset_path, &progress),
            cancel,
        )
        .await??;
        confirm(data, &mut journal, &asset_path)?;
        if received != plan.asset_size {
            return Err(UpdateError::Download.into());
        }
        if self.is_cancelled() {
            return Err(Stop::Cancelled);
        }

        let verify = deps.verify.clone();
        let comment = plan.expected_comment.clone();
        let asset = asset_path.clone();
        blocking(move || {
            let signature =
                std::fs::read_to_string(&sig_path).map_err(|_| UpdateError::Signature)?;
            let bytes = std::fs::read(&asset).map_err(|_| UpdateError::Download)?;
            verify(&bytes, &signature, &comment)
        })
        .await??;
        Ok(asset_path)
    }

    /// Throttled progress: at most one state change per [`PROGRESS_INTERVAL_SECONDS`].
    fn progress(&self, value: u64) {
        let now = self.clock.now().unix_seconds();
        self.change_if(|m| {
            let Phase::Downloading { received, .. } = &mut m.phase else {
                return false;
            };
            // A clock that stepped backwards (`now < last`) counts as elapsed, so progress
            // never stalls until wall time catches up again.
            if m.last_progress
                .is_some_and(|last| now >= last && now - last < PROGRESS_INTERVAL_SECONDS)
            {
                return false;
            }
            *received = value;
            m.last_progress = Some(now);
            true
        });
    }

    /// Removes a not-installed attempt (owned folder, created files, journal) through the
    /// recovery rules.
    async fn cleanup(&self) {
        let deps = self.deps();
        let (data, running) = (deps.data_dir.clone(), deps.running.clone());
        if blocking(move || recover(&data, &running)).await.is_err() {
            log::warn!("update cleanup failed");
        }
    }
}

/// The staging of the site's kind: Windows `stage_exe`, macOS `stage_bundle`.
fn stage(data: &Path, site: &InstallSite, version: &str) -> Result<Journal, UpdateError> {
    match site.kind {
        SiteKind::WindowsExe => stage_exe(data, site, version),
        SiteKind::MacBundle => stage_bundle(data, site, version),
    }
}

/// `(signature, asset)` download paths inside the attempt's owned staging folder: Windows the
/// staged exe itself, macOS `download.zip`; the signature next to it.
fn download_paths(j: &Journal, plan: &UpdatePlan) -> Result<(PathBuf, PathBuf), UpdateError> {
    let asset = match j.kind {
        SiteKind::WindowsExe => j.staged.clone(),
        SiteKind::MacBundle => bundle_zip_path(j),
    };
    let folder = asset
        .parent()
        .ok_or(UpdateError::Install("no staging folder"))?;
    let signature = folder.join(format!("{}.minisig", plan.asset_name));
    if signature == asset || !j.scratch.iter().any(|s| asset.starts_with(s)) {
        return Err(UpdateError::Install("no staging folder"));
    }
    Ok((signature, asset))
}

/// Runs blocking file / tool work off the async threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, UpdateError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| UpdateError::Install("update worker failed"))
}

/// `fut`, unless a cancel arrives first (the future is then dropped, removing a partial file).
async fn race<T>(
    fut: impl Future<Output = T>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<T, Stop> {
    let cancelled = async {
        if cancel.wait_for(|c| *c).await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let mut fut = pin!(fut);
    let mut cancelled = pin!(cancelled);
    poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Stop::Cancelled));
        }
        fut.as_mut().poll(cx).map(Ok)
    })
    .await
}

/// The update target of this build, by `cfg(target_os, target_arch)`.
pub fn native_target() -> Option<UpdateTarget> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(UpdateTarget::WindowsX64)
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(UpdateTarget::MacosArm64)
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(UpdateTarget::MacosX64)
    } else {
        None
    }
}

/// What a relaunch starts: an exe directly, or an `.app` through LaunchServices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaunchTarget {
    Exe(PathBuf),
    App(PathBuf),
}

/// `<x>.app` when `exe` is `<x>.app/Contents/MacOS/<name>`.
fn enclosing_app(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_app = app
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"));
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app)
        .then(|| app.to_owned())
}

/// The install path every relaunch uses, computed once at startup. After a recovery that
/// restored the previous version it is the journal's `site` (the running copy may be the
/// backup); otherwise the running exe, or on macOS its enclosing bundle.
pub fn relaunch_target_for(exe: &Path, restored_site: Option<&Path>, mac: bool) -> RelaunchTarget {
    let path = restored_site.unwrap_or(exe);
    if mac {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            return RelaunchTarget::App(path.to_owned());
        }
        if let Some(app) = enclosing_app(path) {
            return RelaunchTarget::App(app);
        }
    }
    RelaunchTarget::Exe(path.to_owned())
}

/// The relaunch command: `<exe> --wait-for-pid <pid>`, or
/// `/usr/bin/open -n <app> --args --wait-for-pid <pid>`.
pub fn relaunch_command(target: &RelaunchTarget, pid: u32) -> (PathBuf, Vec<OsString>) {
    let wait = [
        OsString::from("--wait-for-pid"),
        OsString::from(pid.to_string()),
    ];
    match target {
        RelaunchTarget::Exe(exe) => (exe.clone(), wait.to_vec()),
        RelaunchTarget::App(app) => {
            let mut args = vec![
                OsString::from("-n"),
                app.clone().into_os_string(),
                OsString::from("--args"),
            ];
            args.extend(wait);
            (PathBuf::from("/usr/bin/open"), args)
        }
    }
}

/// Startup: recovery of a previous attempt (before the install site is computed), then the
/// relaunch target. The outcome is logged with fixed text.
pub fn startup_recovery(data_dir: &Path, exe: &Path) -> (Recovery, RelaunchTarget) {
    let journal_site = read_journal(data_dir).ok().flatten().map(|j| j.site);
    let recovery = recover(data_dir, exe);
    log::info!(
        "{}",
        match recovery {
            Recovery::Nothing => "update recovery: nothing to do",
            Recovery::Cleaned => "update recovery: cleaned up",
            Recovery::Restored => "update recovery: previous version restored",
            Recovery::Kept => "update recovery: journal kept",
        }
    );
    let restored = journal_site.filter(|_| recovery == Recovery::Restored);
    let target = relaunch_target_for(exe, restored.as_deref(), cfg!(target_os = "macos"));
    (recovery, target)
}

/// The production [`Installer`]: Windows `exe_support` / `install_exe`, macOS `bundle_support`
/// / `install_bundle` over `SystemBundleTools`; debug builds never self-update.
pub struct NativeInstaller {
    data_dir: PathBuf,
    target: Option<RelaunchTarget>,
    support: OnceLock<Support>,
}

impl NativeInstaller {
    /// `target` = the startup relaunch target; `None` when the running exe is unknown.
    pub fn new(data_dir: PathBuf, target: Option<RelaunchTarget>) -> Self {
        Self {
            data_dir,
            target,
            support: OnceLock::new(),
        }
    }

    fn evaluate(&self) -> Support {
        if cfg!(debug_assertions) {
            return Support::Unsupported("debug build");
        }
        match &self.target {
            None => Support::Unsupported("running app unknown"),
            Some(target) => native_support(target),
        }
    }
}

#[cfg(windows)]
fn native_support(target: &RelaunchTarget) -> Support {
    match target {
        RelaunchTarget::Exe(exe) => lingopop_platform::update_install::exe_support(exe),
        RelaunchTarget::App(_) => Support::Unsupported("not an exe file"),
    }
}

#[cfg(target_os = "macos")]
fn native_support(target: &RelaunchTarget) -> Support {
    match target {
        RelaunchTarget::App(app) => lingopop_platform::update_install::bundle_support(
            app,
            &lingopop_platform::SystemBundleTools,
        ),
        RelaunchTarget::Exe(_) => Support::Unsupported("not an app bundle"),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn native_support(_: &RelaunchTarget) -> Support {
    Support::Unsupported("not a supported OS")
}

#[cfg(target_os = "macos")]
fn install_bundle_native(
    data_dir: &Path,
    site: &InstallSite,
    plan: &UpdatePlan,
    zip: &Path,
) -> Result<(), UpdateError> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    lingopop_platform::update_install::install_bundle(
        data_dir,
        site,
        zip,
        &plan.version,
        VERSION,
        arch,
        &lingopop_platform::SystemBundleTools,
    )
}

#[cfg(not(target_os = "macos"))]
fn install_bundle_native(
    _: &Path,
    _: &InstallSite,
    _: &UpdatePlan,
    _: &Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::Unsupported("not a macOS build"))
}

impl Installer for NativeInstaller {
    fn support(&self) -> &Support {
        self.support.get_or_init(|| self.evaluate())
    }

    fn install(&self, plan: &UpdatePlan, downloaded: &Path) -> Result<(), UpdateError> {
        let site = match self.support() {
            Support::Supported(site) => site,
            Support::Unsupported(why) => return Err(UpdateError::Unsupported(why)),
        };
        match site.kind {
            SiteKind::WindowsExe => {
                lingopop_platform::update_install::install_exe(&self.data_dir, site, downloaded)
            }
            SiteKind::MacBundle => install_bundle_native(&self.data_dir, site, plan, downloaded),
        }
    }

    fn relaunch(&self) -> std::io::Result<()> {
        let Some(target) = &self.target else {
            return Err(std::io::Error::other("running app unknown"));
        };
        let (program, args) = relaunch_command(target, std::process::id());
        let mut command = std::process::Command::new(program);
        command.args(args);
        #[cfg(windows)]
        detach_windows(&mut command);
        command.spawn().map(|_| ())
    }
}

#[cfg(windows)]
fn detach_windows(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // `CREATE_NEW_PROCESS_GROUP` (0x200) and `DETACHED_PROCESS` (0x8),
    // `windows` 0.62.2 `Win32_System_Threading`.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

/// Startup lookup, then one lookup per interval. The first interval tick fires at once and is
/// skipped. Each newer tag only signals `settings://changed`: the webview re-reads the view.
pub fn spawn_loop(state: Arc<UpdateState>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let signal = || {
            let _ = app.emit("settings://changed", ());
        };
        if state.check_at_startup().await.is_some() {
            signal();
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs_f64(
            lingopop_core::updates::CHECK_INTERVAL_SECONDS,
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            if state.check_now().await.is_some() {
                signal();
            }
        }
    });
}

/// Checks for a release. A newer one also signals `settings://changed` (the banner), as before.
#[tauri::command]
pub async fn updates_check(
    app: AppHandle,
    state: State<'_, Arc<UpdateState>>,
) -> Result<UpdateView, String> {
    let view = state.inner().check().await;
    if matches!(
        view.state,
        UpdateStatus::Available | UpdateStatus::Unsupported
    ) {
        let _ = app.emit("settings://changed", ());
    }
    Ok(view)
}

#[tauri::command]
pub async fn updates_install(state: State<'_, Arc<UpdateState>>) -> Result<UpdateView, String> {
    Ok(state.inner().install())
}

#[tauri::command]
pub async fn updates_cancel(state: State<'_, Arc<UpdateState>>) -> Result<UpdateView, String> {
    Ok(state.inner().cancel().await)
}

/// A read of the current state; changes nothing.
#[tauri::command]
pub async fn updates_state(state: State<'_, Arc<UpdateState>>) -> Result<UpdateView, String> {
    Ok(state.current())
}

/// Starts the installed app at the path recorded at startup and exits.
#[tauri::command]
pub async fn updates_restart(
    app: AppHandle,
    state: State<'_, Arc<UpdateState>>,
) -> Result<(), String> {
    state.restart()?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_cmds::{settings_view_parts, url_allowed};
    use crate::state::ShellConfig;
    use lingopop_core::errors::TranslationError;
    use lingopop_core::http::{FakeTransport, HttpRequest, HttpResponse};
    use lingopop_core::paths::DataPaths;
    use lingopop_core::time::{ManualClock, Timestamp};
    use lingopop_core::ui_prefs::UiPrefs;
    use lingopop_core::updates::{CHECK_INTERVAL_SECONDS, RELEASES_PAGE_URL};

    const NOW: f64 = 1_800_000_000.0;

    struct Rig {
        dir: tempfile::TempDir,
        ui: Arc<UiState>,
        config: Arc<ShellConfig>,
        clock: Arc<ManualClock>,
        fake: Arc<FakeTransport>,
        state: UpdateState,
    }

    fn rig_with(fake: FakeTransport, deadline: Deadline) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (config, _stores, ui) = crate::task4_tests::rig(dir.path());
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(NOW)));
        let fake = Arc::new(fake);
        let mut state = UpdateState::new(fake.clone(), ui.clone(), clock.clone());
        state.deadline = deadline;
        Rig {
            dir,
            ui,
            config,
            clock,
            fake,
            state,
        }
    }

    fn never() -> Deadline {
        Arc::new(|| Box::pin(std::future::pending()))
    }

    fn tag(version: &str) -> FakeTransport {
        FakeTransport::json(200, &format!(r#"{{"tag_name":"{version}"}}"#))
    }

    fn stored(rig: &Rig) -> UiPrefs {
        UiPrefs::load(
            &DataPaths::new(rig.dir.path()).ui_json(),
            &ManualClock::new(Timestamp::from_unix_seconds(NOW)),
        )
        .unwrap()
        .0
    }

    #[tokio::test]
    async fn check_now_stores_cache_and_reports_newer() {
        let rig = rig_with(tag("v9.9.9"), never());
        assert_eq!(rig.state.check_now().await, Some("9.9.9".into()));
        let prefs = stored(&rig);
        assert_eq!(prefs.latest_version.as_deref(), Some("9.9.9"));
        assert_eq!(prefs.last_update_check, Some(NOW));
        assert_eq!(rig.ui.prefs().latest_version.as_deref(), Some("9.9.9"));
    }

    #[tokio::test]
    async fn check_now_silent_on_404_and_network() {
        for fake in [
            FakeTransport::json(404, r#"{"tag_name":"v9.9.9"}"#),
            FakeTransport::new(|_| Err(TranslationError::Network("offline".into()))),
        ] {
            let rig = rig_with(fake, never());
            assert_eq!(rig.state.check_now().await, None);
            assert_eq!(rig.ui.prefs().latest_version, None);
            assert_eq!(rig.ui.prefs().last_update_check, None);
            assert!(!DataPaths::new(rig.dir.path()).ui_json().exists());
        }
    }

    #[tokio::test]
    async fn check_at_startup_uses_cache_within_3h() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW - 100.0);
            })
            .unwrap();
        assert_eq!(rig.state.check_at_startup().await, Some("9.9.9".into()));
        assert!(rig.fake.requests().is_empty());
        // An older cached tag is not an update.
        rig.ui
            .update(|p| p.latest_version = Some("0.0.1".into()))
            .unwrap();
        assert_eq!(rig.state.check_at_startup().await, None);
        assert!(rig.fake.requests().is_empty());
    }

    #[tokio::test]
    async fn check_at_startup_refetches_at_exactly_3h() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW);
            })
            .unwrap();
        rig.clock.advance(CHECK_INTERVAL_SECONDS - 1.0);
        assert_eq!(rig.state.check_at_startup().await, Some("9.9.9".into()));
        assert!(rig.fake.requests().is_empty());
        rig.clock.advance(1.0);
        assert_eq!(rig.state.check_at_startup().await, Some("8.8.8".into()));
        assert_eq!(rig.fake.requests().len(), 1);
        assert_eq!(
            rig.ui.prefs().last_update_check,
            Some(NOW + CHECK_INTERVAL_SECONDS)
        );
    }

    #[tokio::test]
    async fn check_now_ignores_cache() {
        let rig = rig_with(tag("v8.8.8"), never());
        rig.ui
            .update(|p| {
                p.latest_version = Some("9.9.9".into());
                p.last_update_check = Some(NOW - 1.0);
            })
            .unwrap();
        assert_eq!(rig.state.check_now().await, Some("8.8.8".into()));
        assert_eq!(rig.fake.requests().len(), 1);
    }

    struct PendingTransport;
    #[async_trait::async_trait]
    impl Transport for PendingTransport {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, TranslationError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn check_now_times_out_silently() {
        let dir = tempfile::tempdir().unwrap();
        let (_config, _stores, ui) = crate::task4_tests::rig(dir.path());
        let clock = Arc::new(ManualClock::new(Timestamp::from_unix_seconds(NOW)));
        let mut state = UpdateState::new(Arc::new(PendingTransport), ui.clone(), clock);
        state.deadline = Arc::new(|| Box::pin(std::future::ready(())));
        assert_eq!(state.check_now().await, None);
        assert_eq!(ui.prefs().latest_version, None);
        assert_eq!(ui.prefs().last_update_check, None);
    }

    #[test]
    fn settings_view_hides_older_cached_version() {
        let rig = rig_with(tag("v9.9.9"), never());
        for (cached, want) in [("0.0.1", None), ("9.9.9", Some("9.9.9".to_string()))] {
            rig.ui
                .update(|p| p.latest_version = Some(cached.into()))
                .unwrap();
            let view = settings_view_parts(&rig.config, &rig.ui, &[]);
            assert_eq!(view.latest_version, want);
        }
    }

    #[tokio::test]
    async fn equal_or_older_latest_is_none() {
        let version = lingopop_core::VERSION;
        let rig = rig_with(tag(&format!("v{version}")), never());
        assert_eq!(rig.state.check_now().await, None);
        // The cache still records the lookup.
        assert_eq!(rig.ui.prefs().latest_version.as_deref(), Some(version));
        // A prerelease build sees the stable tag of the same triple as an update.
        if version.contains('-') {
            let stable = version.split('-').next().unwrap().to_string();
            let rig = rig_with(tag(&stable), never());
            assert_eq!(rig.state.check_now().await, Some(stable));
        }
    }

    #[test]
    fn open_url_allows_release_and_privacy_pages() {
        assert!(url_allowed(RELEASES_PAGE_URL));
        assert!(url_allowed(
            "https://github.com/slucheninov/lingopop-app/blob/main/PRIVACY.md"
        ));
    }
}
