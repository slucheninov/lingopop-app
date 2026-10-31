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
    *,
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
    fn show_at(&self, p: Point, m: &PopupViewModel) {
        self.view.show_at(p, m);
    }
    fn update(&self, m: &PopupViewModel) {
        self.view.update(m);
    }
    fn resize(&self, s: (f64, f64)) -> Option<Rect> {
        self.view.resize(s)
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
    let config = Arc::new(FakeConfig::new(Settings::default(), snapshot));
    let stores = Arc::new(FakePersist::new());
    let dismiss = Arc::new(FakeDismissWatcher::default());
    let orch = Orchestrator::new(Deps {
        platform: Platform {
            popup_window: Arc::new(FakePopupWindow::default()),
            dismiss: dismiss.clone(),
            clipboard: clip.clone(),
            keys: keys.clone(),
            foreground: fg.clone(),
            monitors: Arc::new(FakeMonitors::new(
                Point { x: 10, y: 20 },
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
        gate: PlatformGate::new(Default::default()),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.))),
        log: None,
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
#[tokio::test]
async fn copy_failure_shows_error_and_clears_displayed_result() {
    let (r, t) = ready_rig();
    ready(&r).await;
    r.clip
        .fail_next(FakeOp::Write, PlatformError::ClipboardBusy);
    r.orch.copy().await;
    error(&r, KEY_CLIPBOARD_TIMEOUT);
    let calls = r.popup.view.calls();
    r.orch.copy().await;
    r.orch.rewrite().await;
    assert_eq!(r.popup.view.calls(), calls);
    assert_eq!(t.requests().len(), 1);
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
    let Some(PopupCall::Resize(size)) = r.popup.view.calls().last().cloned() else {
        panic!("expected a Resize call");
    };
    assert_eq!(size, (700.0, 400.0));
    assert_eq!(r.popup.view.calls().len(), popup_calls + 1);
    assert!(matches!(
        r.dismiss.calls().last(),
        Some(DismissCall::UpdateRect(rect)) if (rect.width, rect.height) == (700, 400)
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
