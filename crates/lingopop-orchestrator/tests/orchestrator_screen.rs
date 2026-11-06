//! The Screen Translate flow over platform fakes, a scripted region picker and a `FakeTransport`.

#[path = "support/presentation_transport.rs"]
mod presentation_transport;
use lingopop_platform::fake::{DismissCall, FakeDismissWatcher, FakePopupWindow};
use presentation_transport::AfterPresentation;
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
    KEY_PLATFORM_GENERIC, KEY_SCREEN_TIMEOUT, KEY_SELECTION_TOO_SMALL,
};
use lingopop_orchestrator::{
    popup_origin, Deps, GateTimeouts, Orchestrator, PlatformGate, PopupPhase, PopupViewModel,
    INITIAL_POPUP_SIZE,
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
    popup: Arc<FakePopup>,
}

impl Ocr for TestOcr {
    fn recognize(&self, image: &CapturedImage) -> Result<OcrText, PlatformError> {
        self.popup.flush();
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
    logs: Arc<Mutex<Vec<String>>>,
    status: Arc<lingopop_orchestrator::fake::FakeStatus>,
    orch: Orchestrator,
    popup: Arc<FakePopup>,
    dismiss: Arc<FakeDismissWatcher>,
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
    rig_gated(
        pick,
        ocr,
        transport,
        keys_for,
        fallbacks,
        GateTimeouts::default(),
    )
}

fn rig_gated(
    pick: Result<Option<Rect>, PlatformError>,
    ocr: Result<OcrText, PlatformError>,
    transport: Arc<dyn Transport>,
    keys_for: &[ProviderType],
    fallbacks: &[(ProviderType, ProviderType)],
    gate: GateTimeouts,
) -> Rig {
    rig_with_picker(pick, ocr, transport, keys_for, fallbacks, gate, None)
}

/// `custom` replaces the scripted `FakePicker` inside `Deps` (the `Rig.picker` stays unused).
fn rig_with_picker(
    pick: Result<Option<Rect>, PlatformError>,
    ocr: Result<OcrText, PlatformError>,
    transport: Arc<dyn Transport>,
    keys_for: &[ProviderType],
    fallbacks: &[(ProviderType, ProviderType)],
    gate: GateTimeouts,
    custom: Option<Arc<dyn lingopop_orchestrator::RegionPicker>>,
) -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    let capture = Arc::new(FakeCapture::new([10, 20, 30, 255]));
    let picker = Arc::new(FakePicker::new(pick));
    let popup = Arc::new(FakePopup::new());
    let ocr = Arc::new(TestOcr {
        popup: popup.clone(),
        inner: FakeOcr::new(ocr),
        threads: Mutex::new(Vec::new()),
        after: Mutex::new(None),
    });
    let dismiss = Arc::new(FakeDismissWatcher::default());
    let platform = Platform {
        overlay: Arc::new(lingopop_platform::fake::FakeOverlay::new(Ok(None))),
        system: Arc::new(lingopop_platform::fake::FakeSystem::new(vec![])),
        login_item: Arc::new(lingopop_platform::fake::FakeLoginItem::new()),
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: dismiss.clone(),
        clipboard: clip.clone(),
        keys: keys.clone(),
        foreground: FakeForeground::new(Some(WindowHandle(7))),
        last_foreground: lingopop_platform::fake::FakeLastForeground::new(None),
        monitors: Arc::new(FakeMonitors::new(Point { x: 5, y: 6 }, monitor_list())),
        capture: capture.clone(),
        ocr: ocr.clone(),
        permissions: Arc::new(lingopop_platform::AlwaysGranted),
        protector: Arc::new(PlainProtector),
        sleeper,
    };
    let registry = Arc::new(ProviderRegistry::standard());
    let persist = Arc::new(FakePersist::new());
    // Written against Claude as the active provider (the Windows default is Google (Free)).
    let settings = Settings {
        active_provider: ProviderType::Claude,
        ..Settings::default()
    };
    let config = Arc::new(FakeConfig::new(settings, snapshot(keys_for, fallbacks)));
    let status = Arc::new(lingopop_orchestrator::fake::FakeStatus::default());
    let logs = Arc::new(Mutex::new(Vec::new()));
    let log_lines = logs.clone();
    let popup_barrier = popup.clone();
    let transport = Arc::new(AfterPresentation {
        transport,
        flush: Arc::new(move || popup_barrier.flush()),
    });
    let orch = Orchestrator::new(Deps {
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
        gate: PlatformGate::new(gate),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.0))),
        log: Some(Arc::new(move |line| {
            log_lines.lock().unwrap().push(line.to_string())
        })),
        picker: custom.unwrap_or_else(|| picker.clone()),
    });
    Rig {
        logs,
        status,
        orch,
        popup,
        dismiss,
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
            PopupCall::Hide | PopupCall::Resize(..) => None,
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
async fn pick_os_error_logs_failure_and_shows_generic_message() {
    let r = rig_full(
        Err(PlatformError::Os {
            api: "overlay thread",
            code: 0,
        }),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let logs = r.logs.lock().unwrap().clone();
    assert!(logs
        .iter()
        .any(|line| line == "screen translate: region picker failed"));
    assert!(!logs.iter().any(|line| line == "region picker unsupported"));
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
async fn ocr_native_failure_names_the_recognition_stage_on_macos() {
    let r = rig(
        Err(PlatformError::Os {
            api: "VNImageRequestHandler.performRequests",
            code: 0,
        }),
        ok_transport(),
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    let expected = if cfg!(target_os = "macos") {
        lingopop_orchestrator::messages::KEY_TEXT_RECOGNITION_FAILED
    } else {
        lingopop_orchestrator::messages::KEY_PLATFORM_GENERIC
    };
    assert_eq!(error_key(&m), expected);
    // The vision retry stays on offer: a different provider may still read the image.
    assert!(!m.retry_providers.is_empty());
}

#[tokio::test]
async fn capture_native_failure_names_the_capture_stage_on_macos() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    // A display layout that changed during the capture (macOS adapter) or any native failure.
    r.capture.fail_next(PlatformError::Os {
        api: "display topology changed",
        code: 0,
    });
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    let expected = if cfg!(target_os = "macos") {
        lingopop_orchestrator::messages::KEY_SCREEN_CAPTURE_FAILED
    } else {
        lingopop_orchestrator::messages::KEY_PLATFORM_GENERIC
    };
    assert_eq!(error_key(&m), expected);
    assert_ne!(
        error_key(&m),
        lingopop_orchestrator::messages::KEY_SELECTION_TOO_SMALL,
        "a failed capture is not a too-small selection"
    );
    assert_eq!(r.capture.regions().len(), 1, "the capture was attempted");
}

#[tokio::test]
async fn the_pick_revision_travels_to_the_capture() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.picker.set_revision(Some(7));
    r.capture.set_live_revision(Some(7));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.capture.revisions(), vec![Some(7)]);
    assert_eq!(
        r.ocr.threads.lock().unwrap().len(),
        1,
        "capture succeeded, OCR ran"
    );
}

#[tokio::test]
async fn a_topology_change_between_pick_and_capture_fails_the_capture() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.picker.set_revision(Some(7));
    r.capture.set_live_revision(Some(8)); // the layout changed after the pick
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    let expected = if cfg!(target_os = "macos") {
        lingopop_orchestrator::messages::KEY_SCREEN_CAPTURE_FAILED
    } else {
        lingopop_orchestrator::messages::KEY_PLATFORM_GENERIC
    };
    assert_eq!(error_key(&m), expected);
    assert_ne!(
        error_key(&m),
        lingopop_orchestrator::messages::KEY_SELECTION_TOO_SMALL
    );
    assert_eq!(r.capture.topology_checks(), vec![Some(7)]);
    assert!(
        r.capture.regions().is_empty(),
        "the stale pick is refused before any capture"
    );
    assert!(
        r.ocr.threads.lock().unwrap().is_empty(),
        "no OCR of a stale rect"
    );
}

#[tokio::test]
async fn a_stale_pick_outside_the_current_monitors_is_still_a_failed_capture() {
    // The pick was on a display that is gone: its rect lies outside every current monitor, so
    // clipping alone would say InvalidRegion ("selection too small"). The revision wins.
    let outside = Rect::new(5000, 4000, 200, 50);
    let r = rig_full(
        Ok(Some(outside)),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.picker.set_revision(Some(7));
    r.capture.set_live_revision(Some(8));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    let m = last_model(&r);
    let expected = if cfg!(target_os = "macos") {
        lingopop_orchestrator::messages::KEY_SCREEN_CAPTURE_FAILED
    } else {
        lingopop_orchestrator::messages::KEY_PLATFORM_GENERIC
    };
    assert_eq!(error_key(&m), expected);
    assert_ne!(
        error_key(&m),
        lingopop_orchestrator::messages::KEY_SELECTION_TOO_SMALL
    );
    assert_eq!(r.capture.topology_checks(), vec![Some(7)]);
    assert!(r.capture.regions().is_empty(), "zero native capture calls");
    assert!(r.ocr.threads.lock().unwrap().is_empty(), "zero OCR calls");
}

#[tokio::test]
async fn an_outside_pick_without_a_revision_is_still_a_small_selection() {
    // Windows / None path unchanged: clipping decides.
    let outside = Rect::new(5000, 4000, 200, 50);
    let r = rig_full(
        Ok(Some(outside)),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(
        error_key(&last_model(&r)),
        lingopop_orchestrator::messages::KEY_SELECTION_TOO_SMALL
    );
    assert!(r.capture.regions().is_empty());
}

#[tokio::test]
async fn a_pick_without_a_revision_captures_unchecked() {
    // Windows: no revision to carry.
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.capture.set_live_revision(Some(8));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.capture.revisions(), vec![None]);
    assert_eq!(r.ocr.threads.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn ocr_gate_timeout_uses_the_screen_message() {
    let r = rig_gated(
        Ok(Some(REGION)),
        ocr_ok(OCR_TEXT),
        ok_transport(),
        &KEYED,
        &[],
        GateTimeouts {
            acquire: std::time::Duration::from_secs(2),
            run: std::time::Duration::from_millis(50),
        },
    );
    *r.ocr.after.lock().unwrap() = Some(Box::new(|| {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_SCREEN_TIMEOUT);
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

#[tokio::test]
async fn reposition_updates_dismiss_rect_instead_of_restarting() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.popup.flush();
    let initial_calls = r.dismiss.calls();
    assert!(matches!(initial_calls.as_slice(), [DismissCall::Start(_)]));
    r.orch.popup_resized((700.0, 400.0));
    r.popup.flush();
    let Some(DismissCall::UpdateRect(resized)) = r.dismiss.calls().last().cloned() else {
        panic!("expected UpdateRect");
    };
    r.picker.set_result(Ok(Some(Rect::new(900, 500, 100, 40))));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    r.popup.flush();
    // The reposition uses the measured size, not the initial one.
    let origin = popup_origin((700.0, 400.0), Point { x: 1000, y: 540 }, &monitor_list());
    assert_eq!(
        &r.dismiss.calls()[1..],
        &[
            DismissCall::UpdateRect(resized),
            DismissCall::UpdateRect(Rect::new(origin.x, origin.y, 700, 400)),
        ]
    );
}

#[tokio::test]
async fn failed_reposition_hides_popup_and_stops_dismissal() {
    let r = rig(ocr_ok(OCR_TEXT), ok_transport());
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(r.popup.is_visible());
    r.popup.fail_show(true);
    r.picker.set_result(Ok(Some(Rect::new(900, 500, 100, 40))));
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(!r.popup.is_visible());
    assert_eq!(r.dismiss.calls().last(), Some(&DismissCall::Stop));
}

#[tokio::test]
async fn screen_translate_working_starts_after_pick() {
    use lingopop_orchestrator::TrayStatus::{Idle, Working};
    let cancelled = rig_full(
        Ok(None),
        ocr_ok(OCR_TEXT),
        Arc::new(FakeTransport::json(200, CLAUDE_OK)),
        &[ProviderType::Claude],
        &[],
    );
    cancelled.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(cancelled.status.statuses(), vec![Idle]);
    let picked = rig_full(
        Ok(Some(REGION)),
        ocr_ok(OCR_TEXT),
        Arc::new(FakeTransport::json(200, CLAUDE_OK)),
        &[ProviderType::Claude],
        &[],
    );
    assert!(picked.status.statuses().is_empty());
    picked.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(picked.status.statuses().first(), Some(&Working));
    assert_eq!(picked.status.statuses().last(), Some(&Idle));
}

#[tokio::test]
async fn unsupported_pick_is_a_silent_cancel() {
    let r = rig_full(
        Err(PlatformError::Unsupported("region overlay on macOS")),
        ocr_ok(OCR_TEXT),
        Arc::new(FakeTransport::json(200, CLAUDE_OK)),
        &KEYED,
        &[],
    );
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert!(r.popup.calls().is_empty());
    assert_eq!(
        r.status.statuses().last(),
        Some(&lingopop_orchestrator::TrayStatus::Idle)
    );
    assert!(r.capture.regions().is_empty());
    assert!(r
        .logs
        .lock()
        .unwrap()
        .iter()
        .any(|line| line == "region picker unsupported"));
}

#[tokio::test]
async fn second_screen_translate_cancels_first_pick() {
    let r = rig(
        ocr_ok(OCR_TEXT),
        Arc::new(FakeTransport::json(200, CLAUDE_OK)),
    );
    r.picker.block_forever();
    let orch = r.orch.clone();
    let first = tokio::spawn(async move { orch.trigger(AIOperation::ScreenTranslate).await });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while r.picker.calls() < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let orch = r.orch.clone();
    let second = tokio::spawn(async move { orch.trigger(AIOperation::ScreenTranslate).await });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while r.picker.cancelled() < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    r.orch.close().await;
    first.await.unwrap();
    second.await.unwrap();
}

/// The orchestrator over the REAL macOS overlay lifecycle (`MacOverlay`) on a fake native
/// backend: a second Screen Translate cancels the first pick, and the second overlay is
/// created only after the first one's native cleanup finished (never two native overlays).
mod mac_overlay_lifecycle {
    use super::*;
    use async_trait::async_trait;
    use lingopop_orchestrator::RegionPicker;
    use lingopop_platform::fake::{FakeMainThread, FakeOverlayBackend};
    use lingopop_platform::macos_geometry::{MacDisplay, MacGeometry, NativeRect, TopologyCache};
    use lingopop_platform::macos_overlay::MacOverlay;
    use lingopop_platform::RegionOverlay;

    /// Bridges the blocking overlay like the shell's `OverlayPicker`: one serial guard held by
    /// the blocking worker, the cancel sender dropped when the awaiting future is dropped.
    struct BridgePicker {
        overlay: Arc<dyn RegionOverlay>,
        done: Arc<tokio::sync::Mutex<()>>,
    }
    #[async_trait]
    impl RegionPicker for BridgePicker {
        async fn pick(&self) -> Result<Option<Rect>, PlatformError> {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let guard = self.done.clone().lock_owned().await;
            let overlay = self.overlay.clone();
            let out = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                overlay.pick(rx)
            })
            .await
            .map_err(|_| PlatformError::Os {
                api: "overlay worker join",
                code: 0,
            })?;
            drop(tx);
            out
        }
    }

    async fn wait(f: impl Fn() -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !f() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn restarted_screen_translate_waits_for_overlay_cleanup() {
        let bounds = NativeRect::new(0.0, 0.0, 1920.0, 1080.0);
        let cache = TopologyCache::new(
            MacGeometry::new(
                1,
                1080.0,
                vec![MacDisplay {
                    id: 1,
                    desktop_bounds: bounds,
                    work_area: bounds,
                    backing_scale: 1.0,
                    primary: true,
                }],
            )
            .unwrap(),
        );
        let main = FakeMainThread::new();
        let backend = FakeOverlayBackend::new();
        let overlay = Arc::new(MacOverlay::new(
            backend.clone(),
            Arc::new(cache),
            Arc::new(FakeSleeper::new()),
        ));
        overlay.bind_main_thread(main.clone());
        let pump = {
            let m = main.clone();
            std::thread::spawn(move || m.pump())
        };
        backend.hold_cleanup(true);
        let picker = Arc::new(BridgePicker {
            overlay,
            done: Arc::new(tokio::sync::Mutex::new(())),
        });
        let r = rig_with_picker(
            Ok(None),
            ocr_ok(OCR_TEXT),
            Arc::new(FakeTransport::json(200, CLAUDE_OK)),
            &KEYED,
            &[],
            GateTimeouts::default(),
            Some(picker),
        );
        let orch = r.orch.clone();
        let first = tokio::spawn(async move { orch.trigger(AIOperation::ScreenTranslate).await });
        wait(|| backend.creates() == 1).await;
        // The restart cancels the first flow (its picker future is dropped) ...
        let orch = r.orch.clone();
        let second = tokio::spawn(async move { orch.trigger(AIOperation::ScreenTranslate).await });
        // (a blocking wait would starve the current-thread runtime that cancels the first flow)
        wait(|| {
            backend.calls().iter().any(|c| {
                matches!(
                    c,
                    lingopop_platform::fake::OverlayCall::CleanupStarted { .. }
                )
            })
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        // ... but nothing new is created while the first overlay is still being cleaned up.
        assert_eq!(backend.creates(), 1);
        backend.hold_cleanup(false);
        wait(|| backend.creates() == 2).await;
        assert_eq!(backend.max_native(), 1);
        assert!(backend.cleanup_seq(1).unwrap() < backend.create_seq(2).unwrap());
        r.orch.close().await;
        let _ = first.await;
        let _ = second.await;
        wait(|| backend.native_active() == 0).await;
        main.close();
        pump.join().unwrap();
    }
}
