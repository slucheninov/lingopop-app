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

fn issue_rig() -> (
    Arc<FakeLoginItem>,
    ShellHooks,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let item = Arc::new(FakeLoginItem::new());
    let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = changes.clone();
    let mut hooks = ShellHooks::synchronous(item.clone());
    hooks.on_login_issue = Some(Arc::new(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    (item, hooks, changes)
}

#[test]
fn login_requires_approval_is_remembered_signalled_and_cleared_on_success() {
    use crate::settings_cmds::{LoginIssue, SettingsHooks};
    let (item, hooks, changes) = issue_rig();
    assert_eq!(hooks.login_issue(), None);
    item.fail_next_with(PlatformError::LoginItemRequiresApproval);
    hooks.launch_at_login(true);
    assert_eq!(hooks.login_issue(), Some(LoginIssue::RequiresApproval));
    assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The user allowed it: the next successful enable clears the issue.
    hooks.launch_at_login(true);
    assert_eq!(hooks.login_issue(), None);
    assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn turning_the_toggle_off_clears_a_pending_approval() {
    use crate::settings_cmds::{LoginIssue, SettingsHooks};
    let (item, hooks, _) = issue_rig();
    item.fail_next_with(PlatformError::LoginItemRequiresApproval);
    hooks.launch_at_login(true);
    assert_eq!(hooks.login_issue(), Some(LoginIssue::RequiresApproval));
    hooks.launch_at_login(false);
    assert_eq!(hooks.login_issue(), None);
}

#[test]
fn an_unsupported_login_item_is_unavailable_and_other_errors_are_macos_only_failures() {
    use crate::settings_cmds::{LoginIssue, SettingsHooks};
    let (item, hooks, _) = issue_rig();
    item.fail_next_with(PlatformError::Unsupported(
        lingopop_platform::login_item::UNSUPPORTED_LOGIN_ITEM,
    ));
    hooks.launch_at_login(true);
    assert_eq!(hooks.login_issue(), Some(LoginIssue::Unavailable));
    item.fail_next();
    hooks.launch_at_login(true);
    // Windows keeps its log-only behaviour: the Run-key failure is not a UI state.
    let expected = if cfg!(target_os = "macos") {
        Some(LoginIssue::Failed)
    } else {
        None
    };
    assert_eq!(hooks.login_issue(), expected);
}

#[test]
fn a_stale_job_does_not_touch_the_remembered_issue() {
    use crate::settings_cmds::SettingsHooks;
    let item = Arc::new(FakeLoginItem::new());
    let queue = Arc::new(QueuedLogin(std::sync::Mutex::new(Vec::new())));
    let mut hooks = ShellHooks::synchronous(item.clone());
    hooks.spawn = queue.clone();
    hooks.launch_at_login(true);
    hooks.launch_at_login(false);
    let mut jobs: Vec<_> = queue.0.lock().unwrap().drain(..).collect();
    // The newer disable runs first (clean), then the stale enable must be skipped: it must not
    // call the item and must not record the approval error it would now get.
    (jobs.pop().expect("disable job"))();
    item.fail_next_with(PlatformError::LoginItemRequiresApproval);
    (jobs.pop().expect("enable job"))();
    assert_eq!(hooks.login_issue(), None);
    assert_eq!(item.calls().len(), 1);
}

#[test]
fn settings_view_carries_the_login_issue_only_when_there_is_one() {
    use crate::settings_cmds::{settings_view_parts, LoginIssue};
    let dir = tempfile::tempdir().expect("tempdir");
    let (config, _, ui) = load(
        &DataPaths::new(dir.path()),
        &platform(),
        Arc::new(SystemClock),
    );
    let mut view = settings_view_parts(&config, &ui, &[]);
    let plain = serde_json::to_value(&view).unwrap();
    assert!(plain.get("loginItemIssue").is_none());
    view.login_item_issue = Some(LoginIssue::RequiresApproval);
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["loginItemIssue"], "requiresApproval");
    view.login_item_issue = Some(LoginIssue::Unavailable);
    assert_eq!(
        serde_json::to_value(&view).unwrap()["loginItemIssue"],
        "unavailable"
    );
}

#[test]
fn open_login_items_settings_calls_the_platform_and_maps_a_failure_to_a_catalog_key() {
    use crate::settings_cmds::open_login_items_settings;
    use lingopop_core::ui_prefs::Language;
    let item = FakeLoginItem::new();
    assert_eq!(open_login_items_settings(&item, Language::En), Ok(()));
    assert_eq!(item.settings_opened(), 1);
    item.fail_next();
    let error = open_login_items_settings(&item, Language::En).unwrap_err();
    assert_eq!(error, "Couldn't open System Settings.");
    // Translated into the launched language (macOS catalog), no platform detail in it.
    item.fail_next();
    let uk = open_login_items_settings(&item, Language::Uk).unwrap_err();
    if cfg!(target_os = "macos") {
        assert_eq!(uk, "Не вдалося відкрити Системні налаштування.");
    }
    assert!(!uk.contains("RegSetValueExW"));
}

#[test]
fn a_second_launch_is_handled_before_any_native_service_exists() {
    // The single-instance plugin is registered first and a second process exits inside its
    // initialisation, so it never reaches `setup`, where the platform (and with it the login
    // item) is created and the saved launch-at-login flag is applied. Source order pins that.
    let src = include_str!("lib.rs");
    let single = src
        .find("tauri_plugin_single_instance::init")
        .expect("plugin");
    let setup_call = src
        .find(".setup(move |app| setup(app, &data_dir))")
        .expect("setup");
    let setup_fn = src.find("fn setup(").expect("setup fn");
    let run_fn = src.find("pub fn run()").expect("run fn");
    assert!(single > run_fn && single < setup_call);
    for needle in [
        "lingopop_platform::native()",
        "apply_startup_login(hooks.as_ref()",
    ] {
        let at: Vec<_> = src.match_indices(needle).map(|(i, _)| i).collect();
        assert_eq!(at.len(), 1, "{needle}");
        assert!(
            at[0] > setup_fn && at[0] < run_fn,
            "{needle} must live in setup"
        );
    }
    // Registering twice is impossible by construction: one login-item service per process, and
    // the handler of a second launch only opens Settings.
    assert!(src.contains("windows::open_settings(app, None)"));
}
