//! Popup actions over real runner requests and deterministic platform fakes.
use lingopop_core::{
    http::{FakeTransport, HttpResponse, Transport},
    operation::AIOperation,
    provider::{default_providers, ProviderType},
    providers::ProviderRegistry,
    runner::{ConfigSnapshot, OperationRunner},
    secrets::{ProtectError, SecretProtector},
    settings::Settings,
    time::{ManualClock, Timestamp},
};
use lingopop_orchestrator::{
    fake::{FakeConfig, FakePersist, FakePicker, FakePopup, PopupCall},
    messages::{KEY_CLIPBOARD_TIMEOUT, KEY_FOREGROUND_LOST, KEY_INPUT_BLOCKED},
    INPUT_BLOCKED_AUTO_CLOSE, *,
};
use lingopop_platform::{fake::*, *};
use std::sync::{Arc, Mutex};

struct Protector;
impl SecretProtector for Protector {
    fn protect(&self, p: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(p.to_vec())
    }
    fn unprotect(&self, p: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(p.to_vec())
    }
}
// The popup hide records the current platform event count, so paste ordering is checked
// against the platform fakes' shared event log without adding user text to that log.
struct Popup {
    view: FakePopup,
    log: Arc<EventLog>,
    hidden_at: Mutex<Vec<usize>>,
}
impl PopupView for Popup {
    fn show_at(&self, p: Point, s: (f64, f64), m: &PopupViewModel) -> bool {
        self.view.show_at(p, s, m)
    }
    fn update(&self, m: &PopupViewModel) {
        self.view.update(m);
    }
    fn resize(&self, o: Point, s: (f64, f64)) {
        self.view.resize(o, s);
    }
    fn hide(&self) {
        self.hidden_at.lock().unwrap().push(self.log.events().len());
        self.view.hide();
    }
    fn is_visible(&self) -> bool {
        self.view.is_visible()
    }
}
struct Rig {
    logs: Arc<Mutex<Vec<String>>>,
    orch: Orchestrator,
    dismiss: Arc<FakeDismissWatcher>,
    popup: Arc<Popup>,
    clip: Arc<FakeClipboard>,
    keys: Arc<FakeKeys>,
    fg: Arc<FakeForeground>,
    config: Arc<FakeConfig>,
    stores: Arc<FakePersist>,
}
const OK: &str = r#"{"content":[{"type":"text","text":"DISPLAYED"}],"usage":{"input_tokens":3,"output_tokens":2}}"#;
fn rig(t: Arc<dyn Transport>, target: Option<WindowHandle>) -> Rig {
    rig_with_gate(t, target, Default::default())
}
fn rig_with_gate(t: Arc<dyn Transport>, target: Option<WindowHandle>, gate: GateTimeouts) -> Rig {
    rig_full(t, target, gate, Point { x: 10, y: 20 })
}
fn rig_full(
    t: Arc<dyn Transport>,
    target: Option<WindowHandle>,
    gate: GateTimeouts,
    cursor: Point,
) -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    keys.on_copy(CopyBehavior::Select("ORIGINAL".into()));
    let fg = FakeForeground::new(target);
    let popup = Arc::new(Popup {
        view: FakePopup::new(),
        log: clip.log(),
        hidden_at: Mutex::new(vec![]),
    });
    let snapshot = ConfigSnapshot {
        magic_prompt: String::new(),
        fallbacks: [(ProviderType::OpenAI, ProviderType::Claude)].into(),
        connections: default_providers(),
        api_keys: ProviderType::ALL
            .into_iter()
            .map(|p| (p, "key".into()))
            .collect(),
    };
    // These tests are written against Claude as the active provider (the Windows default is Google (Free)).
    let settings = Settings {
        active_provider: ProviderType::Claude,
        ..Settings::default()
    };
    let config = Arc::new(FakeConfig::new(settings, snapshot));
    let stores = Arc::new(FakePersist::new());
    let dismiss = Arc::new(FakeDismissWatcher::default());
    let logs = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = logs.clone();
    let log: Arc<lingopop_platform::LogFn> =
        Arc::new(move |m: &str| sink.lock().unwrap().push(m.to_string()));
    let orch = Orchestrator::new(Deps {
        platform: Platform {
            popup_window: Arc::new(FakePopupWindow::default()),
            dismiss: dismiss.clone(),
            clipboard: clip.clone(),
            keys: keys.clone(),
            foreground: fg.clone(),
            monitors: Arc::new(FakeMonitors::new(
                cursor,
                vec![MonitorInfo {
                    bounds: Rect::new(-1000, -1000, 4000, 4000),
                    work_area: Rect::new(-1000, -1000, 4000, 4000),
                    scale_factor: 1.5,
                    primary: true,
                }],
            )),
            capture: Arc::new(FakeCapture::new([0; 4])),
            ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
            protector: Arc::new(Protector),
            sleeper,
        },
        runner: Arc::new(OperationRunner::new(ProviderRegistry::standard(), t)),
        registry_has: Arc::new(|_| true),
        config: config.clone(),
        stores: stores.clone(),
        popup: popup.clone(),
        gate: PlatformGate::new(gate),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.))),
        log: Some(log),
        picker: Arc::new(FakePicker::new(Ok(None))),
    });
    Rig {
        orch,
        dismiss,
        popup,
        clip,
        keys,
        fg,
        config,
        stores,
        logs,
    }
}
fn ready_rig() -> (Rig, Arc<FakeTransport>) {
    let t = Arc::new(FakeTransport::json(200, OK));
    (rig(t.clone(), Some(WindowHandle(7))), t)
}
async fn ready(r: &Rig) {
    r.orch.trigger(AIOperation::Translate).await;
    assert!(matches!(model(r).phase, PopupPhase::Result(_)));
}
fn model(r: &Rig) -> PopupViewModel {
    match r.popup.view.calls().last().unwrap() {
        PopupCall::Update(m) | PopupCall::ShowAt(_, m) => m.clone(),
        _ => panic!("hidden"),
    }
}
fn error(r: &Rig, key: &str) {
    let m = model(r);
    assert!(
        matches!(m.phase, PopupPhase::Error(e) if e.message.key == key && e.message.args.is_empty())
    );
    assert!(m.retry_providers.is_empty());
}
fn body(t: &FakeTransport) -> serde_json::Value {
    serde_json::from_slice(t.requests().last().unwrap().body.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn copy_writes_clipboard_and_keeps_popup() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.orch.copy().await;
    assert_eq!(r.clip.text().as_deref(), Some("DISPLAYED"));
    assert!(r.popup.is_visible());
    assert_eq!(r.popup.view.calls().len(), 2);
}
#[tokio::test]
async fn replace_hides_popup_before_paste() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.orch.replace().await;
    let at = r.popup.hidden_at.lock().unwrap()[0];
    let events = r.clip.log().events();
    assert!(events[at..]
        .iter()
        .any(|e| e.event == Event::Key(KeyCall::SendPaste)));
    assert!(!events[..at]
        .iter()
        .any(|e| e.event == Event::Key(KeyCall::SendPaste)));
    assert_eq!(r.keys.pasted(), vec![Some("DISPLAYED".into())]);
    assert!(!r.popup.is_visible());
}
#[tokio::test]
async fn replace_keeps_whitespace_of_captured_selection() {
    let (r, _) = ready_rig();
    r.keys
        .on_copy(CopyBehavior::Select("  ORIGINAL\r\n".into()));
    ready(&r).await;
    r.orch.replace().await;
    assert_eq!(r.keys.pasted(), vec![Some("  DISPLAYED\r\n".into())]);
}

#[tokio::test]
async fn replace_targets_window_captured_before_capture() {
    let (r, _) = ready_rig();
    let fg = r.fg.clone();
    r.clip.on_read_text(Box::new(move |_| {
        fg.restore(WindowHandle(99)).unwrap();
        assert_eq!(fg.current(), Some(WindowHandle(99)));
    }));
    ready(&r).await;
    r.orch.replace().await;
    assert_eq!(r.fg.restored(), vec![WindowHandle(99), WindowHandle(7)]);
}
#[tokio::test]
async fn replace_elevated_shows_uipi_message() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.fg.set_elevated(WindowHandle(7), true);
    r.orch.replace().await;
    error(&r, KEY_INPUT_BLOCKED);
    assert!(r.keys.pasted().is_empty());
    assert!(matches!(
        r.popup.view.calls().last(),
        Some(PopupCall::ShowAt(_, _))
    ));
}
#[tokio::test]
async fn replace_foreground_lost_shows_manual_paste_message() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.fg.fail_restore(true);
    r.orch.replace().await;
    error(&r, KEY_FOREGROUND_LOST);
    assert_eq!(r.clip.text().as_deref(), Some("DISPLAYED"));
    assert!(r.keys.pasted().is_empty());
}
#[tokio::test]
async fn replace_without_target_leaves_text_on_clipboard() {
    let r = rig(Arc::new(FakeTransport::json(200, OK)), None);
    ready(&r).await;
    r.orch.replace().await;
    error(&r, KEY_FOREGROUND_LOST);
    assert_eq!(r.clip.text().as_deref(), Some("DISPLAYED"));
    assert!(r.fg.restored().is_empty());
    assert!(r.keys.pasted().is_empty());
}
#[tokio::test]
async fn rewrite_runs_on_result_text() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.orch.rewrite().await;
    assert_eq!(
        body(&t)["messages"][0]["content"][0]["text"],
        "<text>\nDISPLAYED\n</text>"
    );
    assert_eq!(r.stores.history()[1].0.operation, "rewrite");
    r.orch.retry(ProviderType::Claude).await;
    assert_eq!(
        body(&t)["messages"][0]["content"][0]["text"],
        "<text>\nDISPLAYED\n</text>"
    );
}
#[tokio::test]
async fn explain_runs_on_result_text() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.orch.explain().await;
    assert_eq!(
        body(&t)["messages"][0]["content"][0]["text"],
        "<text>\nDISPLAYED\n</text>"
    );
    assert_eq!(r.stores.history()[1].0.operation, "explain");
}
#[tokio::test]
async fn retry_uses_chosen_provider_without_fallback() {
    let t = Arc::new(FakeTransport::new(|req| {
        if req.url.as_str().contains("openai") {
            Ok(HttpResponse {
                status: 503,
                body: vec![],
            })
        } else {
            Ok(HttpResponse {
                status: 200,
                body: OK.as_bytes().to_vec(),
            })
        }
    }));
    let r = rig(t.clone(), Some(WindowHandle(7)));
    ready(&r).await;
    r.orch.retry(ProviderType::OpenAI).await;
    assert_eq!(t.requests().len(), 2);
    assert!(t.requests()[1].url.as_str().contains("openai"));
    assert_eq!(
        body(&t)["messages"][1]["content"],
        "<text>\nORIGINAL\n</text>"
    );
    assert!(matches!(model(&r).phase, PopupPhase::Error(_)));
}
#[tokio::test]
async fn retranslate_runs_translate_on_original_text() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.config
        .update_settings(|s| s.target_language = "French".into());
    r.orch.retranslate("German".into()).await;
    assert_eq!(
        body(&t)["messages"][0]["content"][0]["text"],
        "<text>\nORIGINAL\n</text>"
    );
    assert!(body(&t)["system"].as_str().unwrap().contains("German"));
    let h = r.stores.history();
    assert_eq!(h[1].0.operation, "translate");
    assert_eq!(h[1].0.target_language, "German");
}
#[tokio::test]
async fn escape_closes_popup_and_clears_session() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.orch.escape().await;
    let calls = r.popup.view.calls();
    assert!(matches!(calls.last(), Some(PopupCall::Hide)));
    r.orch.copy().await;
    r.orch.replace().await;
    r.orch.retry(ProviderType::Claude).await;
    r.orch.retranslate("German".into()).await;
    assert_eq!(r.popup.view.calls(), calls);
    assert_eq!(t.requests().len(), 1);
}
#[tokio::test]
async fn actions_without_result_do_nothing() {
    let (r, t) = ready_rig();
    r.orch.copy().await;
    r.orch.replace().await;
    r.orch.rewrite().await;
    r.orch.explain().await;
    r.orch.retry(ProviderType::Claude).await;
    r.orch.retranslate("German".into()).await;
    assert!(r.popup.view.calls().is_empty());
    assert!(t.requests().is_empty());
}
fn notice_key(r: &Rig) -> Option<String> {
    model(r).notice.map(|n| n.key)
}
fn result_text(r: &Rig) -> String {
    match model(r).phase {
        PopupPhase::Result(v) => v.text,
        _ => panic!("result expected"),
    }
}
#[tokio::test]
async fn copy_failure_keeps_result_with_notice() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.clip
        .fail_next(FakeOp::Write, PlatformError::ClipboardBusy);
    r.orch.copy().await;
    assert_eq!(result_text(&r), "DISPLAYED");
    assert_eq!(notice_key(&r).as_deref(), Some(KEY_CLIPBOARD_TIMEOUT));
    assert!(r.popup.is_visible());
    // The session stays usable: a later Copy succeeds and drops the notice.
    r.orch.copy().await;
    assert_eq!(r.clip.text().as_deref(), Some("DISPLAYED"));
    assert_eq!(result_text(&r), "DISPLAYED");
    assert_eq!(notice_key(&r), None);
    // Replace and Retry still work.
    r.orch.retry(ProviderType::Claude).await;
    assert_eq!(t.requests().len(), 2);
    r.orch.replace().await;
    assert_eq!(r.keys.pasted(), vec![Some("DISPLAYED".into())]);
}
#[tokio::test]
async fn replace_gate_failure_reshows_result() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.clip
        .fail_next(FakeOp::WriteTransient, PlatformError::ClipboardBusy);
    r.orch.replace().await;
    assert!(r.keys.pasted().is_empty());
    assert!(r.popup.is_visible());
    assert!(matches!(
        r.popup.view.calls().last(),
        Some(PopupCall::ShowAt(_, _))
    ));
    assert_eq!(result_text(&r), "DISPLAYED");
    assert_eq!(notice_key(&r).as_deref(), Some(KEY_CLIPBOARD_TIMEOUT));
    // The dismiss watcher is restarted with the re-shown popup.
    let starts = r
        .dismiss
        .calls()
        .iter()
        .filter(|c| matches!(c, DismissCall::Start(_)))
        .count();
    assert_eq!(starts, 2);
    assert_eq!(
        r.dismiss
            .calls()
            .iter()
            .filter(|c| **c == DismissCall::Stop)
            .count(),
        1
    );
    // Same session: Copy works and Replace now pastes.
    r.orch.copy().await;
    assert_eq!(r.clip.text().as_deref(), Some("DISPLAYED"));
    r.orch.replace().await;
    assert_eq!(r.keys.pasted(), vec![Some("DISPLAYED".into())]);
    assert!(!r.popup.is_visible());
}

fn replace_shown(r: &Rig) -> bool {
    matches!(&model(r).phase, PopupPhase::Result(v) if v.show_replace)
}
#[tokio::test]
async fn replace_gate_failure_keeps_replace_available() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.clip
        .fail_next(FakeOp::WriteTransient, PlatformError::ClipboardBusy);
    r.orch.replace().await;
    assert!(replace_shown(&r), "a confirmed failure keeps Replace");
}
#[tokio::test]
async fn replace_timeout_disables_further_replace() {
    let timeouts = GateTimeouts {
        acquire: std::time::Duration::from_millis(200),
        run: std::time::Duration::from_millis(100),
    };
    let r = rig_with_gate(
        Arc::new(FakeTransport::json(200, OK)),
        Some(WindowHandle(7)),
        timeouts,
    );
    ready(&r).await;
    assert!(replace_shown(&r));
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    r.keys.on_paste(Box::new(move |_| {
        let _ = wait.lock().unwrap().recv();
    }));
    r.orch.replace().await;
    // Unknown outcome: the result is shown again with a notice, without Replace.
    assert!(r.popup.is_visible());
    assert_eq!(result_text(&r), "DISPLAYED");
    assert_eq!(notice_key(&r).as_deref(), Some(KEY_CLIPBOARD_TIMEOUT));
    assert!(!replace_shown(&r));
    // A second Replace does nothing: no new popup call, no extra paste.
    let calls = r.popup.view.calls().len();
    r.orch.replace().await;
    assert_eq!(r.popup.view.calls().len(), calls);
    // Release the blocked job and wait for it to finish.
    release.send(()).unwrap();
    for _ in 0..200 {
        if !r.keys.pasted().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(r.keys.pasted().len(), 1, "only the first job pasted");
    assert_eq!(
        r.popup.view.calls().len(),
        calls,
        "the late job shows nothing"
    );
    assert!(r.popup.is_visible());
    // A later result of the same session does not offer Replace, and Replace stays a no-op.
    r.orch.rewrite().await;
    assert!(matches!(model(&r).phase, PopupPhase::Result(_)));
    assert!(!replace_shown(&r));
    r.orch.replace().await;
    assert_eq!(r.keys.pasted().len(), 1);
}

#[tokio::test]
async fn reshow_uses_measured_size_for_dismiss_rect() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.orch.popup_resized((700.0, 400.0));
    for _ in 0..2 {
        r.clip
            .fail_next(FakeOp::WriteTransient, PlatformError::ClipboardBusy);
        r.orch.replace().await;
    }
    let starts: Vec<Rect> = r
        .dismiss
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            DismissCall::Start(rect) => Some(rect),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 3);
    let measured = physical_size(700.0, 400.0, 1.5);
    for rect in &starts[1..] {
        assert_eq!((rect.width, rect.height), (measured.width, measured.height));
    }
    assert_eq!(r.popup.view.show_sizes()[1..], [(700.0, 400.0); 2]);
}

#[tokio::test]
async fn resize_near_edge_is_clamped() {
    let work = Rect::new(-1000, -1000, 4000, 4000);
    // Cursor near the bottom-right corner of the monitor.
    let cursor = Point { x: 2990, y: 2990 };
    let r = rig_full(
        Arc::new(FakeTransport::json(200, OK)),
        Some(WindowHandle(7)),
        Default::default(),
        cursor,
    );
    ready(&r).await;
    r.orch.popup_resized((700.0, 400.0));
    let Some(PopupCall::Resize(origin, size)) = r.popup.view.calls().last().cloned() else {
        panic!("expected a Resize call");
    };
    let monitors = vec![MonitorInfo {
        bounds: work,
        work_area: work,
        scale_factor: 1.5,
        primary: true,
    }];
    assert_eq!(origin, popup_origin(size, cursor, &monitors));
    let phys = physical_size(size.0, size.1, 1.5);
    let inset = (8.0f64 * 1.5).ceil() as i32;
    assert!(origin.x + phys.width <= work.right() - inset);
    assert!(origin.y + phys.height <= work.bottom() - inset);
    assert!(origin.x >= work.left() + inset && origin.y >= work.top() + inset);
}

struct PendingTransport {
    entered: tokio::sync::Notify,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
struct Dropped(Arc<std::sync::atomic::AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Transport for PendingTransport {
    fn send<'a, 'b>(
        &'a self,
        _: lingopop_core::http::HttpRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<HttpResponse, lingopop_core::errors::TranslationError>,
                > + Send
                + 'b,
        >,
    >
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move {
            let _guard = Dropped(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn action_during_run_cancels_it() {
    let t = Arc::new(PendingTransport {
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let r = rig(t.clone(), Some(WindowHandle(7)));
    let me = r.orch.clone();
    let run = tokio::spawn(async move {
        me.trigger(AIOperation::Translate).await;
    });
    t.entered.notified().await;
    r.orch.close().await;
    run.await.unwrap();
    assert!(t.dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(matches!(r.popup.view.calls().last(), Some(PopupCall::Hide)));
    assert!(r.stores.history().is_empty());
}

#[tokio::test]
async fn rewrite_and_explain_use_active_provider_with_fallback() {
    for operation in [AIOperation::Rewrite, AIOperation::Explain] {
        let t = Arc::new(FakeTransport::by_host(vec![
            ("api.anthropic.com", HttpResponse::new(200, OK)),
            ("api.openai.com", HttpResponse::new(503, "unavailable")),
        ]));
        let r = rig(t.clone(), Some(WindowHandle(7)));
        ready(&r).await;
        r.config
            .update_settings(|s| s.active_provider = ProviderType::OpenAI);
        match operation {
            AIOperation::Rewrite => r.orch.rewrite().await,
            _ => r.orch.explain().await,
        }
        let requests = t.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].url.host_str(), Some("api.openai.com"));
        assert_eq!(requests[2].url.host_str(), Some("api.anthropic.com"));
        assert_eq!(
            body(&t)["messages"][0]["content"][0]["text"],
            "<text>\nDISPLAYED\n</text>"
        );
        assert!(
            matches!(model(&r).phase, PopupPhase::Result(result) if result.via_fallback && result.operation == operation)
        );
    }
}

#[tokio::test]
async fn dismiss_event_closes_popup() {
    for event in [DismissEvent::Escape, DismissEvent::ClickOutside] {
        let (r, _) = ready_rig();
        ready(&r).await;
        r.dismiss.fire(event);
        r.orch
            .dismissed(r.dismiss.events().try_recv().unwrap())
            .await;
        assert!(!r.popup.is_visible());
        assert!(matches!(r.popup.view.calls().last(), Some(PopupCall::Hide)));
        let count = r.popup.view.calls().len();
        r.orch.retry(ProviderType::Claude).await;
        assert_eq!(
            r.popup.view.calls().len(),
            count,
            "dismissal clears session"
        );
    }
}
#[tokio::test]
async fn showing_popup_starts_dismiss_watcher_with_popup_rect() {
    let (r, _) = ready_rig();
    ready(&r).await;
    let PopupCall::ShowAt(origin, _) = &r.popup.view.calls()[0] else {
        panic!("show expected");
    };
    let size = physical_size(
        popup::INITIAL_POPUP_SIZE.0,
        popup::INITIAL_POPUP_SIZE.1,
        1.5,
    );
    assert_eq!(
        r.dismiss.calls(),
        [DismissCall::Start(Rect::new(
            origin.x,
            origin.y,
            size.width,
            size.height
        ))]
    );
}
#[tokio::test]
async fn hiding_popup_stops_dismiss_watcher() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.orch.close().await;
    assert_eq!(r.dismiss.calls().last(), Some(&DismissCall::Stop));
    r.dismiss.fire(DismissEvent::Escape);
    assert!(r.dismiss.events().try_recv().is_err());
}
#[tokio::test]
async fn popup_resized_updates_dismiss_rect() {
    let (r, _) = ready_rig();
    ready(&r).await;
    let popup_calls = r.popup.view.calls().len();
    r.orch.popup_resized((700.0, 400.0));
    let Some(PopupCall::Resize(_, size)) = r.popup.view.calls().last().cloned() else {
        panic!("expected a Resize call");
    };
    assert_eq!(size, (700.0, 400.0));
    assert_eq!(r.popup.view.calls().len(), popup_calls + 1);
    assert!(matches!(
        r.dismiss.calls().last(),
        Some(DismissCall::UpdateRect(rect)) if (rect.width, rect.height) == (1050, 600)
    ));
}
#[tokio::test]
async fn popup_resized_after_close_does_nothing() {
    let (r, _) = ready_rig();
    ready(&r).await;
    r.orch.close().await;
    let popup_calls = r.popup.view.calls();
    let dismiss_calls = r.dismiss.calls();
    r.orch.popup_resized((700.0, 400.0));
    assert_eq!(r.popup.view.calls(), popup_calls, "no show or resize");
    assert!(!r.popup.view.is_visible());
    assert_eq!(r.dismiss.calls(), dismiss_calls, "no update_rect");
}
#[tokio::test]
async fn show_failure_does_not_start_dismiss_watcher() {
    let (r, _) = ready_rig();
    r.popup.view.fail_show(true);
    r.orch.trigger(AIOperation::Translate).await;
    assert!(!r.popup.view.is_visible());
    assert!(r.dismiss.calls().is_empty(), "no dismiss.start");
    assert!(r
        .logs
        .lock()
        .unwrap()
        .iter()
        .any(|l| l == "popup show failed"));
    let popup_calls = r.popup.view.calls();
    r.orch.popup_resized((700.0, 400.0));
    assert_eq!(r.popup.view.calls(), popup_calls, "no resize");
    assert!(r.dismiss.calls().is_empty(), "no update_rect");
}

#[tokio::test]
async fn input_blocked_error_closes_itself_after_delay() {
    let (r, _) = ready_rig();
    r.fg.set_elevated(WindowHandle(7), true);
    r.orch.trigger(AIOperation::Translate).await;
    error(&r, KEY_INPUT_BLOCKED);
    assert!(r.popup.is_visible());
    tokio::time::pause();
    tokio::time::sleep(INPUT_BLOCKED_AUTO_CLOSE + std::time::Duration::from_secs(1)).await;
    assert!(!r.popup.is_visible());
    assert_eq!(r.dismiss.calls().last(), Some(&DismissCall::Stop));
}

#[tokio::test]
async fn newer_trigger_prevents_input_blocked_auto_close() {
    let (r, _) = ready_rig();
    r.fg.set_elevated(WindowHandle(7), true);
    r.orch.trigger(AIOperation::Translate).await;
    error(&r, KEY_INPUT_BLOCKED);
    r.fg.set_elevated(WindowHandle(7), false);
    ready(&r).await;
    tokio::time::pause();
    tokio::time::sleep(INPUT_BLOCKED_AUTO_CLOSE + std::time::Duration::from_secs(1)).await;
    assert!(r.popup.is_visible());
    assert!(matches!(model(&r).phase, PopupPhase::Result(_)));
}
