//! The text-operation flow over platform fakes, a `FakeTransport` and a recording popup.

use lingopop_platform::fake::{FakeDismissWatcher, FakePopupWindow};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use lingopop_core::errors::TranslationError;
use lingopop_core::http::FakeTransport;
use lingopop_core::http::{HttpResponse, Transport};
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::{default_providers, ProviderType};
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::{ConfigSnapshot, OperationRunner};
use lingopop_core::secrets::{ProtectError, SecretProtector};
use lingopop_core::settings::Settings;
use lingopop_core::time::{ManualClock, Timestamp};
use lingopop_orchestrator::fake::{FakeConfig, FakePersist, FakePicker, FakePopup, PopupCall};
use lingopop_orchestrator::messages::{KEY_CLIPBOARD_TIMEOUT, KEY_NOTHING_SELECTED};
use lingopop_orchestrator::{
    max_input, missing_key, nothing_selected, popup_origin, Deps, GateTimeouts, Orchestrator,
    PlatformGate, PopupPhase, PopupViewModel, INITIAL_POPUP_SIZE,
};
use lingopop_platform::fake::{
    CopyBehavior, FakeCapture, FakeClipboard, FakeForeground, FakeKeys, FakeMonitors, FakeOcr,
    FakeSleeper,
};
use lingopop_platform::{
    Clipboard, ClipboardSnapshot, LogFn, MonitorInfo, Monitors, Platform, PlatformError, Point,
    Rect, WindowHandle,
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
        self.inner.write_text(t)
    }
}

struct TestMonitors {
    inner: FakeMonitors,
    threads: Mutex<Vec<ThreadId>>,
}

impl Monitors for TestMonitors {
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        self.inner.cursor_position()
    }
    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        self.inner.monitors()
    }
}

struct Rig {
    orch: Orchestrator,
    popup: Arc<FakePopup>,
    persist: Arc<FakePersist>,
    config: Arc<FakeConfig>,
    keys: Arc<FakeKeys>,
    clipboard: Arc<TestClipboard>,
    monitors: Arc<TestMonitors>,
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
    let monitors = Arc::new(TestMonitors {
        inner: FakeMonitors::new(CURSOR, monitor_list()),
        threads: Mutex::new(Vec::new()),
    });
    let platform = Platform {
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: Arc::new(FakeDismissWatcher::default()),
        clipboard: clipboard.clone(),
        keys: keys.clone(),
        foreground: FakeForeground::new(Some(WindowHandle(7))),
        monitors: monitors.clone(),
        capture: Arc::new(FakeCapture::new([0, 0, 0, 255])),
        ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
        protector: Arc::new(PlainProtector),
        sleeper,
    };
    let registry = Arc::new(ProviderRegistry::standard());
    let popup = Arc::new(FakePopup::new());
    let persist = Arc::new(FakePersist::new());
    let config = Arc::new(FakeConfig::new(
        Settings::default(),
        snapshot_with(opts.keys_for),
    ));
    let deps = Deps {
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
        orch: Orchestrator::new(deps),
        popup,
        persist,
        config,
        keys,
        clipboard,
        monitors,
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

fn result_text(m: &PopupViewModel) -> Option<&str> {
    match &m.phase {
        PopupPhase::Result(v) => Some(&v.text),
        _ => None,
    }
}

fn last_model(r: &Rig) -> PopupViewModel {
    match r.popup.calls().last().cloned().expect("a popup call") {
        PopupCall::ShowAt(_, m) | PopupCall::Update(m) => m,
        PopupCall::Hide | PopupCall::Resize(_) => panic!("popup was hidden or resized"),
    }
}

fn error_message_key(m: &PopupViewModel) -> String {
    match &m.phase {
        PopupPhase::Error(e) => e.message.key.clone(),
        other => panic!("not an error: {other:?}"),
    }
}

async fn wait_until(what: &str, cond: impl Fn() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timed out: {what}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test]
async fn translate_happy_path_shows_loading_then_result() {
    let t = ok_transport();
    let r = rig(t.clone(), "Hello");
    r.orch.trigger(AIOperation::Translate).await;
    let calls = r.popup.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let PopupCall::ShowAt(origin, loading) = &calls[0] else {
        panic!("first call must be show_at: {calls:?}")
    };
    assert_eq!(
        *origin,
        popup_origin(INITIAL_POPUP_SIZE, CURSOR, &monitor_list())
    );
    assert_eq!(
        *loading,
        PopupViewModel::loading(AIOperation::Translate, "Hello")
    );
    let PopupCall::Update(done) = &calls[1] else {
        panic!("second call must be update: {calls:?}")
    };
    assert_eq!(result_text(done), Some("TRANSLATED"));
    assert_eq!(t.requests().len(), 1);
}

#[tokio::test]
async fn nothing_selected_shows_error_and_no_request() {
    let t = ok_transport();
    let r = rig_with(t.clone(), RigOptions::default());
    r.keys.on_copy(CopyBehavior::NothingSelected);
    r.orch.trigger(AIOperation::Translate).await;
    let calls = r.popup.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let PopupCall::ShowAt(origin, m) = &calls[0] else {
        panic!("expected show_at: {calls:?}")
    };
    assert_eq!(
        *origin,
        popup_origin(INITIAL_POPUP_SIZE, CURSOR, &monitor_list())
    );
    assert_eq!(*m, PopupViewModel::error(nothing_selected(), vec![], &[]));
    assert_eq!(error_message_key(m), KEY_NOTHING_SELECTED);
    assert!(t.requests().is_empty());
}

#[tokio::test]
async fn max_input_exceeded_shows_error_and_no_request() {
    let t = ok_transport();
    let r = rig(t.clone(), "0123456789");
    r.config.update_settings(|s| s.max_input_chars = 5);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(
        last_model(&r),
        PopupViewModel::error(max_input(10, 5), vec![], &[])
    );
    assert!(t.requests().is_empty());
}

#[tokio::test]
async fn max_input_zero_means_no_limit() {
    let t = ok_transport();
    let r = rig(t.clone(), &"x".repeat(6000));
    r.config.update_settings(|s| s.max_input_chars = 0);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(result_text(&last_model(&r)), Some("TRANSLATED"));
    assert_eq!(t.requests().len(), 1);
}

const ENGLISH: &str = "Hello, how are you doing today my dear friend? I hope everything is fine.";

fn target_of(m: &PopupViewModel) -> String {
    match &m.phase {
        PopupPhase::Result(v) => v.target_language.clone(),
        other => panic!("not a result: {other:?}"),
    }
}

#[tokio::test]
async fn translate_uses_detected_language_pair() {
    let r = rig(ok_transport(), ENGLISH);
    r.config.update_settings(|s| {
        s.use_auto_detect = true;
        s.language_pairs.insert("English".into(), "German".into());
    });
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(target_of(&last_model(&r)), "German");
    let (entry, _) = r.persist.history().remove(0);
    assert_eq!(entry.source_language.as_deref(), Some("English"));
    assert_eq!(entry.target_language, "German");
}

#[tokio::test]
async fn grammar_fix_uses_target_language_without_detection() {
    let r = rig(ok_transport(), ENGLISH);
    r.config.update_settings(|s| {
        s.use_auto_detect = true;
        s.target_language = "Polish".into();
        s.language_pairs.insert("English".into(), "German".into());
    });
    r.orch.trigger(AIOperation::GrammarFix).await;
    assert_eq!(target_of(&last_model(&r)), "Polish");
    let (entry, _) = r.persist.history().remove(0);
    assert_eq!(entry.source_language, None);
}

#[tokio::test]
async fn success_persists_history_and_usage() {
    let r = rig(ok_transport(), "Hello");
    r.config.update_settings(|s| s.max_history_count = 7);
    r.orch.trigger(AIOperation::Translate).await;
    let history = r.persist.history();
    assert_eq!(history.len(), 1);
    let (entry, max) = &history[0];
    assert_eq!(*max, 7);
    assert_eq!(entry.original, "Hello");
    assert_eq!(entry.translated, "TRANSLATED");
    assert_eq!(entry.timestamp, Timestamp::from_unix_seconds(1000.0));
    assert_eq!(r.persist.usage().len(), 1);
}

#[tokio::test]
async fn error_shows_retry_providers_excluding_primary() {
    let t = Arc::new(FakeTransport::json(401, "{}"));
    let r = rig_with(
        t.clone(),
        RigOptions {
            keys_for: &[ProviderType::Claude, ProviderType::OpenAI],
            ..Default::default()
        },
    );
    r.keys.on_copy(CopyBehavior::Select("Hello".into()));
    r.orch.trigger(AIOperation::Translate).await;
    let m = last_model(&r);
    assert!(matches!(m.phase, PopupPhase::Error(_)), "{m:?}");
    let providers: Vec<ProviderType> = m.retry_providers.iter().map(|p| p.provider).collect();
    assert!(!providers.contains(&ProviderType::Claude));
    assert!(providers.contains(&ProviderType::OpenAI));
    assert!(r.persist.history().is_empty());
}

#[tokio::test]
async fn missing_key_message_names_provider() {
    let t = ok_transport();
    let r = rig_with(
        t.clone(),
        RigOptions {
            keys_for: &[],
            ..Default::default()
        },
    );
    r.keys.on_copy(CopyBehavior::Select("Hello".into()));
    r.orch.trigger(AIOperation::Translate).await;
    let PopupPhase::Error(e) = last_model(&r).phase else {
        panic!("expected an error")
    };
    assert_eq!(e.message, missing_key(ProviderType::Claude));
    assert!(e.details.is_empty());
    assert!(t.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_trigger_cancels_first_and_only_second_result_is_shown() {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let calls = Arc::new(AtomicUsize::new(0));
    let first_started = Arc::new(AtomicBool::new(false));
    let (c, fs) = (calls.clone(), first_started.clone());
    let t = Arc::new(FakeTransport::new(move |_| {
        let body = |text: &str| {
            Ok(HttpResponse::new(
                200,
                format!(
                    r#"{{"content":[{{"type":"text","text":"{text}"}}],"stop_reason":"end_turn"}}"#
                )
                .as_bytes(),
            ))
        };
        if c.fetch_add(1, Ordering::SeqCst) == 0 {
            fs.store(true, Ordering::SeqCst);
            let _ = release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            body("RESULT-ONE")
        } else {
            body("RESULT-TWO")
        }
    }));
    let r = rig(t, "first");
    let o = r.orch.clone();
    let first = tokio::spawn(async move { o.trigger(AIOperation::Translate).await });
    wait_until("first request in flight", || {
        first_started.load(Ordering::SeqCst)
    })
    .await;
    r.keys.on_copy(CopyBehavior::Select("second".into()));
    // The second trigger aborts the first flow, which is blocked inside the transport; its
    // JoinHandle resolves only after the release below.
    let o = r.orch.clone();
    let second = tokio::spawn(async move { o.trigger(AIOperation::Translate).await });
    wait_until("second result shown", || {
        r.popup
            .calls()
            .iter()
            .any(|c| matches!(c, PopupCall::Update(m) if result_text(m) == Some("RESULT-TWO")))
    })
    .await;
    release_tx.send(()).unwrap();
    first.await.unwrap();
    second.await.unwrap();
    let all = format!("{:?}", r.popup.calls());
    assert!(!all.contains("RESULT-ONE"), "{all}");
    let history = r.persist.history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].0.original, "second");
    assert_eq!(result_text(&last_model(&r)), Some("RESULT-TWO"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_while_capture_hangs_reports_clipboard_busy() {
    let r = rig_with(
        ok_transport(),
        RigOptions {
            timeouts: GateTimeouts {
                acquire: Duration::from_millis(100),
                run: Duration::from_millis(150),
            },
            ..Default::default()
        },
    );
    r.keys.on_copy(CopyBehavior::Select("Hello".into()));
    r.clipboard.blocked.store(true, Ordering::SeqCst);
    // The capture job hangs past the run timeout.
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_message_key(&last_model(&r)), KEY_CLIPBOARD_TIMEOUT);
    // The hung job still owns the gate: the next trigger cannot acquire it.
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_message_key(&last_model(&r)), KEY_CLIPBOARD_TIMEOUT);
    r.clipboard.blocked.store(false, Ordering::SeqCst);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(result_text(&last_model(&r)), Some("TRANSLATED"));
}

#[tokio::test(flavor = "current_thread")]
async fn platform_calls_run_off_the_async_thread() {
    let r = rig(ok_transport(), "Hello");
    r.orch.trigger(AIOperation::Translate).await;
    let here = std::thread::current().id();
    let clip = r.clipboard.threads.lock().unwrap().clone();
    let mon = r.monitors.threads.lock().unwrap().clone();
    assert!(!clip.is_empty() && !mon.is_empty());
    assert!(clip.iter().chain(mon.iter()).all(|&id| id != here));
}

#[tokio::test]
async fn orchestrator_never_calls_focus_on_popup() {
    let r = rig(ok_transport(), "Hello");
    r.orch.trigger(AIOperation::Translate).await;
    let calls = r.popup.calls();
    assert!(matches!(calls[0], PopupCall::ShowAt(..)));
    // The PopupView trait has no focus method; only these three kinds can ever be recorded.
    assert!(calls[1..].iter().all(|c| matches!(c, PopupCall::Update(_))));
}

#[tokio::test]
async fn capture_log_hook_receives_no_user_text() {
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let log: Arc<LogFn> = Arc::new(move |m: &str| sink.lock().unwrap().push(m.to_string()));
    let marker = "ZQ-MARKER-7f3a91";
    let r = rig_with(
        Arc::new(FakeTransport::new(|_| {
            Err(TranslationError::Network("down".into()))
        })),
        RigOptions {
            log: Some(log),
            ..Default::default()
        },
    );
    r.keys
        .on_copy(CopyBehavior::Select(format!("{marker} text")));
    r.orch.trigger(AIOperation::Translate).await;
    r.keys.on_copy(CopyBehavior::NothingSelected);
    r.orch.trigger(AIOperation::Translate).await;
    let lines = lines.lock().unwrap();
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|l| !l.contains(marker)), "{lines:?}");
}
