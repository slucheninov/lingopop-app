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
    view: ManualPopup,
    log: Arc<EventLog>,
    hidden_at: Mutex<Vec<usize>>,
}
impl PopupView for Popup {
    fn submit(&self, cmd: PresentationCommand) -> Result<AckReceiver, PresentationQueueError> {
        if matches!(cmd.kind, PresentationKind::Hide) {
            self.hidden_at.lock().unwrap().push(self.log.events().len());
        }
        self.view.submit(cmd)
    }
    fn bind(&self, fence: Arc<PresentationFence>, sink: Arc<AckSink>) {
        self.view.bind(fence, sink);
    }
}
impl Popup {
    fn is_visible(&self) -> bool {
        self.view.visible.load(std::sync::atomic::Ordering::SeqCst)
    }
}

struct Rig {
    logs: Arc<Mutex<Vec<String>>>,
    orch: Orchestrator,
    monitors: Arc<BarrierMonitors>,
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
        view: ManualPopup::default(),
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
    let monitors = Arc::new(BarrierMonitors {
        inner: FakeMonitors::new(
            cursor,
            vec![MonitorInfo {
                bounds: Rect::new(-1000, -1000, 4000, 4000),
                work_area: Rect::new(-1000, -1000, 4000, 4000),
                scale_factor: 1.5,
                primary: true,
            }],
        ),
        barrier: Mutex::new(None),
    });
    let orch = Orchestrator::new(Deps {
        status: None,
        platform: Platform {
            overlay: Arc::new(lingopop_platform::fake::FakeOverlay::new(Ok(None))),
            system: Arc::new(lingopop_platform::fake::FakeSystem::new(vec![])),
            login_item: Arc::new(FakeLoginItem::new()),
            popup_window: Arc::new(FakePopupWindow::default()),
            dismiss: dismiss.clone(),
            clipboard: clip.clone(),
            keys: keys.clone(),
            foreground: fg.clone(),
            last_foreground: lingopop_platform::fake::FakeLastForeground::new(None),
            monitors: monitors.clone(),
            capture: Arc::new(FakeCapture::new([0; 4])),
            ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
            permissions: Arc::new(lingopop_platform::AlwaysGranted),
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
        monitors,
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

#[derive(Default)]
struct ManualPopup {
    queue: PresentationQueue,
    binding: std::sync::OnceLock<(Arc<PresentationFence>, Arc<AckSink>)>,
    submitted: tokio::sync::Notify,
    visible: std::sync::atomic::AtomicBool,
    effects: std::sync::atomic::AtomicUsize,
}
impl PopupView for ManualPopup {
    fn bind(&self, fence: Arc<PresentationFence>, sink: Arc<AckSink>) {
        assert!(self.binding.set((fence, sink)).is_ok());
    }
    fn submit(&self, cmd: PresentationCommand) -> Result<AckReceiver, PresentationQueueError> {
        let rx = self.queue.submit(cmd)?;
        self.submitted.notify_one();
        Ok(rx)
    }
}
impl ManualPopup {
    async fn take(&self) -> PresentationJob {
        loop {
            let notified = self.submitted.notified();
            if let Some(job) = self.queue.try_next() {
                return job;
            }
            notified.await;
        }
    }
    async fn take_hide(&self) -> PresentationJob {
        loop {
            let job = self.take().await;
            if matches!(job.command.kind, PresentationKind::Hide) {
                return job;
            }
            self.complete(job);
        }
    }
    fn complete(&self, job: PresentationJob) -> PresentationAck {
        let (fence, sink) = self.binding.get().unwrap();
        self.queue.complete_superseded(sink.as_ref());
        let ack = fence
            .apply_if_current(job.command.id, || {
                self.effects
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let visible = !matches!(job.command.kind, PresentationKind::Hide);
                self.visible
                    .store(visible, std::sync::atomic::Ordering::SeqCst);
                Ok(if visible {
                    PresentationVisibility::Visible
                } else {
                    PresentationVisibility::Hidden
                })
            })
            .unwrap();
        job.complete(ack.clone(), sink.as_ref());
        ack
    }
}
#[tokio::test]
async fn old_show_queued_during_new_capture_has_zero_effects() {
    let (r, _) = ready_rig();
    r.orch.trigger(AIOperation::Translate).await;
    let old = r.popup.view.take().await;
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    *r.monitors.barrier.lock().unwrap() = Some((entered, blocked));
    let me = r.orch.clone();
    let second = tokio::spawn(async move {
        me.trigger(AIOperation::Translate).await;
    });
    started.await.unwrap(); // the new generation is inside capture, before presentation
    let old_show_ack = r.popup.view.complete(old);
    let stale_native_effect_count = r
        .popup
        .view
        .effects
        .load(std::sync::atomic::Ordering::SeqCst);
    let visibility = r.orch.visibility();
    assert_eq!(stale_native_effect_count, 0);
    assert_eq!(old_show_ack.result, Ok(PresentationVisibility::Superseded));
    assert_eq!(visibility, Visibility::Hidden);
    assert!(r.dismiss.calls().is_empty());
    release.send(()).unwrap();
    second.await.unwrap();
}
#[tokio::test]
async fn superseded_show_and_hide_ack_do_not_paste() {
    let (r, _) = ready_rig();
    r.orch.trigger(AIOperation::Translate).await;
    let show = r.popup.view.take().await;
    let stale_id = show.command.id;
    let me = r.orch.clone();
    let replace = tokio::spawn(async move {
        me.replace_with_deadline(std::future::pending()).await;
    });
    let hide = r.popup.view.take_hide().await;
    let paste_count_before_current_hide_ack = r.keys.pasted().len();
    r.orch.presentation_acked(PresentationAck {
        id: stale_id,
        result: Ok(PresentationVisibility::Hidden),
    });
    let paste_count_after_stale_hide_ack = r.keys.pasted().len();
    r.popup.view.complete(show);
    r.popup.view.complete(hide);
    replace.await.unwrap();
    let paste_count_after_current_hide_ack = r.keys.pasted().len();
    assert_eq!(paste_count_before_current_hide_ack, 0);
    assert_eq!(paste_count_after_stale_hide_ack, 0);
    assert_eq!(paste_count_after_current_hide_ack, 1);
}
#[tokio::test]
async fn replace_deadline_and_cancellation_never_paste() {
    for cancel in [false, true] {
        let (r, _) = ready_rig();
        r.orch.trigger(AIOperation::Translate).await;
        let show = r.popup.view.take().await;
        r.popup.view.complete(show);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let me = r.orch.clone();
        let replace = tokio::spawn(async move {
            me.replace_with_deadline(async {
                let _ = rx.await;
            })
            .await;
        });
        let hide = r.popup.view.take_hide().await;
        if cancel {
            r.orch.copy().await;
        } else {
            tx.send(()).unwrap();
        }
        replace.await.unwrap();
        r.popup.view.complete(hide);
        assert_eq!(r.keys.pasted().len(), 0);
    }
}

type CaptureBarrier = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);
struct BarrierMonitors {
    inner: FakeMonitors,
    barrier: Mutex<Option<CaptureBarrier>>,
}
impl Monitors for BarrierMonitors {
    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        self.inner.monitors()
    }
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        let barrier = self.barrier.lock().unwrap().take();
        if let Some((started, release)) = barrier {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        self.inner.cursor_position()
    }
}
#[tokio::test]
async fn duplicate_visible_ack_is_inert_and_failed_hide_never_pastes() {
    let (r, _) = ready_rig();
    r.orch.trigger(AIOperation::Translate).await;
    while let Some(job) = r.popup.view.queue.try_next() {
        let ack = r.popup.view.complete(job);
        let calls = r.dismiss.calls();
        r.orch.presentation_acked(ack);
        assert_eq!(r.dismiss.calls(), calls);
    }
    let me = r.orch.clone();
    let replace = tokio::spawn(async move {
        me.replace_with_deadline(std::future::pending()).await;
    });
    let hide = r.popup.view.take_hide().await;
    let id = hide.command.id;
    hide.complete(
        PresentationAck {
            id,
            result: Err(PlatformError::Unsupported("fixture hide")),
        },
        r.popup.view.binding.get().unwrap().1.as_ref(),
    );
    replace.await.unwrap();
    assert!(r.keys.pasted().is_empty());
}

#[tokio::test]
async fn effect_can_reenter_orchestrator_without_session_lock() {
    let (r, _) = ready_rig();
    r.orch.trigger(AIOperation::Translate).await;
    let mut current = r.popup.view.take().await;
    while let Some(next) = r.popup.view.queue.try_next() {
        r.popup.view.complete(current);
        current = next;
    }
    let (fence, sink) = r.popup.view.binding.get().unwrap();
    let ack = fence
        .apply_if_current(current.command.id, || {
            r.orch.popup_resized((500., 300.));
            Ok(PresentationVisibility::Visible)
        })
        .unwrap();
    assert_eq!(ack.result, Ok(PresentationVisibility::Superseded));
    current.complete(ack, sink.as_ref());
    assert!(matches!(r.orch.visibility(), Visibility::Pending(_)));
    let resize = r.popup.view.take().await;
    r.popup.view.complete(resize);
    assert!(matches!(r.orch.visibility(), Visibility::Visible(_)));
}
