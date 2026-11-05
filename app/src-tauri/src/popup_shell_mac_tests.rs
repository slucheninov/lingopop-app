//! The macOS presentation consumer on fakes: the real shell worker, the platform
//! `MacPresenter` over a `FakeMainThread` (jobs run only when the test says so) and the
//! portable `MacDismissWatcher` over a `FakeMonitorBackend`. Compiled on every OS; no
//! AppKit, no Tauri runtime, no clock.
use super::*;
use lingopop_platform::fake::{
    FakeMainThread, FakeMonitorBackend, FakeMonitors, FakePopupWindow, PopupWindowCall,
};
use lingopop_platform::macos_dismiss::{MacDismissWatcher, ObservedEvent};
use lingopop_platform::{DismissEvent, DismissWatcher, MonitorInfo, NoActivateWindow};

const WINDOW: WindowHandle = WindowHandle(0x77);

fn id(generation: u64, sequence: u64) -> PresentationId {
    PresentationId {
        generation,
        sequence,
    }
}
fn model(text: &str) -> PopupViewModel {
    PopupViewModel::loading(lingopop_core::operation::AIOperation::Translate, text)
}
fn show(id: PresentationId, text: &str) -> PresentationCommand {
    PresentationCommand {
        id,
        kind: PresentationKind::Show {
            origin: Point { x: 40, y: 50 },
            size_logical: (360.0, 100.0),
            model: model(text),
        },
    }
}
fn monitors() -> Arc<dyn lingopop_platform::Monitors> {
    Arc::new(FakeMonitors::new(
        Point { x: 0, y: 0 },
        vec![MonitorInfo {
            bounds: Rect::new(0, 0, 1440, 900),
            work_area: Rect::new(0, 25, 1440, 875),
            scale_factor: 1.0,
            primary: true,
        }],
    ))
}
type Emitted = Arc<Mutex<Vec<serde_json::Value>>>;
fn mac_popup(
    main: Arc<FakeMainThread>,
    window: Arc<FakePopupWindow>,
    dismiss: Arc<dyn DismissWatcher>,
) -> (ShellPopup, Emitted) {
    let emitted: Emitted = Arc::default();
    let sink = emitted.clone();
    let window: Arc<dyn NoActivateWindow> = window;
    let consumer = PresentationConsumer::new(
        Arc::new(move |value: &serde_json::Value| sink.lock().unwrap().push(value.clone())),
        monitors(),
        Box::new(MacPanelSurface::new(main, window, Some(WINDOW))),
    )
    .with_dismiss(dismiss);
    (ShellPopup::from_consumer(consumer), emitted)
}
/// Worker shutdown waits for the posted native monitor removal: play main once, then wait.
fn finish(popup: &ShellPopup, main: &FakeMainThread) {
    let done = popup.shutdown();
    main.wait_for_pending(1);
    main.run_pending();
    done.blocking_recv().unwrap();
}
fn native_shows(window: &FakePopupWindow) -> usize {
    window
        .calls()
        .iter()
        .filter(|c| matches!(c, PopupWindowCall::ShowAt(..)))
        .count()
}
fn watcher(
    trusted: bool,
    main: &Arc<FakeMainThread>,
) -> (
    Arc<MacDismissWatcher<FakeMonitorBackend>>,
    Arc<FakeMonitorBackend>,
) {
    let backend = Arc::new(FakeMonitorBackend::new(trusted));
    let watcher = Arc::new(MacDismissWatcher::new(backend.clone()));
    watcher.bind_main_thread(main.clone());
    (watcher, backend)
}

#[test]
fn stale_show_after_dismiss_has_no_effect() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, backend) = watcher(true, &main);
    let (popup, _) = mac_popup(main.clone(), window.clone(), watcher.clone());
    let fence = Arc::new(PresentationFence::default());
    popup.bind(fence.clone(), Arc::new(|_| {}));

    // Generation 2 is on screen and watched.
    fence.advance(id(2, 1));
    let shown = popup.submit(show(id(2, 1), "two")).unwrap();
    main.wait_for_pending(1);
    main.run_pending();
    assert_eq!(
        shown.blocking_recv().unwrap().result,
        Ok(PresentationVisibility::Visible)
    );
    watcher.start(Rect::new(40, 50, 360, 100)).unwrap();
    main.run_pending();

    // Generation 3's Show is queued on the main thread but has not run.
    fence.advance(id(3, 2));
    let queued = popup.submit(show(id(3, 2), "three")).unwrap();
    main.wait_for_pending(1);
    let shows_before = native_shows(&window);

    // A click outside arrives before the dispatcher runs; the orchestrator's answer is a
    // newer generation, a stopped watcher and a Hide.
    backend.fire(ObservedEvent::MouseDown {
        location: Point { x: 1000, y: 800 },
        marker: 0,
    });
    assert_eq!(watcher.events().try_recv(), Ok(DismissEvent::ClickOutside));
    fence.advance(id(4, 3));
    watcher.stop();
    let hidden = popup
        .submit(PresentationCommand {
            id: id(4, 3),
            kind: PresentationKind::Hide,
        })
        .unwrap();

    main.run_pending();
    let ack = queued.blocking_recv().unwrap();
    main.wait_for_pending(1);
    main.run_pending();
    assert_eq!(
        hidden.blocking_recv().unwrap().result,
        Ok(PresentationVisibility::Hidden)
    );
    let stale_native_show_count = native_shows(&window) - shows_before;

    finish(&popup, &main);
    let native_monitors_after_shutdown = backend.native_monitors();

    assert_eq!(stale_native_show_count, 0);
    assert_eq!(ack.result, Ok(PresentationVisibility::Superseded));
    assert_eq!(native_monitors_after_shutdown, 0);
}

#[test]
fn show_acks_only_after_native_completion_and_no_lock_spans_dispatch() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, _) = watcher(true, &main);
    let (popup, emitted) = mac_popup(main.clone(), window.clone(), watcher);
    let fence = Arc::new(PresentationFence::default());
    popup.bind(fence.clone(), Arc::new(|_| {}));
    fence.advance(id(1, 1));
    let mut shown = popup.submit(show(id(1, 1), "one")).unwrap();
    main.wait_for_pending(1);
    assert!(window.calls().is_empty());
    assert!(shown.try_recv().is_err(), "no ack before main ran");
    // While the worker waits on main, the queue and the model cache stay free: a newer
    // submission and a ready() replay request return at once. (The fence is not advanced,
    // so the Update itself resolves Superseded and cannot disturb the Show.)
    let update = popup
        .submit(PresentationCommand {
            id: id(1, 2),
            kind: PresentationKind::Update {
                model: model("newer"),
            },
        })
        .unwrap();
    popup.ready();
    assert_eq!(
        emitted.lock().unwrap().len(),
        1,
        "only the Show emitted so far"
    );
    main.run_pending();
    let ack = shown.blocking_recv().unwrap();
    assert_eq!(ack.result, Ok(PresentationVisibility::Visible));
    assert_eq!(
        window.calls(),
        vec![PopupWindowCall::ShowAt(WINDOW, Rect::new(40, 50, 360, 100))]
    );
    assert_eq!(window.main_thread_calls(), 1);
    assert_eq!(
        update.blocking_recv().unwrap().result,
        Ok(PresentationVisibility::Superseded)
    );
    finish(&popup, &main);
}

#[test]
fn ready_replays_the_newest_model() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, _) = watcher(true, &main);
    let (popup, emitted) = mac_popup(main.clone(), window, watcher);
    let fence = Arc::new(PresentationFence::default());
    popup.bind(fence.clone(), Arc::new(|_| {}));
    fence.advance(id(1, 1));
    let shown = popup.submit(show(id(1, 1), "old")).unwrap();
    main.wait_for_pending(1);
    main.run_pending();
    shown.blocking_recv().unwrap();
    fence.advance(id(1, 2));
    popup
        .submit(PresentationCommand {
            id: id(1, 2),
            kind: PresentationKind::Update {
                model: model("new"),
            },
        })
        .unwrap()
        .blocking_recv()
        .unwrap();
    popup.ready();
    finish(&popup, &main);
    let emitted = emitted.lock().unwrap();
    let last = emitted.last().unwrap();
    assert_eq!(emitted.len(), 3, "show, update, replay");
    assert_eq!(last, &serde_json::to_value(model("new")).unwrap());
}

#[test]
fn worker_shutdown_removes_monitors() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, backend) = watcher(true, &main);
    let (popup, _) = mac_popup(main.clone(), window, watcher.clone());
    popup.bind(Arc::default(), Arc::new(|_| {}));
    watcher.start(Rect::new(0, 0, 10, 10)).unwrap();
    main.run_pending();
    assert!(backend.native_monitors() > 0);
    let mut done = popup.shutdown();
    // The worker posted the native removal and waits for it before resolving completion.
    main.wait_for_pending(1);
    assert!(done.try_recv().is_err(), "completion before native removal");
    assert!(backend.native_monitors() > 0);
    main.run_pending();
    done.blocking_recv().unwrap();
    assert_eq!(backend.native_monitors(), 0, "removed before completion");
}

#[test]
fn missing_accessibility_adds_the_escape_notice() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, _) = watcher(false, &main);
    let (popup, emitted) = mac_popup(main.clone(), window, watcher);
    let fence = Arc::new(PresentationFence::default());
    popup.bind(fence.clone(), Arc::new(|_| {}));
    fence.advance(id(1, 1));
    let shown = popup.submit(show(id(1, 1), "x")).unwrap();
    main.wait_for_pending(1);
    main.run_pending();
    shown.blocking_recv().unwrap();
    finish(&popup, &main);
    let emitted = emitted.lock().unwrap();
    assert_eq!(
        emitted[0]["notice"]["key"],
        serde_json::Value::from(lingopop_orchestrator::messages::KEY_ESCAPE_UNAVAILABLE)
    );
}

#[test]
fn trusted_popup_has_no_escape_notice() {
    let main = FakeMainThread::new();
    let window = Arc::new(FakePopupWindow::default());
    let (watcher, _) = watcher(true, &main);
    let (popup, emitted) = mac_popup(main.clone(), window, watcher);
    let fence = Arc::new(PresentationFence::default());
    popup.bind(fence.clone(), Arc::new(|_| {}));
    fence.advance(id(1, 1));
    let shown = popup.submit(show(id(1, 1), "x")).unwrap();
    main.wait_for_pending(1);
    main.run_pending();
    shown.blocking_recv().unwrap();
    finish(&popup, &main);
    assert!(emitted.lock().unwrap()[0].get("notice").is_none());
}

mod replace {
    use super::*;
    use lingopop_core::{
        http::FakeTransport,
        operation::AIOperation,
        provider::{default_providers, ProviderType},
        providers::ProviderRegistry,
        runner::{ConfigSnapshot, OperationRunner},
        secrets::{ProtectError, SecretProtector},
        settings::Settings,
        time::{ManualClock, Timestamp},
    };
    use lingopop_orchestrator::fake::{FakeConfig, FakePersist, FakePicker};
    use lingopop_orchestrator::{Deps, GateTimeouts, Orchestrator, PlatformGate};
    use lingopop_platform::fake::*;

    struct Plain;
    impl SecretProtector for Plain {
        fn protect(&self, p: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Ok(p.to_vec())
        }
        fn unprotect(&self, p: &[u8]) -> Result<Vec<u8>, ProtectError> {
            Ok(p.to_vec())
        }
    }
    const OK: &str = r#"{"content":[{"type":"text","text":"DISPLAYED"}],"usage":{"input_tokens":3,"output_tokens":2}}"#;

    /// Replace's Hide ack resolves before any paste on the Mac consumer: at the moment of the
    /// paste the native Hide has run on main (a pump thread plays the main loop).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hide_ack_precedes_paste() {
        let main = FakeMainThread::new();
        let pump = {
            let main = main.clone();
            std::thread::spawn(move || main.pump())
        };
        let window = Arc::new(FakePopupWindow::default());
        let sleeper = Arc::new(FakeSleeper::new());
        let clip = FakeClipboard::new(sleeper.clone());
        let keys = FakeKeys::new(clip.clone(), sleeper.clone());
        keys.on_copy(CopyBehavior::Select("ORIGINAL".into()));
        let seen_at_paste: Arc<Mutex<Option<Vec<PopupWindowCall>>>> = Arc::default();
        {
            let window = window.clone();
            let seen = seen_at_paste.clone();
            keys.on_paste(Box::new(move |_| {
                *seen.lock().unwrap() = Some(window.calls());
            }));
        }
        let fg = FakeForeground::new(Some(WindowHandle(7)));
        let dismiss = Arc::new(FakeDismissWatcher::default());
        let (popup, _) = mac_popup(main.clone(), window.clone(), dismiss.clone());
        let popup = Arc::new(popup);
        let snapshot = ConfigSnapshot {
            magic_prompt: String::new(),
            fallbacks: Default::default(),
            connections: default_providers(),
            api_keys: ProviderType::ALL
                .into_iter()
                .map(|p| (p, "key".into()))
                .collect(),
        };
        let settings = Settings {
            active_provider: ProviderType::Claude,
            ..Settings::default()
        };
        let transport = Arc::new(FakeTransport::json(200, OK));
        let orch = Orchestrator::new(Deps {
            status: None,
            platform: lingopop_platform::Platform {
                overlay: Arc::new(FakeOverlay::new(Ok(None))),
                system: Arc::new(FakeSystem::new(vec![])),
                login_item: Arc::new(FakeLoginItem::new()),
                popup_window: window.clone(),
                dismiss,
                clipboard: clip.clone(),
                keys: keys.clone(),
                foreground: fg,
                last_foreground: FakeLastForeground::new(None),
                monitors: monitors(),
                capture: Arc::new(FakeCapture::new([0; 4])),
                ocr: Arc::new(FakeOcr::new(Err(
                    lingopop_platform::PlatformError::OcrUnavailable,
                ))),
                permissions: Arc::new(lingopop_platform::AlwaysGranted),
                protector: Arc::new(Plain),
                sleeper,
            },
            runner: Arc::new(OperationRunner::new(
                ProviderRegistry::standard(),
                transport,
            )),
            registry_has: Arc::new(|_| true),
            config: Arc::new(FakeConfig::new(settings, snapshot)),
            stores: Arc::new(FakePersist::new()),
            popup: popup.clone(),
            gate: PlatformGate::new(GateTimeouts::default()),
            clock: Arc::new(ManualClock::new(Timestamp::from_unix_seconds(1000.))),
            log: None,
            picker: Arc::new(FakePicker::new(Ok(None))),
        });
        orch.trigger(AIOperation::Translate).await;
        orch.replace_with_deadline(std::future::pending()).await;
        assert_eq!(keys.pasted().len(), 1, "Replace pasted once");
        let at_paste = seen_at_paste.lock().unwrap().clone().unwrap();
        assert_eq!(
            at_paste.last(),
            Some(&PopupWindowCall::Hide(WINDOW)),
            "the native Hide ran before the paste: {at_paste:?}"
        );
        drop(popup.shutdown().await);
        main.close();
        pump.join().unwrap();
    }
}
