//! Permission handling of the text and Screen Translate flows over platform fakes: a prompt only
//! for a `NotDetermined` permission of the operation's own kind, a `PermissionDenied` error with
//! the per-kind message otherwise, and no cached denial once the permission is granted.

use std::sync::{Arc, Mutex};

use lingopop_core::http::FakeTransport;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::{default_providers, ProviderType};
use lingopop_core::providers::ProviderRegistry;
use lingopop_core::runner::{ConfigSnapshot, OperationRunner};
use lingopop_core::secrets::{ProtectError, SecretProtector};
use lingopop_core::settings::Settings;
use lingopop_core::time::{ManualClock, Timestamp};
use lingopop_orchestrator::fake::{FakeConfig, FakePersist, FakePicker, FakePopup, PopupCall};
use lingopop_orchestrator::messages::{
    KEY_INPUT_BLOCKED, KEY_PERMISSION_ACCESSIBILITY, KEY_PERMISSION_SCREEN_RECORDING,
};
use lingopop_orchestrator::{
    Deps, GateTimeouts, Orchestrator, PlatformGate, PopupPhase, PopupViewModel,
};
use lingopop_platform::fake::{
    CopyBehavior, FakeCapture, FakeClipboard, FakeDismissWatcher, FakeForeground, FakeKeys,
    FakeLastForeground, FakeMonitors, FakeOcr, FakePermissions, FakePopupWindow, FakeSleeper,
};
use lingopop_platform::{
    Clipboard, MonitorInfo, OcrText, PermissionKind, PermissionState, Platform, Point, Rect,
    WindowHandle,
};

const CLAUDE_OK: &str = r#"{"content":[{"type":"text","text":"TRANSLATED"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#;
const SECRET: &str = "SECRET-SELECTION-TEXT";
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

struct Rig {
    orch: Orchestrator,
    popup: Arc<FakePopup>,
    permissions: Arc<FakePermissions>,
    clip: Arc<FakeClipboard>,
    keys: Arc<FakeKeys>,
    capture: Arc<FakeCapture>,
    /// What the capture service itself sees (it re-checks, like macOS does at capture time).
    capture_permissions: Arc<FakePermissions>,
    picker: Arc<FakePicker>,
    transport: Arc<FakeTransport>,
    logs: Arc<Mutex<Vec<String>>>,
}

fn rig() -> Rig {
    let sleeper = Arc::new(FakeSleeper::new());
    let clip = FakeClipboard::new(sleeper.clone());
    let keys = FakeKeys::new(clip.clone(), sleeper.clone());
    keys.on_copy(CopyBehavior::Select(SECRET.to_string()));
    let permissions = FakePermissions::new();
    let capture_permissions = FakePermissions::new();
    let capture =
        Arc::new(FakeCapture::new([10, 20, 30, 255]).with_permissions(capture_permissions.clone()));
    let picker = Arc::new(FakePicker::new(Ok(Some(REGION))));
    let bounds = Rect::new(0, 0, 1920, 1080);
    let monitors = vec![MonitorInfo {
        bounds,
        work_area: bounds,
        scale_factor: 1.0,
        primary: true,
    }];
    let platform = Platform {
        overlay: Arc::new(lingopop_platform::fake::FakeOverlay::new(Ok(None))),
        system: Arc::new(lingopop_platform::fake::FakeSystem::new(vec![])),
        login_item: Arc::new(lingopop_platform::fake::FakeLoginItem::new()),
        popup_window: Arc::new(FakePopupWindow::default()),
        dismiss: Arc::new(FakeDismissWatcher::default()),
        clipboard: clip.clone(),
        keys: keys.clone(),
        foreground: FakeForeground::new(Some(WindowHandle(7))),
        last_foreground: FakeLastForeground::new(None),
        monitors: Arc::new(FakeMonitors::new(Point { x: 5, y: 6 }, monitors)),
        capture: capture.clone(),
        ocr: Arc::new(FakeOcr::new(Ok(OcrText {
            text: "Hello world".into(),
            language: Some("en-US".into()),
        }))),
        permissions: permissions.clone(),
        protector: Arc::new(PlainProtector),
        sleeper,
    };
    let registry = Arc::new(ProviderRegistry::standard());
    let transport = Arc::new(FakeTransport::json(200, CLAUDE_OK));
    let settings = Settings {
        active_provider: ProviderType::Claude,
        ..Settings::default()
    };
    let snapshot = ConfigSnapshot {
        magic_prompt: String::new(),
        fallbacks: Default::default(),
        connections: default_providers(),
        api_keys: [(ProviderType::Claude, "key-FIXTURE".to_string())].into(),
    };
    let popup = Arc::new(FakePopup::new());
    let logs = Arc::new(Mutex::new(Vec::new()));
    let sink = logs.clone();
    let orch = Orchestrator::new(Deps {
        status: None,
        platform,
        runner: Arc::new(OperationRunner::new(
            ProviderRegistry::standard(),
            transport.clone(),
        )),
        registry_has: Arc::new(move |p| registry.get(p).is_ok()),
        config: Arc::new(FakeConfig::new(settings, snapshot)),
        stores: Arc::new(FakePersist::new()),
        popup: popup.clone(),
        gate: PlatformGate::new(GateTimeouts::default()),
        clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.0))),
        log: Some(Arc::new(move |line| {
            sink.lock().unwrap().push(line.to_string())
        })),
        picker: picker.clone(),
    });
    Rig {
        orch,
        popup,
        permissions,
        clip,
        keys,
        capture,
        capture_permissions,
        picker,
        transport,
        logs,
    }
}

fn last_model(r: &Rig) -> PopupViewModel {
    r.popup
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            PopupCall::ShowAt(_, m) | PopupCall::Update(m) => Some(m),
            PopupCall::Hide | PopupCall::Resize(..) => None,
        })
        .next_back()
        .expect("a popup model")
}

fn error_key(m: &PopupViewModel) -> String {
    match &m.phase {
        PopupPhase::Error(e) => e.message.key.clone(),
        other => panic!("not an error: {other:?}"),
    }
}

fn is_result(m: &PopupViewModel) -> bool {
    matches!(m.phase, PopupPhase::Result(_))
}

const ACC: PermissionKind = PermissionKind::Accessibility;
const SCR: PermissionKind = PermissionKind::ScreenRecording;

#[tokio::test]
async fn text_operation_requests_only_accessibility_when_not_determined() {
    let r = rig();
    r.permissions.set_state(ACC, PermissionState::NotDetermined);
    r.permissions.set_state(SCR, PermissionState::NotDetermined);
    // The system prompt is up: the answer is not in yet.
    r.permissions
        .set_request_outcome(ACC, PermissionState::Denied);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(r.permissions.requests(), vec![ACC]);
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_ACCESSIBILITY);
    // Nothing was copied, captured or sent.
    assert!(r.keys.events().is_empty());
    assert!(r.transport.requests().is_empty());
}

#[tokio::test]
async fn text_operation_proceeds_when_the_request_is_granted_at_once() {
    let r = rig();
    r.permissions.set_state(ACC, PermissionState::NotDetermined);
    r.permissions
        .set_request_outcome(ACC, PermissionState::Granted);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(r.permissions.requests(), vec![ACC]);
    assert!(is_result(&last_model(&r)));
}

#[tokio::test]
async fn denied_and_restricted_text_operations_never_prompt() {
    for state in [PermissionState::Denied, PermissionState::Restricted] {
        let r = rig();
        r.permissions.set_state(ACC, state);
        r.orch.trigger(AIOperation::Translate).await;
        assert!(r.permissions.requests().is_empty(), "{state:?}");
        assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_ACCESSIBILITY);
        assert!(r.keys.events().is_empty());
    }
}

#[tokio::test]
async fn text_operation_works_after_a_grant_without_a_cached_denial() {
    let r = rig();
    r.permissions.set_state(ACC, PermissionState::Denied);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_ACCESSIBILITY);
    r.permissions.set_state(ACC, PermissionState::Granted);
    r.orch.trigger(AIOperation::Translate).await;
    assert!(is_result(&last_model(&r)));
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn text_operations_never_ask_for_screen_recording() {
    let r = rig();
    r.permissions.set_state(SCR, PermissionState::NotDetermined);
    r.orch.trigger(AIOperation::Translate).await;
    assert!(is_result(&last_model(&r)));
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn permission_revoked_during_the_copy_is_reported_as_accessibility() {
    let r = rig();
    // The state read says Granted, but the key synthesizer finds Accessibility gone.
    let revoked = FakePermissions::new();
    revoked.set_state(ACC, PermissionState::Denied);
    r.keys.require_permission(revoked);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_ACCESSIBILITY);
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn windows_input_blocked_keeps_its_administrator_text() {
    let r = rig();
    r.keys.on_copy(CopyBehavior::Blocked);
    r.orch.trigger(AIOperation::Translate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_INPUT_BLOCKED);
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn screen_translate_requests_only_screen_recording_when_not_determined() {
    let r = rig();
    r.permissions.set_state(ACC, PermissionState::NotDetermined);
    r.permissions.set_state(SCR, PermissionState::NotDetermined);
    r.permissions
        .set_request_outcome(SCR, PermissionState::Denied);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.permissions.requests(), vec![SCR]);
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_SCREEN_RECORDING);
    // The picker overlay never appeared and nothing was captured.
    assert_eq!(r.picker.calls(), 0);
    assert!(r.capture.regions().is_empty());
}

#[tokio::test]
async fn denied_screen_translate_never_prompts_or_picks() {
    for state in [PermissionState::Denied, PermissionState::Restricted] {
        let r = rig();
        r.permissions.set_state(SCR, state);
        r.orch.trigger(AIOperation::ScreenTranslate).await;
        assert!(r.permissions.requests().is_empty(), "{state:?}");
        assert_eq!(r.picker.calls(), 0);
        assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_SCREEN_RECORDING);
    }
}

#[tokio::test]
async fn screen_translate_works_after_a_grant_without_a_cached_denial() {
    let r = rig();
    r.permissions.set_state(SCR, PermissionState::Denied);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_SCREEN_RECORDING);
    r.permissions.set_state(SCR, PermissionState::Granted);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.picker.calls(), 1);
    assert!(!r.capture.regions().is_empty());
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn granted_screen_translate_never_prompts() {
    let r = rig();
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.picker.calls(), 1);
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn capture_denied_after_the_check_is_reported_as_screen_recording() {
    // The orchestrator's read says Granted, but the capture service finds it revoked.
    let r = rig();
    r.capture_permissions
        .set_state(SCR, PermissionState::Denied);
    r.orch.trigger(AIOperation::ScreenTranslate).await;
    assert_eq!(r.picker.calls(), 1);
    assert_eq!(r.capture.regions().len(), 1);
    assert_eq!(error_key(&last_model(&r)), KEY_PERMISSION_SCREEN_RECORDING);
    assert!(r.permissions.requests().is_empty());
}

#[tokio::test]
async fn permission_logs_are_fixed_text_without_user_data() {
    let r = rig();
    r.permissions.set_state(ACC, PermissionState::Denied);
    let before = r.clip.sequence_number();
    r.orch.trigger(AIOperation::Translate).await;
    let logs = r.logs.lock().unwrap().clone();
    assert!(
        logs.iter()
            .any(|l| l == "permission missing: accessibility"),
        "{logs:?}"
    );
    assert!(logs.iter().all(|l| !l.contains(SECRET)), "{logs:?}");
    // The clipboard was never touched.
    assert_eq!(r.clip.sequence_number(), before);
}

#[tokio::test]
async fn replace_without_accessibility_keeps_the_result_with_a_permission_notice() {
    let r = rig();
    r.orch.trigger(AIOperation::Translate).await;
    assert!(is_result(&last_model(&r)));
    let revoked = FakePermissions::new();
    revoked.set_state(ACC, PermissionState::Denied);
    r.keys.require_permission(revoked);
    r.orch.replace().await;
    let model = last_model(&r);
    assert!(is_result(&model), "{model:?}");
    assert_eq!(
        model.notice.map(|n| n.key),
        Some(KEY_PERMISSION_ACCESSIBILITY.to_string())
    );
}
