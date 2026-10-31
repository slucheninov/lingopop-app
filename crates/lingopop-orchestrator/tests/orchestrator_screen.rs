//! The Screen Translate flow over platform fakes, a scripted region picker and a `FakeTransport`.

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use lingopop_core::http::{FakeTransport, HttpResponse, Transport};
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::{default_providers, ProviderType};
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::{ConfigSnapshot, OperationRunner};
use lingopop_core::secrets::{ProtectError, SecretProtector};
use lingopop_core::settings::Settings;
use lingopop_core::time::{ManualClock, Timestamp};
use lingopop_orchestrator::fake::{FakeConfig, FakePersist, FakePicker, FakePopup, PopupCall};
use lingopop_orchestrator::messages::{
    KEY_NOTHING_SELECTED, KEY_NO_TEXT_FOUND, KEY_NO_TEXT_FOUND_RETRY, KEY_OCR_UNAVAILABLE,
    KEY_PLATFORM_GENERIC, KEY_SELECTION_TOO_SMALL,
};
use lingopop_orchestrator::{
    popup_origin, Deps, Orchestrator, PlatformGate, PopupPhase, PopupViewModel, INITIAL_POPUP_SIZE,
};
use lingopop_platform::fake::{
    CopyBehavior, FakeCapture, FakeClipboard, FakeForeground, FakeKeys, FakeMonitors, FakeOcr,
    FakeSleeper,
};
use lingopop_platform::{
    CapturedImage, Clipboard, MonitorInfo, Ocr, OcrText, Platform, PlatformError, Point, Rect,
    WindowHandle,
};

const CLAUDE_OK: &str = r#"{"content":[{"type":"text","text":"TRANSLATED"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#;
const OPENAI_OK: &str = r#"{"choices":[{"message":{"content":"VIA-OPENAI"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#;
const OCR_TEXT: &str = "Hello world";
const REGION: Rect = Rect {
    x: 100,
    y: 100,
    width: 200,
    height: 50,
};

struct PlainProtector;

impl SecretProtector for PlainProtector {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(plain.to_vec())
    }
    fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, ProtectError> {
        Ok(data.to_vec())
    }
}

/// Delegates to a `FakeOcr` and records the threads it ran on.
struct TestOcr {
    inner: FakeOcr,
    threads: Mutex<Vec<ThreadId>>,
    after: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Ocr for TestOcr {
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError> {
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        let out = self.inner.recognize(image);
        if let Some(hook) = self.after.lock().unwrap().take() {
            hook();
        }
        out
    }
}

struct Rig {
    orch: Orchestrator,
    popup: Arc<FakePopup>,
    persist: Arc<FakePersist>,
    config: Arc<FakeConfig>,
    clip: Arc<FakeClipboard>,
    keys: Arc<FakeKeys>,
    capture: Arc<FakeCapture>,
    picker: Arc<FakePicker>,
    ocr: Arc<TestOcr>,
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

fn snapshot(keys: &[ProviderType], fallbacks: &[(ProviderType, ProviderType)]) -> ConfigSnapshot {
    ConfigSnapshot {
        magic_prompt: String::new(),
        fallbacks: fallbacks.iter().copied().collect(),
        connections: default_providers(),
        api_keys: keys.iter().map(|&p| (p, "key-FIXTURE".into())).collect(),
    }
}

fn ocr_ok(text: &str) -> Result<OcrText, PlatformError> {
    Ok(OcrText {
        text: text.into(),
        language: Some("en-US".into()),
    })
}

fn rig_full(
    pick: Result<Option<Rect>, PlatformError>,
    ocr: Result<OcrText, PlatformError>,
    transport: Arc<dyn Transport>,
    keys_for: &[ProviderType],
    fallbacks: &[(ProviderType, ProviderType)],
) -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    let capture = Arc::new(FakeCapture::new([10, 20, 30, 255]));
    let picker = Arc::new(FakePicker::new(pick));
    let ocr = Arc::new(TestOcr {
        inner: FakeOcr::new(ocr),
        threads: Mutex::new(Vec::new()),
        after: Mutex::new(None),
    });
    let platform = Platform {
        clipboard: clip.clone(),
        keys: keys.clone(),
        foreground: FakeForeground::new(Some(WindowHandle(7))),
        monitors: Arc::new(FakeMonitors::new(Point { x: 5, y: 6 }, monitor_list())),
        capture: capture.clone(),
        ocr: ocr.clone(),
        protector: Arc::new(PlainProtector),
        sleeper,
    };
    let registry = Arc::new(ProviderRegistry::standard());
    let popup = Arc::new(FakePopup::new());
    let persist = Arc::new(FakePersist::new());
    let config = Arc::new(FakeConfig::new(
        Settings::default(),
        snapshot(keys_for, fallbacks),
    ));
    let orch = Orchestrator::new(Deps {
        platform,
        runner: Arc::new(OperationRunner::new(
            ProviderRegistry::standard(),
            transport,
        )),
        registry_has: Arc::new(move |p| registry.get(p).is_ok()),
        config: config.clone(),
        stores: persist.clone(),
        popup: popup.clone(),
        gate: PlatformGate::new(Default::default()),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.0))),
        log: None,
        picker: picker.clone(),
    });
    Rig {
        orch,
        popup,
        persist,
        config,
        clip,
        keys,
        capture,
        picker,
        ocr,
    }
}

const KEYED: [ProviderType; 4] = [
    ProviderType::Claude,
    ProviderType::OpenAI,
    ProviderType::Gemini,
    ProviderType::Grok,
];

fn rig(ocr: Result<OcrText, PlatformError>, transport: Arc<FakeTransport>) -> Rig {
    rig_full(Ok(Some(REGION)), ocr, transport, &KEYED, &[])
}

fn ok_transport() -> Arc<FakeTransport> {
    Arc::new(FakeTransport::json(200, CLAUDE_OK))
}

fn models(r: &Rig) -> Vec<PopupViewModel> {
    r.popup
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            PopupCall::ShowAt(_, m) | PopupCall::Update(m) => Some(m),
            PopupCall::Hide => None,
        })
        .collect()
}

fn last_model(r: &Rig) -> PopupViewModel {
    models(r).pop().expect("a popup model")
}

fn error_key(m: &PopupViewModel) -> String {
    match &m.phase {
        PopupPhase::Error(e) => e.message.key.clone(),
        other => panic!("not an error: {other:?}"),
    }
}

fn clip_seq(r: &Rig) -> u64 {
    r.clip.sequence_number()
}

#[tokio::test]
async fn cancelled_pick_shows_nothing() {
    let r = rig_full(Ok(None), ocr_ok(OCR_TEXT), ok_transport(), &KEYED, &[]);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.picker.calls(), 1);
    assert!(r.popup.calls().is_empty());
    assert!(r.capture.regions().is_empty());
    assert!(r.ocr.inner.images().is_empty());
}

#[tokio::test]
async fn pick_error_shows_message() {
    let r = rig_full(
        Err(PlatformError::Unsupported("picker")),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let calls = r.popup.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let PopupCall::ShowAt(origin, m) = &calls[0] else {
        panic!("expected show_at: {calls:?}")
    };
    assert_eq!(error_key(m), KEY_PLATFORM_GENERIC);
    assert_eq!(
        *origin,
        popup_origin(INITIAL_POPUP_SIZE, Point { x: 5, y: 6 }, &monitor_list())
    );
    assert!(r.capture.regions().is_empty());
}

#[tokio::test]
async fn invalid_region_shows_selection_too_small() {
    let r = rig_full(
        Ok(Some(Rect::new(5000, 5000, 100, 100))),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_SELECTION_TOO_SMALL);
    assert!(r.capture.regions().is_empty());
    assert!(r.ocr.inner.images().is_empty());
    assert!(!r.orch.has_captured_image());
}

#[tokio::test]
async fn capture_uses_clipped_region() {
    let r = rig_full(
        Ok(Some(Rect::new(1800, 1000, 400, 300))),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.capture.regions(), vec![Rect::new(1800, 1000, 120, 80)]);
    assert_eq!(r.ocr.inner.images(), vec![(120, 80)]);
    let calls = r.popup.calls();
    let PopupCall::ShowAt(origin, loading) = &calls[0] else {
        panic!("first call must be show_at: {calls:?}")
    };
    assert!(matches!(
        &loading.phase,
        PopupPhase::Loading { operation: AIOperation::ScreenTranslate, preview } if preview.is_empty()
    ));
    // Anchored at the bottom-right of the clipped rect.
    assert_eq!(
        *origin,
        popup_origin(
            INITIAL_POPUP_SIZE,
            Point { x: 1920, y: 1080 },
            &monitor_list()
        )
    );
}

#[tokio::test]
async fn ocr_success_shows_ocr_phase_and_auto_copies() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    let before = clip_seq(&r);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    let PopupPhase::Ocr {
        text,
        translate_providers,
        active_translate_provider,
        ..
    } = &m.phase
    else {
        panic!("expected the ocr phase: {m:?}")
    };
    assert_eq!(text, OCR_TEXT);
    assert!(translate_providers
        .iter()
        .any(|p| p.provider == ProviderType::Claude));
    assert_eq!(*active_translate_provider, Some(ProviderType::Claude));
    assert!(!m.retry_providers.is_empty());
    assert!(m
        .retry_providers
        .iter()
        .all(|p| p.provider != ProviderType::GoogleFree));
    assert_ne!(clip_seq(&r), before);
    assert_eq!(r.clip.read_text().unwrap().as_deref(), Some(OCR_TEXT));
    assert!(r.persist.history().is_empty());
    assert!(r.orch.has_captured_image());
}

#[tokio::test]
async fn auto_copy_off_does_not_touch_clipboard() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.config.update_settings(|s| s.auto_copy_ocr = false);
    let before = clip_seq(&r);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(matches!(last_model(&r).phase, PopupPhase::Ocr { .. }));
    assert_eq!(clip_seq(&r), before);
}

#[tokio::test]
async fn ocr_empty_offers_vision_retry() {
    let r = rig(ocr_ok("  \n "), ok_transport());
    let before = clip_seq(&r);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    assert_eq!(error_key(&m), KEY_NO_TEXT_FOUND_RETRY);
    assert!(!m.retry_providers.is_empty());
    assert_eq!(clip_seq(&r), before);
    assert!(r.orch.has_captured_image());
}

#[tokio::test]
async fn ocr_empty_without_vision_providers_has_no_retry() {
    let r = rig_full(Ok(Some(REGION)), ocr_ok(""), ok_transport(), &[], &[]);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    assert_eq!(error_key(&m), KEY_NO_TEXT_FOUND);
    assert!(m.retry_providers.is_empty());
}

#[tokio::test]
async fn ocr_unavailable_message() {
    let r = rig(Err(PlatformError::OcrUnavailable), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    assert_eq!(error_key(&m), KEY_OCR_UNAVAILABLE);
    assert!(!m.retry_providers.is_empty());
}

#[tokio::test]
async fn translate_ocr_hides_replace_and_auto_copies_result() {
    let t = ok_transport();
    let r = rig(ocr_ok(OCR_TEXT), t.clone());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::Claude).await;
    let all = models(&r);
    assert!(matches!(
        &all[all.len() - 2].phase,
        PopupPhase::Loading { operation: AIOperation::ScreenTranslate, preview } if preview == OCR_TEXT
    ));
    let PopupPhase::Result(v) = &last_model(&r).phase else {
        panic!("expected a result")
    };
    assert_eq!(v.text, "TRANSLATED");
    assert!(!v.show_replace);
    assert_eq!(v.operation, AIOperation::ScreenTranslate);
    assert_eq!(r.clip.read_text().unwrap().as_deref(), Some("TRANSLATED"));
    assert_eq!(r.persist.history().len(), 1);
    assert_eq!(r.persist.history()[0].0.original, OCR_TEXT);
    assert_eq!(t.requests().len(), 1);
}

#[tokio::test]
async fn translate_ocr_without_auto_copy_leaves_clipboard() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.config.update_settings(|s| s.auto_copy_ocr = false);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let before = clip_seq(&r);
    r.orch.translate_ocr(ProviderType::Claude).await;
    assert!(matches!(last_model(&r).phase, PopupPhase::Result(_)));
    assert_eq!(clip_seq(&r), before);
}

fn hosts() -> Arc<FakeTransport> {
    Arc::new(FakeTransport::by_host(vec![
        ("api.anthropic.com", HttpResponse::new(500, b"{}")),
        (
            "api.openai.com",
            HttpResponse::new(200, OPENAI_OK.as_bytes()),
        ),
    ]))
}

#[tokio::test]
async fn translate_ocr_fallback_only_for_active_provider() {
    // Active provider (Claude) fails with a 5xx: falls back to OpenAI.
    let t = hosts();
    let r = rig_full(
        Ok(Some(REGION)),
        ocr_ok(OCR_TEXT),
        t.clone(),
        &KEYED,
        &[(ProviderType::Claude, ProviderType::OpenAI)],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::Claude).await;
    let PopupPhase::Result(v) = &last_model(&r).phase else {
        panic!("expected a fallback result: {:?}", last_model(&r))
    };
    assert_eq!(v.text, "VIA-OPENAI");
    assert!(v.via_fallback);
    assert_eq!(t.requests().len(), 2);

    // A non-active provider never falls back.
    let t = Arc::new(FakeTransport::by_host(vec![
        (
            "api.anthropic.com",
            HttpResponse::new(200, CLAUDE_OK.as_bytes()),
        ),
        ("api.openai.com", HttpResponse::new(500, b"{}")),
    ]));
    let r = rig_full(
        Ok(Some(REGION)),
        ocr_ok(OCR_TEXT),
        t.clone(),
        &KEYED,
        &[(ProviderType::OpenAI, ProviderType::Claude)],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::OpenAI).await;
    assert!(matches!(last_model(&r).phase, PopupPhase::Error(_)));
    assert_eq!(t.requests().len(), 1);
    assert!(r.persist.history().is_empty());
}

#[tokio::test]
async fn translate_ocr_without_ocr_text_does_nothing() {
    let t = ok_transport();
    let r = rig(Err(PlatformError::OcrUnavailable), t.clone());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let before = r.popup.calls().len();
    r.orch.translate_ocr(ProviderType::Claude).await;
    assert_eq!(r.popup.calls().len(), before);
    assert!(t.requests().is_empty());
}

#[tokio::test]
async fn vision_retry_sends_png_and_never_falls_back() {
    // Success: the request carries the base64 PNG, the result hides Replace and is persisted.
    let t = ok_transport();
    let r = rig(Err(PlatformError::OcrUnavailable), t.clone());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.retry(ProviderType::Claude).await;
    let reqs = t.requests();
    assert_eq!(reqs.len(), 1);
    let body = String::from_utf8(reqs[0].body.clone().unwrap()).unwrap();
    // base64 of the PNG signature 89 50 4E 47 0D 0A 1A 0A.
    assert!(body.contains("iVBORw0KGgo"), "no PNG in the request body");
    let PopupPhase::Result(v) = &last_model(&r).phase else {
        panic!("expected a result")
    };
    assert_eq!(v.text, "TRANSLATED");
    assert!(!v.show_replace);
    assert_eq!(r.persist.history().len(), 1);
    // auto-copy of the translated text
    assert_eq!(r.clip.read_text().unwrap().as_deref(), Some("TRANSLATED"));

    // A 429 from the vision provider with a fallback configured: error, a single request.
    let t = Arc::new(FakeTransport::json(429, "{}"));
    let r = rig_full(
        Ok(Some(REGION)),
        Err(PlatformError::OcrUnavailable),
        t.clone(),
        &KEYED,
        &[(ProviderType::OpenAI, ProviderType::Claude)],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.retry(ProviderType::OpenAI).await;
    let m = last_model(&r);
    assert!(matches!(m.phase, PopupPhase::Error(_)));
    assert!(m
        .retry_providers
        .iter()
        .all(|p| p.provider != ProviderType::OpenAI));
    assert!(!m.retry_providers.is_empty());
    assert_eq!(t.requests().len(), 1);
    assert!(r.persist.history().is_empty());
}

#[tokio::test]
async fn vision_retry_from_translate_error_uses_the_image() {
    let t = Arc::new(FakeTransport::by_host(vec![
        ("api.anthropic.com", HttpResponse::new(500, b"{}")),
        (
            "api.openai.com",
            HttpResponse::new(200, OPENAI_OK.as_bytes()),
        ),
    ]));
    let r = rig(ocr_ok(OCR_TEXT), t.clone());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::Claude).await;
    assert!(matches!(last_model(&r).phase, PopupPhase::Error(_)));
    r.orch.retry(ProviderType::OpenAI).await;
    let body = String::from_utf8(t.requests().last().unwrap().body.clone().unwrap()).unwrap();
    assert!(body.contains("data:image/png;base64,"), "{body:.200}");
    let PopupPhase::Result(v) = &last_model(&r).phase else {
        panic!("expected a result")
    };
    assert_eq!(v.text, "VIA-OPENAI");
    // The original is the OCR text.
    assert_eq!(r.persist.history().last().unwrap().0.original, OCR_TEXT);
}

#[tokio::test]
async fn closing_drops_the_image() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(r.orch.has_captured_image());
    r.orch.escape().await;
    assert!(!r.orch.has_captured_image());
}

#[tokio::test]
async fn text_operation_drops_image() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(r.orch.has_captured_image());
    // Nothing selected: the text flow fails early but still drops the image.
    r.keys.on_copy(CopyBehavior::NothingSelected);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_NOTHING_SELECTED);
    assert!(!r.orch.has_captured_image());

    // Rewrite / explain / retranslate of a screen result drop it too.
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::Claude).await;
    assert!(r.orch.has_captured_image());
    r.orch.rewrite().await;
    assert!(!r.orch.has_captured_image());
}

#[tokio::test(flavor = "current_thread")]
async fn ocr_runs_off_the_async_thread() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let here = std::thread::current().id();
    let threads = r.ocr.threads.lock().unwrap().clone();
    assert_eq!(threads.len(), 1);
    assert!(threads.iter().all(|&id| id != here));
}

#[tokio::test]
async fn screen_translate_repositions_visible_popup() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(matches!(r.popup.calls().last(), Some(PopupCall::Update(_))));
    let second = Rect {
        x: 900,
        y: 500,
        width: 100,
        height: 40,
    };
    r.picker.set_result(Ok(Some(second)));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let anchor = Point {
        x: second.right(),
        y: second.bottom(),
    };
    let want = popup_origin(INITIAL_POPUP_SIZE, anchor, &monitor_list());
    let shows: Vec<(Point, PopupViewModel)> = r
        .popup
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            PopupCall::ShowAt(o, m) => Some((o, m)),
            _ => None,
        })
        .collect();
    assert_eq!(shows.len(), 2, "{shows:?}");
    assert_eq!(shows[1].0, want);
    assert!(matches!(shows[1].1.phase, PopupPhase::Loading { .. }));
}

#[tokio::test]
async fn vision_retry_keeps_detected_source_in_history() {
    let t = Arc::new(FakeTransport::by_host(vec![
        ("api.anthropic.com", HttpResponse::new(500, b"{}")),
        (
            "api.openai.com",
            HttpResponse::new(200, OPENAI_OK.as_bytes()),
        ),
    ]));
    let r = rig(
        ocr_ok("Привет, как твои дела сегодня? Я надеюсь, что у тебя всё хорошо."),
        t,
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.orch.translate_ocr(ProviderType::Claude).await;
    r.orch.retry(ProviderType::OpenAI).await;
    let entry = r.persist.history().last().unwrap().0.clone();
    assert_eq!(entry.source_language.as_deref(), Some("Russian"));
}

/// Occupies the only blocking-pool thread right after OCR so the auto-copy job is queued
/// behind it, cancels the flow, then lets the stale job run.
#[test]
fn superseded_auto_copy_never_writes_clipboard() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let r = rig(ocr_ok(OCR_TEXT), ok_transport());
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(Some(release_rx));
        *r.ocr.after.lock().unwrap() = Some(Box::new(move || {
            let rx = release_rx.lock().unwrap().take().unwrap();
            // Queued behind the running OCR job, ahead of the auto-copy job.
            tokio::task::spawn_blocking(move || {
                let _ = started_tx.send(());
                let _ = rx.recv();
            });
        }));
        let before = clip_seq(&r);
        let orch = r.orch.clone();
        let flow = tokio::spawn(async move { orch.trigger(AIOperation::ScreenTranslate).await });
        started_rx.recv().await.unwrap();
        // Let the flow reach the auto-copy gate job (queued behind the blocker).
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        r.orch.escape().await;
        release_tx.send(()).unwrap();
        let _ = flow.await;
        // FIFO on the single blocking thread: runs after the stale job.
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        assert_eq!(clip_seq(&r), before, "stale text reached the clipboard");
    });
}
