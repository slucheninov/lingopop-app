//! Status lifecycle and gated History/tray clipboard writes over platform fakes.

use lingopop_platform::fake::{FakeDismissWatcher, FakePopupWindow};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use lingopop_core::http::FakeTransport;
use lingopop_core::http::Transport;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::{default_providers, ProviderType};
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::{ConfigSnapshot, OperationRunner};
use lingopop_core::secrets::{ProtectError, SecretProtector};
use lingopop_core::settings::Settings;
use lingopop_core::time::{ManualClock, Timestamp};
use lingopop_orchestrator::fake::{FakeConfig, FakePersist, FakePicker, FakePopup};
use lingopop_orchestrator::{Deps, GateTimeouts, Orchestrator, PlatformGate};
use lingopop_platform::fake::{
    CopyBehavior, FakeCapture, FakeClipboard, FakeForeground, FakeKeys, FakeMonitors, FakeOcr,
    FakeSleeper,
};
use lingopop_platform::{
    Clipboard, ClipboardSnapshot, LogFn, MonitorInfo, Platform, PlatformError, Point, Rect,
    WindowHandle,
};

const CLAUDE_OK: &str = r#"{"content":[{"type":"text","text":"TRANSLATED"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#;
const CURSOR: Point = Point { x: 100, y: 100 };

struct PlainProtector;

impl SecretProtector for PlainProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(plain.to_vec())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(data.to_vec())
    }
}

fn monitor_list() -> Vec<MonitorInfo> {
    let r = Rect::new(0, 0, 1920, 1080);
    vec![MonitorInfo {
        bounds: r,
        work_area: r,
        scale_factor: 1.0,
        primary: true,
    }]
}

/// Delegates to a `FakeClipboard`; can block `snapshot` and records the threads it ran on.
struct TestClipboard {
    inner: Arc<FakeClipboard>,
    blocked: AtomicBool,
    threads: Mutex<Vec<ThreadId>>,
}

impl Clipboard for TestClipboard {
    fn sequence_number(&self) -> u64 {
        self.inner.sequence_number()
    }
    fn snapshot(&self) -> Result<ClipboardSnapshot, PlatformError> {
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        let start = Instant::now();
        while self.blocked.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.inner.snapshot()
    }
    fn restore(&self, s: &ClipboardSnapshot) -> Result<(), PlatformError> {
        self.inner.restore(s)
    }
    fn restore_if(&self, s: &ClipboardSnapshot, seq: u64) -> Result<bool, PlatformError> {
        self.inner.restore_if(s, seq)
    }
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        self.inner.read_text()
    }
    fn write_text_transient(&self, t: &str) -> Result<(), PlatformError> {
        self.inner.write_text_transient(t)
    }
    fn write_text(&self, t: &str) -> Result<(), PlatformError> {
        while self.blocked.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.inner.write_text(t)
    }
}

struct Rig {
    status: Arc<lingopop_orchestrator::fake::FakeStatus>,
    orch: Orchestrator,
    popup: Arc<FakePopup>,
    keys: Arc<FakeKeys>,
    clipboard: Arc<TestClipboard>,
}

struct RigOptions {
    timeouts: GateTimeouts,
    log: Option<Arc<LogFn>>,
    keys_for: &'static [ProviderType],
}

impl Default for RigOptions {
    fn default() -> Self {
        Self {
            timeouts: GateTimeouts {
                acquire: Duration::from_secs(2),
                run: Duration::from_secs(3),
            },
            log: None,
            keys_for: &[ProviderType::Claude],
        }
    }
}

fn snapshot_with(keys: &[ProviderType]) -> ConfigSnapshot {
    ConfigSnapshot {
        magic_prompt: String::new(),
        fallbacks: Default::default(),
        connections: default_providers(),
        api_keys: keys
            .iter()
            .map(|&p| (p, "key-FIXTURE".to_string()))
            .collect(),
    }
}

fn rig_with(transport: Arc<dyn Transport>, opts: RigOptions) -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let fake_clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(fake_clip.clone(), sleeper.clone());
    let clipboard = Arc::new(TestClipboard {
        inner: fake_clip,
        blocked: AtomicBool::new(false),
        threads: Mutex::new(Vec::new()),
    });
    let monitors = Arc::new(FakeMonitors::new(CURSOR, monitor_list()));
    let platform = Platform {
        system: Arc::new(lingopop_platform::fake::FakeSystem::new(vec![])),
        login_item: Arc::new(lingopop_platform::fake::FakeLoginItem::new()),
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: Arc::new(FakeDismissWatcher::default()),
        clipboard: clipboard.clone(),
        keys: keys.clone(),
        foreground: FakeForeground::new(Some(WindowHandle(7))),
        last_foreground: lingopop_platform::fake::FakeLastForeground::new(None),
        monitors: monitors.clone(),
        capture: Arc::new(FakeCapture::new([0, 0, 0, 255])),
        ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
        protector: Arc::new(PlainProtector),
        sleeper,
    };
    let registry = Arc::new(ProviderRegistry::standard());
    let popup = Arc::new(FakePopup::new());
    let persist = Arc::new(FakePersist::new());
    // Written against Claude as the active provider (the Windows default is Google (Free)).
    let settings = Settings {
        active_provider: ProviderType::Claude,
        ..Settings::default()
    };
    let config = Arc::new(FakeConfig::new(settings, snapshot_with(opts.keys_for)));
    let status = Arc::new(lingopop_orchestrator::fake::FakeStatus::default());
    let deps = Deps {
        status: Some(status.clone()),
        platform,
        runner: Arc::new(OperationRunner::new(
            ProviderRegistry::standard(),
            transport,
        )),
        registry_has: Arc::new(move |p| registry.get(p).is_ok()),
        config: config.clone(),
        stores: persist.clone(),
        popup: popup.clone(),
        gate: PlatformGate::new(opts.timeouts),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.0))),
        log: opts.log,
        picker: Arc::new(FakePicker::new(Ok(None))),
    };
    Rig {
        status,
        orch: Orchestrator::new(deps),
        popup,
        keys,
        clipboard,
    }
}

fn ok_transport() -> Arc<FakeTransport> {
    Arc::new(FakeTransport::json(200, CLAUDE_OK))
}

fn rig(transport: Arc<FakeTransport>, selected: &str) -> Rig {
    let r = rig_with(transport, RigOptions::default());
    r.keys.on_copy(CopyBehavior::Select(selected.to_string()));
    r
}

use lingopop_orchestrator::TrayStatus::{Error, Idle, Working};
#[tokio::test]
async fn status_working_then_idle_on_success() {
    let r = rig(ok_transport(), "Hello");
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(r.status.statuses(), vec![Working, Idle]);
}
#[tokio::test]
async fn status_error_on_failure() {
    let r = rig(Arc::new(FakeTransport::json(400, "{}")), "Hello");
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(r.status.statuses(), vec![Working, Error]);
}
#[tokio::test]
async fn status_idle_on_close() {
    let r = rig(ok_transport(), "Hello");
    r.orch.close().await;
    assert_eq!(r.status.statuses().last(), Some(&Idle));
}
#[tokio::test]
async fn screen_translate_cancel_never_working() {
    let r = rig(ok_transport(), "Hello");
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(!r.status.statuses().contains(&Working));
}
#[tokio::test]
async fn write_clipboard_goes_through_gate() {
    let r = rig(ok_transport(), "Hello");
    r.orch.write_clipboard("copied".into()).await.unwrap();
    assert_eq!(r.clipboard.read_text().unwrap().as_deref(), Some("copied"));
    assert!(r.popup.calls().is_empty());
}
#[tokio::test]
async fn superseded_working_flow_ends_idle() {
    struct Pending;
    #[async_trait::async_trait]
    impl Transport for Pending {
        async fn send(
            &self,
            _: lingopop_core::http::HttpRequest,
        ) -> Result<lingopop_core::http::HttpResponse, lingopop_core::errors::TranslationError>
        {
            std::future::pending().await
        }
    }
    let r = rig_with(Arc::new(Pending), RigOptions::default());
    r.keys.on_copy(CopyBehavior::Select("Hello".into()));
    let o = r.orch.clone();
    let task = tokio::spawn(async move {
        o.trigger(AIOperation::Translate).await;
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while r.status.statuses() != vec![Working] {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = r.orch.clone();
    let second_task = tokio::spawn(async move {
        second.trigger(AIOperation::Translate).await;
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while r.status.statuses().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(r.status.statuses()[..3], [Working, Idle, Working]);
    r.orch.close().await;
    task.await.unwrap();
    second_task.await.unwrap();
}

#[tokio::test]
async fn write_clipboard_timeout_and_busy_without_popup() {
    let r = rig_with(
        ok_transport(),
        RigOptions {
            timeouts: GateTimeouts {
                acquire: Duration::from_millis(10),
                run: Duration::from_millis(20),
            },
            ..RigOptions::default()
        },
    );
    // Hold the same platform gate through a blocked snapshot. The clipboard action must
    // report Busy and never touch popup/session state.
    r.clipboard.blocked.store(true, Ordering::SeqCst);
    let orch = r.orch.clone();
    let task = tokio::spawn(async move {
        orch.trigger(AIOperation::Translate).await;
    });
    while r.clipboard.threads.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        r.orch.write_clipboard("copy".into()).await,
        Err(lingopop_orchestrator::GateError::Busy)
    );
    r.clipboard.blocked.store(false, Ordering::SeqCst);
    task.await.unwrap();
}

#[tokio::test]
async fn hung_clipboard_write_times_out() {
    let r = rig_with(
        ok_transport(),
        RigOptions {
            timeouts: GateTimeouts {
                acquire: Duration::from_millis(10),
                run: Duration::from_millis(20),
            },
            ..RigOptions::default()
        },
    );
    r.clipboard.blocked.store(true, Ordering::SeqCst);
    let result = r.orch.write_clipboard("copy".into()).await;
    r.clipboard.blocked.store(false, Ordering::SeqCst);
    assert_eq!(result, Err(lingopop_orchestrator::GateError::TimedOut));
    assert!(r.popup.calls().is_empty());
}

#[tokio::test]
async fn panicked_working_flow_ends_error() {
    let r = rig(
        Arc::new(FakeTransport::new(|_| panic!("fixture panic"))),
        "Hello",
    );
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(r.status.statuses(), vec![Working, Error]);
}
