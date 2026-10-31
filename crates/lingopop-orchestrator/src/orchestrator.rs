//! The text-operation flow (port of `AppDelegate.handleHotkey` / `runOperation`): capture the
//! selection, resolve the target language, run the provider, show the result.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use lingopop_core::detector;
use lingopop_core::errors::TranslationError;
use lingopop_core::operation::AIOperation;
use lingopop_core::provider::ProviderType;
use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::runner::{ConfigSnapshot, OperationRunner, RunRequest};
use lingopop_core::settings::Settings;
use lingopop_core::target_language::{operation_uses_auto_detect, target_language_for};
use lingopop_core::time::Clock;
use lingopop_platform::{
    capture_selection, CaptureOptions, LogFn, MonitorInfo, Platform, PlatformError, Point,
    WindowHandle,
};

use crate::gate::PlatformGate;
use crate::messages::{
    gate_message, max_input, missing_key, nothing_selected, platform_message,
    translation_error_view,
};
use crate::placement::popup_origin;
use crate::popup::{PopupView, INITIAL_POPUP_SIZE};
use crate::providers::retry_providers;
use crate::session::{Session, SessionState};
use crate::view_model::PopupViewModel;

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
            me.text_flow(operation, generation).await;
        })
        .await;
    }

    /// The cancellation primitive shared by `trigger` and the popup actions: bumps the
    /// generation, aborts the previous flow, spawns `make(self, generation)` and waits for it
    /// (a cancelled join is ignored). The previous flow's blocking gate job still finishes.
    pub(crate) async fn start<F, Fut>(&self, make: F)
    where
        F: FnOnce(Orchestrator, u64) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handle = {
            let mut st = self.state();
            st.generation += 1;
            let generation = st.generation;
            if let Some(previous) = st.current.take() {
                previous.abort();
            }
            let handle = tokio::spawn(make(self.clone(), generation));
            st.current = Some(handle.abort_handle());
            handle
        };
        let _ = handle.await;
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
        let (cursor, monitors) = (st.cursor, &st.monitors);
        let popup = &self.deps().popup;
        if popup.is_visible() {
            popup.update(model);
        } else {
            popup.show_at(popup_origin(INITIAL_POPUP_SIZE, cursor, monitors), model);
        }
    }

    fn present_error(&self, generation: u64, message: crate::view_model::UserMessageView) {
        self.present(generation, &PopupViewModel::error(message, Vec::new(), &[]));
    }

    async fn text_flow(&self, operation: AIOperation, generation: u64) {
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
                detected_source: detected_source.clone(),
                target_language: target_language.clone(),
                target_window: captured.target,
                primary,
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
        let d = self.deps();
        let operation = request.operation;
        let primary = request.primary;
        let registry_has = |p: ProviderType| (d.registry_has)(p);
        let outcome = d.runner.run(request, snapshot).await;
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
            }
            Err(error) => {
                let (message, details) = match &error {
                    TranslationError::MissingApiKey => (missing_key(primary), Vec::new()),
                    other => translation_error_view(other),
                };
                let retry = retry_providers(primary, operation, snapshot, registry_has);
                self.present_locked(&mut st, &PopupViewModel::error(message, details, &retry));
            }
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
        fn show_at(&self, origin: Point, model: &PopupViewModel) {
            assert!(self
                .inner
                .get()
                .unwrap()
                .upgrade()
                .unwrap()
                .state
                .try_lock()
                .is_err());
            self.view.show_at(origin, model);
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
        });
        stores.inner.set(Arc::downgrade(&me.inner)).unwrap();
        popup.inner.set(Arc::downgrade(&me.inner)).unwrap();
        me.state().generation = 2;
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
