//! The text-operation flow (port of `AppDelegate.handleHotkey` / `runOperation`): capture the
//! selection, resolve the target language, run the provider, show the result.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use lingopop_core::detector;
use lingopop_core::errors::TranslationError;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::runner::{ConfigSnapshot, OperationRunner, RunRequest, TranslationResult};
use lingopop_core::settings::Settings;
use lingopop_core::target_language::{operation_uses_auto_detect, target_language_for};
use lingopop_core::time::Clock;
use lingopop_platform::{
    capture_selection, monitor_at, physical_size, CaptureOptions, DismissEvent, LogFn, MonitorInfo,
    Platform, PlatformError, Point, Rect, WindowHandle,
};

use crate::gate::PlatformGate;
use crate::messages::{
    gate_message, max_input, missing_key, nothing_selected, platform_message,
    translation_error_view, KEY_PLATFORM_GENERIC,
};
use crate::placement::popup_origin;
use crate::popup::{PopupView, MAX_LOGICAL_SIDE};
use crate::providers::retry_providers;
use crate::screen::RegionPicker;
use crate::session::{Session, SessionState};
use crate::view_model::{PopupPhase, PopupViewModel, UserMessageView};

/// Settings and the provider snapshot, read fresh for every run.
pub trait ConfigSource: Send + Sync {
    fn settings(&self) -> Settings;
    fn snapshot(&self) -> ConfigSnapshot;
}

/// History and usage writes. The shell implements it over the stores, logs store errors and
/// never fails the flow. Implementations must not synchronously call back into Orchestrator.
pub trait Persist: Send + Sync {
    fn add_history(&self, entry: HistoryEntry, max_count: u32);
    fn append_usage(&self, record: UsageRecord);
}

pub struct Deps {
    pub platform: Platform,
    pub runner: Arc<OperationRunner>,
    pub registry_has: Arc<dyn Fn(ProviderType) -> bool + Send + Sync>,
    pub config: Arc<dyn ConfigSource>,
    pub stores: Arc<dyn Persist>,
    pub popup: Arc<dyn PopupView>,
    pub gate: PlatformGate,
    pub clock: Arc<dyn Clock>,
    pub log: Option<Arc<LogFn>>,
    /// The interactive region picker of Screen Translate (async UI, not a gate job).
    pub picker: Arc<dyn RegionPicker>,
}

pub(crate) struct Inner {
    pub deps: Deps,
    state: Mutex<SessionState>,
}

#[derive(Clone)]
pub struct Orchestrator {
    pub(crate) inner: Arc<Inner>,
}

/// Everything the one capture gate job returns. Cursor and monitor failures are not fatal.
struct Captured {
    cursor: Option<Point>,
    monitors: Option<Vec<MonitorInfo>>,
    target: Option<WindowHandle>,
    selection: Result<Option<String>, PlatformError>,
}

/// The physical rect of a popup of `size_logical` at `origin`, with the scale of the monitor
/// under the origin (the same conversion the shell applies when placing the window).
fn popup_rect(origin: Point, size_logical: (f64, f64), monitors: &[MonitorInfo]) -> Rect {
    let scale = monitor_at(origin, monitors).map_or(1.0, |m| m.scale_factor);
    let size = physical_size(size_logical.0, size_logical.1, scale);
    Rect::new(origin.x, origin.y, size.width, size.height)
}

impl Orchestrator {
    pub fn new(deps: Deps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                state: Mutex::new(SessionState::new()),
            }),
        }
    }

    /// Runs `operation` on the current selection. A newer `trigger` (or action) cancels this
    /// one: the call then returns without having touched the popup or the stores.
    pub async fn trigger(&self, operation: AIOperation) {
        self.start(move |me, generation| async move {
            if operation == AIOperation::ScreenTranslate {
                me.screen_flow(generation).await;
            } else {
                me.text_flow(operation, generation).await;
            }
        })
        .await;
    }

    /// The shell reports the measured LOGICAL popup size after layout. The orchestrator owns the
    /// geometry: the size is stored (so every later show and the dismiss rect use it), and a
    /// visible popup is re-placed with `popup_origin(size, last cursor, last monitors)`, which
    /// keeps it inside the work area, and the dismiss rect follows. All of it happens under the
    /// commit lock and only while the popup is visible, so a resize racing a Close can never
    /// resurrect the popup. Geometry only: it never bumps the generation or cancels the
    /// current translation.
    pub fn popup_resized(&self, size_logical: (f64, f64)) {
        let (width, height) = size_logical;
        if !(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0) {
            return;
        }
        let size = (width.min(MAX_LOGICAL_SIDE), height.min(MAX_LOGICAL_SIDE));
        let mut st = self.state();
        st.size = size;
        let popup = &self.deps().popup;
        if !popup.is_visible() {
            return;
        }
        let origin = popup_origin(size, st.cursor, &st.monitors);
        popup.resize(origin, size);
        self.deps()
            .platform
            .dismiss
            .update_rect(popup_rect(origin, size, &st.monitors));
    }

    /// Called by the shell's dismiss-event forwarder (the shell owns the async runtime).
    pub async fn dismissed(&self, event: DismissEvent) {
        match event {
            DismissEvent::Escape => self.escape().await,
            DismissEvent::ClickOutside => self.close().await,
        }
    }

    /// True while the session holds the captured Screen Translate image (diagnostics / tests).
    pub fn has_captured_image(&self) -> bool {
        self.state()
            .session
            .as_ref()
            .is_some_and(|s| s.image.is_some())
    }

    /// The cancellation primitive shared by `trigger` and the popup actions: bumps the
    /// generation, aborts the previous flow, spawns `make(self, generation)` and waits for it
    /// (a cancelled join is ignored; a panic of the flow task is logged with a fixed line and shown as a generic error while its generation is current). The previous flow's blocking gate job still finishes.
    pub(crate) async fn start<F, Fut>(&self, make: F)
    where
        F: FnOnce(Orchestrator, u64) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (handle, generation) = {
            let mut st = self.state();
            st.generation += 1;
            let generation = st.generation;
            if let Some(previous) = st.current.take() {
                previous.abort();
            }
            let handle = tokio::spawn(make(self.clone(), generation));
            st.current = Some(handle.abort_handle());
            (handle, generation)
        };
        match handle.await {
            Ok(()) => {}
            // Superseded by a newer generation (or the runtime is shutting down).
            Err(e) if e.is_cancelled() => {}
            Err(e) if e.is_panic() => {
                // Fixed text only: the panic payload may hold user text.
                self.log("operation flow panicked");
                self.present_error(
                    generation,
                    UserMessageView::new(KEY_PLATFORM_GENERIC, vec!["internal error".to_string()]),
                );
            }
            Err(_) => {}
        }
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, SessionState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn deps(&self) -> &Deps {
        &self.inner.deps
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.state().generation == generation
    }

    pub(crate) fn log(&self, message: &str) {
        if let Some(log) = &self.deps().log {
            log(message);
        }
    }

    /// Shows or updates the popup unless `generation` is stale. A hidden popup is placed near
    /// the last known cursor.
    pub(crate) fn present(&self, generation: u64, model: &PopupViewModel) {
        let mut st = self.state();
        if st.generation != generation {
            return;
        }
        self.present_locked(&mut st, model);
    }

    // Caller holds the generation mutex through every synchronous side effect.
    pub(crate) fn present_locked(&self, st: &mut SessionState, model: &PopupViewModel) {
        self.present_inner(st, model, false);
    }

    /// Like `present_locked`, but a visible popup is moved to the cursor anchor too (the first
    /// Loading of a Screen Translate flow: the new region may be on another monitor).
    pub(crate) fn present_at_locked(&self, st: &mut SessionState, model: &PopupViewModel) {
        self.present_inner(st, model, true);
    }

    fn present_inner(&self, st: &mut SessionState, model: &PopupViewModel, reposition: bool) {
        // After a Replace of unknown outcome no later Result offers Replace again.
        let replace_disabled = st.session.as_ref().is_some_and(|s| s.replace_disabled);
        let hidden;
        let model = match &model.phase {
            PopupPhase::Result(r) if replace_disabled && r.show_replace => {
                hidden = model.clone().without_replace();
                &hidden
            }
            _ => model,
        };
        if let Some(session) = st.session.as_mut() {
            // The OCR phase's text is what Copy offers there.
            session.last_result = match &model.phase {
                PopupPhase::Result(result) => Some(result.text.clone()),
                PopupPhase::Ocr { text, .. } => Some(text.clone()),
                _ => None,
            };
            session.last_model =
                matches!(model.phase, PopupPhase::Result(_) | PopupPhase::Ocr { .. })
                    .then(|| model.clone());
        }
        let (cursor, monitors, size) = (st.cursor, &st.monitors, st.size);
        let popup = &self.deps().popup;
        let was_visible = popup.is_visible();
        if was_visible && !reposition {
            popup.update(model);
        } else {
            let origin = popup_origin(size, cursor, monitors);
            popup.show_at(origin, size, model);
            let rect = popup_rect(origin, size, monitors);
            if was_visible {
                self.deps().platform.dismiss.update_rect(rect);
            } else if self.deps().platform.dismiss.start(rect).is_err() {
                self.log("popup dismiss watcher failed to start");
            }
        }
    }

    pub(crate) fn present_error(
        &self,
        generation: u64,
        message: crate::view_model::UserMessageView,
    ) {
        self.present(generation, &PopupViewModel::error(message, Vec::new(), &[]));
    }

    /// Shows the on-screen Result / OCR model again with `notice` (a failed Copy or Replace),
    /// so the user keeps the translation. Without a stored model it falls back to an error.
    pub(crate) fn present_notice(&self, generation: u64, notice: UserMessageView) {
        let mut st = self.state();
        if st.generation != generation {
            return;
        }
        let model = match st.session.as_ref().and_then(|s| s.last_model.clone()) {
            Some(m) => m.with_notice(notice),
            None => PopupViewModel::error(notice, Vec::new(), &[]),
        };
        self.present_locked(&mut st, &model);
    }

    /// After a successful Copy: drops a notice left by an earlier failure.
    pub(crate) fn clear_notice(&self, generation: u64) {
        let mut st = self.state();
        if st.generation != generation {
            return;
        }
        let model = match st.session.as_ref().and_then(|s| s.last_model.clone()) {
            Some(m) if m.notice.is_some() => m.without_notice(),
            _ => return,
        };
        self.present_locked(&mut st, &model);
    }

    async fn text_flow(&self, operation: AIOperation, generation: u64) {
        {
            // A text operation ends any Screen Translate session: drop the captured image.
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            if let Some(session) = st.session.as_mut() {
                session.image = None;
            }
        }
        let d = self.deps();
        let (clipboard, keys, foreground, sleeper, monitors, log) = (
            d.platform.clipboard.clone(),
            d.platform.keys.clone(),
            d.platform.foreground.clone(),
            d.platform.sleeper.clone(),
            d.platform.monitors.clone(),
            d.log.clone(),
        );
        let job_log = log.clone();
        let captured = d
            .gate
            .run(move || {
                let note = |m: &str| {
                    if let Some(l) = &job_log {
                        l(m);
                    }
                };
                let cursor = monitors
                    .cursor_position()
                    .map_err(|_| note("cursor position unavailable"))
                    .ok();
                let monitor_list = monitors
                    .monitors()
                    .map_err(|_| note("monitor list unavailable"))
                    .ok();
                let target = foreground.current();
                let options = CaptureOptions {
                    log,
                    ..Default::default()
                };
                let selection = capture_selection(
                    clipboard.as_ref(),
                    keys.as_ref(),
                    foreground.as_ref(),
                    sleeper.as_ref(),
                    &options,
                );
                Ok(Captured {
                    cursor,
                    monitors: monitor_list,
                    target,
                    selection,
                })
            })
            .await;
        if !self.is_current(generation) {
            return;
        }
        let captured = match captured {
            Ok(c) => c,
            Err(e) => {
                self.log("capture did not run");
                self.present_error(generation, gate_message(&e));
                return;
            }
        };
        {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            if let Some(c) = captured.cursor {
                st.cursor = c;
            }
            if let Some(m) = captured.monitors {
                st.monitors = m;
            }
        }
        let text = match captured.selection {
            Ok(Some(text)) => text,
            Ok(None) => {
                self.log("capture: nothing selected");
                self.present_error(generation, nothing_selected());
                return;
            }
            Err(e) => {
                self.log("capture failed");
                self.present_error(generation, platform_message(&e));
                return;
            }
        };

        let settings = d.config.settings();
        let chars = text.chars().count();
        self.log(&format!("capture: {chars} characters"));
        if settings.max_input_chars > 0 && chars > settings.max_input_chars as usize {
            self.present_error(generation, max_input(chars, settings.max_input_chars));
            return;
        }
        let snapshot = d.config.snapshot();
        let detected_source = if operation_uses_auto_detect(operation) {
            detector::detect(&text).map(str::to_string)
        } else {
            None
        };
        let target_language = target_language_for(&settings, operation, &text);
        let primary = settings.active_provider;
        {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            st.session = Some(Session {
                operation,
                text: text.clone(),
                last_result: None,
                detected_source: detected_source.clone(),
                target_language: target_language.clone(),
                target_window: captured.target,
                primary,
                image: None,
                last_model: None,
                replace_disabled: false,
            });
        }
        self.present(generation, &PopupViewModel::loading(operation, &text));
        self.run_and_present(
            generation,
            &settings,
            &snapshot,
            RunRequest {
                operation,
                text,
                detected_source,
                target_language,
                primary,
                allow_fallback: true,
            },
        )
        .await;
    }

    /// Runs the request, persists a success and shows the result or the error. Reused by the
    /// popup actions (Retry / Rewrite / Explain). A stale generation persists and shows nothing.
    pub(crate) async fn run_and_present(
        &self,
        generation: u64,
        settings: &Settings,
        snapshot: &ConfigSnapshot,
        request: RunRequest,
    ) {
        let operation = request.operation;
        let primary = request.primary;
        let outcome = self.deps().runner.run(request, snapshot).await;
        self.finish(generation, settings, snapshot, operation, primary, outcome)
            .await;
    }

    /// Commits a run outcome under the generation lock: persists a success and shows it or the
    /// error. A stale generation persists, shows and copies nothing. A Screen Translate success
    /// is then copied when `auto_copy_ocr` is on (mac `OperationPresenter.present`): the copy
    /// runs after the commit, so a result that was displayed is also copied.
    pub(crate) async fn finish(
        &self,
        generation: u64,
        settings: &Settings,
        snapshot: &ConfigSnapshot,
        operation: AIOperation,
        primary: ProviderType,
        outcome: Result<TranslationResult, TranslationError>,
    ) {
        let d = self.deps();
        let registry_has = |p: ProviderType| (d.registry_has)(p);
        let copy = {
            let mut st = self.state();
            if st.generation != generation {
                return;
            }
            match outcome {
                Ok(result) => {
                    let now = d.clock.now();
                    d.stores
                        .add_history(result.history_entry(now), settings.max_history_count);
                    if let Some(usage) = result.usage_record(now) {
                        d.stores.append_usage(usage);
                    }
                    let retry = retry_providers(result.provider, operation, snapshot, registry_has);
                    self.present_locked(&mut st, &PopupViewModel::result(&result, &retry));
                    (operation == AIOperation::ScreenTranslate && settings.auto_copy_ocr)
                        .then_some(result.translated)
                }
                Err(error) => {
                    let (message, details) = match &error {
                        TranslationError::MissingApiKey => (missing_key(primary), Vec::new()),
                        other => translation_error_view(other),
                    };
                    let retry = retry_providers(primary, operation, snapshot, registry_has);
                    self.present_locked(&mut st, &PopupViewModel::error(message, details, &retry));
                    None
                }
            }
        };
        if let Some(text) = copy {
            self.auto_copy(generation, text).await;
        }
    }

    /// Writes `text` to the clipboard through the gate (autoCopyOCR). A failure is logged
    /// without any text and not shown. `spawn_blocking` survives cancellation, so the
    /// generation is re-checked INSIDE the blocking closure right before `write_text`; the
    /// session mutex is taken only for that check, never during the write (a hung clipboard
    /// owner must not freeze the orchestrator). A sub-millisecond window remains between the
    /// check and the write.
    pub(crate) async fn auto_copy(&self, generation: u64, text: String) {
        if !self.is_current(generation) {
            return;
        }
        let clipboard = self.deps().platform.clipboard.clone();
        let me = self.clone();
        if self
            .deps()
            .gate
            .run(move || {
                if !me.is_current(generation) {
                    return Ok(());
                }
                clipboard.write_text(&text)
            })
            .await
            .is_err()
        {
            self.log("auto-copy failed");
        }
    }
}

#[cfg(test)]
mod commit_tests {
    use super::*;
    use crate::fake::{FakeConfig, FakePersist, FakePopup};
    use lingopop_core::{http::FakeTransport, providers::ProviderRegistry, time::SystemClock};
    use std::sync::OnceLock;

    fn test_platform() -> Platform {
        use lingopop_platform::fake::*;
        struct Protector;
        impl lingopop_core::secrets::SecretProtector for Protector {
            fn protect(&self, _: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
                Ok(vec![])
            }
            fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, lingopop_core::secrets::ProtectError> {
                Ok(vec![])
            }
        }
        let sleeper = Arc::new(FakeSleeper::new());
        let clipboard = FakeClipboard::new(sleeper.clone());
        Platform {
            popup_window: Arc::new(FakePopupWindow::default()),
            dismiss: Arc::new(FakeDismissWatcher::default()),
            keys: FakeKeys::new(clipboard.clone(), sleeper.clone()),
            clipboard,
            foreground: FakeForeground::new(None),
            monitors: Arc::new(FakeMonitors::new(Point { x: 0, y: 0 }, vec![])),
            capture: Arc::new(FakeCapture::new([0; 4])),
            ocr: Arc::new(FakeOcr::new(Err(PlatformError::OcrUnavailable))),
            protector: Arc::new(Protector),
            sleeper,
        }
    }

    struct CheckedPopup {
        inner: OnceLock<std::sync::Weak<Inner>>,
        view: FakePopup,
    }
    impl PopupView for CheckedPopup {
        fn show_at(&self, origin: Point, size: (f64, f64), model: &PopupViewModel) {
            assert!(self
                .inner
                .get()
                .unwrap()
                .upgrade()
                .unwrap()
                .state
                .try_lock()
                .is_err());
            self.view.show_at(origin, size, model);
        }
        fn update(&self, model: &PopupViewModel) {
            assert!(self
                .inner
                .get()
                .unwrap()
                .upgrade()
                .unwrap()
                .state
                .try_lock()
                .is_err());
            self.view.update(model);
        }
        fn resize(&self, origin: Point, size: (f64, f64)) {
            self.view.resize(origin, size);
        }
        fn hide(&self) {
            self.view.hide();
        }
        fn is_visible(&self) -> bool {
            self.view.is_visible()
        }
    }

    struct CheckedPersist {
        inner: OnceLock<std::sync::Weak<Inner>>,
        records: FakePersist,
    }
    impl Persist for CheckedPersist {
        fn add_history(&self, entry: HistoryEntry, max: u32) {
            // Deterministically pin the check/commit boundary: a newer start cannot bump
            // generation while persistence is in progress. No scheduling or timeout needed.
            let inner = self.inner.get().unwrap().upgrade().unwrap();
            assert!(
                inner.state.try_lock().is_err(),
                "generation bump must be excluded during persistence"
            );
            self.records.add_history(entry, max);
        }
        fn append_usage(&self, record: UsageRecord) {
            let inner = self.inner.get().unwrap().upgrade().unwrap();
            assert!(inner.state.try_lock().is_err());
            self.records.append_usage(record);
        }
    }

    #[tokio::test]
    async fn superseded_run_after_check_never_presents_or_persists() {
        let stores = Arc::new(CheckedPersist {
            inner: OnceLock::new(),
            records: FakePersist::new(),
        });
        let popup = Arc::new(CheckedPopup {
            inner: OnceLock::new(),
            view: FakePopup::new(),
        });
        let settings = Settings::default();
        let snapshot = ConfigSnapshot {
            magic_prompt: String::new(),
            fallbacks: Default::default(),
            connections: Default::default(),
            api_keys: Default::default(),
        };
        let me = Orchestrator::new(Deps {
            platform: test_platform(),
            runner: Arc::new(OperationRunner::new(
                ProviderRegistry::standard(),
                Arc::new(FakeTransport::json(200, r#"[[["B"]]]"#)),
            )),
            registry_has: Arc::new(|_| true),
            config: Arc::new(FakeConfig::new(settings.clone(), snapshot.clone())),
            stores: stores.clone(),
            popup: popup.clone(),
            gate: PlatformGate::new(Default::default()),
            clock: Arc::new(SystemClock),
            log: None,
            picker: Arc::new(crate::fake::FakePicker::new(Ok(None))),
        });
        stores.inner.set(Arc::downgrade(&me.inner)).unwrap();
        popup.inner.set(Arc::downgrade(&me.inner)).unwrap();
        me.state().generation = 2;
        me.popup_resized((300.0, 200.0));
        assert_eq!(me.state().generation, 2, "resize must not cancel a run");
        let request = || RunRequest {
            operation: AIOperation::Translate,
            text: "original".into(),
            detected_source: None,
            target_language: "German".into(),
            primary: ProviderType::GoogleFree,
            allow_fallback: false,
        };
        me.run_and_present(1, &settings, &snapshot, request()).await;
        assert!(stores.records.history().is_empty());
        assert!(popup.view.calls().is_empty());
        me.run_and_present(2, &settings, &snapshot, request()).await;
        assert_eq!(stores.records.history().len(), 1);
        assert!(
            matches!(&popup.view.calls()[0], crate::fake::PopupCall::ShowAt(_, m) if matches!(&m.phase, crate::view_model::PopupPhase::Result(r) if r.text == "B"))
        );
    }
}
