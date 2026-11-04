use crate::settings_cmds::{apply_patch, apply_settings_effects, SettingsPatch};
use crate::state::load;
use crate::{apply_startup_login, parse_wait_pid, ShellHooks};
use lingopop_core::paths::DataPaths;
use lingopop_core::settings::Settings;
use lingopop_core::time::SystemClock;
use lingopop_orchestrator::ConfigSource;
use lingopop_platform::{fake::*, Platform, PlatformError, Point};
use std::sync::Arc;

#[test]
fn parse_wait_pid_reads_flag() {
    let flagged = ["lingopop", "--wait-for-pid", "4242"]
        .map(str::to_string)
        .to_vec();
    assert_eq!(parse_wait_pid(&flagged), Some(4242));
    assert_eq!(parse_wait_pid(&[]), None);
    let bad = ["lingopop", "--wait-for-pid", "nope"]
        .map(str::to_string)
        .to_vec();
    assert_eq!(parse_wait_pid(&bad), None);
}

#[test]
fn startup_applies_launch_at_login_once() {
    let item = Arc::new(FakeLoginItem::new());
    let hooks = ShellHooks::synchronous(item.clone());
    let settings = Settings {
        launch_at_login: true,
        ..Settings::default()
    };
    apply_startup_login(&hooks, settings.launch_at_login);
    let exe = std::env::current_exe().expect("current exe");
    assert_eq!(item.calls(), vec![(true, exe)]);
}

#[test]
fn toggle_calls_login_item_and_keeps_setting_on_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (config, stores, _) = load(
        &DataPaths::new(dir.path()),
        &platform(),
        Arc::new(SystemClock),
    );
    let item = Arc::new(FakeLoginItem::new());
    item.fail_next();
    let hooks = ShellHooks::synchronous(item.clone());
    let saved = config.update_settings(|settings| {
        apply_patch(settings, SettingsPatch::LaunchAtLogin(true));
    });
    assert!(saved.is_ok());
    apply_settings_effects(&config, &stores, &hooks, false, true, false);
    assert!(config.settings().launch_at_login);
    assert_eq!(
        item.calls(),
        vec![(true, std::env::current_exe().expect("current exe"))]
    );
}

struct Protector;
impl lingopop_core::secrets::SecretProtector for Protector {
    fn protect(&self, data: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
        Ok(data.to_vec())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
        Ok(data.to_vec())
    }
}

fn platform() -> Platform {
    let sleeper = Arc::new(FakeSleeper::new());
    let clipboard = FakeClipboard::new(sleeper.clone());
    Platform {
        overlay: Arc::new(lingopop_platform::fake::FakeOverlay::new(Ok(None))),
        system: Arc::new(FakeSystem::new(vec!["en-US".into()])),
        login_item: Arc::new(FakeLoginItem::new()),
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: Arc::new(FakeDismissWatcher::default()),
        keys: FakeKeys::new(clipboard.clone(), sleeper.clone()),
        clipboard,
        foreground: FakeForeground::new(None),
        last_foreground: lingopop_platform::fake::FakeLastForeground::new(None),
        monitors: Arc::new(FakeMonitors::new(Point { x: 0, y: 0 }, vec![])),
        capture: Arc::new(FakeCapture::new([0; 4])),
        ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
        permissions: Arc::new(lingopop_platform::AlwaysGranted),
        protector: Arc::new(Protector),
        sleeper,
    }
}

/// Queues jobs so a test can run them in any order.
struct QueuedLogin(std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>);
impl crate::LoginSpawn for QueuedLogin {
    fn spawn_login(&self, job: Box<dyn FnOnce() + Send>) {
        self.0.lock().unwrap().push(job);
    }
}

#[test]
fn stale_login_item_job_is_skipped_when_it_runs_late() {
    use crate::settings_cmds::SettingsHooks;
    let item = Arc::new(FakeLoginItem::new());
    let queue = Arc::new(QueuedLogin(std::sync::Mutex::new(Vec::new())));
    let mut hooks = ShellHooks::synchronous(item.clone());
    hooks.spawn = queue.clone();
    hooks.launch_at_login(true);
    hooks.launch_at_login(false);
    let jobs: Vec<_> = queue.0.lock().unwrap().drain(..).collect();
    // The newer disable runs first, then the stale enable must not recreate the value.
    for job in jobs.into_iter().rev() {
        job();
    }
    let exe = std::env::current_exe().expect("current exe");
    assert_eq!(item.calls(), vec![(false, exe)]);
}
