//! Recording fakes of the orchestrator's UI and storage seams (always compiled so the app
//! crate's tests can reuse them).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use lingopop_core::records::{HistoryEntry, UsageRecord};
use lingopop_core::runner::ConfigSnapshot;
use lingopop_core::settings::Settings;
use lingopop_platform::{PlatformError, Point, Rect};

use crate::orchestrator::{ConfigSource, Persist};
use crate::popup::PopupView;
use crate::screen::RegionPicker;
use crate::view_model::PopupViewModel;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug, Clone, PartialEq)]
pub enum PopupCall {
    ShowAt(Point, PopupViewModel),
    Update(PopupViewModel),
    /// A visible popup moved to the origin and resized to the logical size.
    Resize(Point, (f64, f64)),
    Hide,
}

/// Queued fake. Read helpers are barriers: they wait for all submitted acknowledgments.
#[derive(Default)]
struct PopupState {
    calls: Mutex<Vec<PopupCall>>,
    visible: AtomicBool,
    sizes: Mutex<Vec<(f64, f64)>>,
    fail_show: AtomicBool,
    resize_hold: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    outstanding: Mutex<usize>,
    done: std::sync::Condvar,
}
#[derive(Default)]
pub struct FakePopup {
    state: std::sync::Arc<PopupState>,
    queue: std::sync::Arc<crate::PresentationQueue>,
}
impl FakePopup {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn flush(&self) {
        let mut n = lock(&self.state.outstanding);
        while *n != 0 {
            n = self.state.done.wait(n).unwrap();
        }
    }
    pub fn calls(&self) -> Vec<PopupCall> {
        self.flush();
        lock(&self.state.calls).clone()
    }
    /// Holds the next native Resize inside its effect: the first receiver fires when the effect
    /// is entered, the sender releases it.
    pub fn hold_next_resize(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *lock(&self.state.resize_hold) = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }
    pub fn fail_show(&self, fail: bool) {
        self.state.fail_show.store(fail, Ordering::SeqCst);
    }
    pub fn show_sizes(&self) -> Vec<(f64, f64)> {
        self.flush();
        lock(&self.state.sizes).clone()
    }
    pub fn is_visible(&self) -> bool {
        self.flush();
        self.state.visible.load(Ordering::SeqCst)
    }
}
impl Drop for FakePopup {
    fn drop(&mut self) {
        self.queue.seal();
    }
}
impl PopupView for FakePopup {
    fn submit(
        &self,
        cmd: crate::PresentationCommand,
    ) -> Result<crate::AckReceiver, crate::PresentationQueueError> {
        *lock(&self.state.outstanding) += 1;
        let result = self.queue.submit(cmd);
        if result.is_err() {
            *lock(&self.state.outstanding) -= 1;
            self.state.done.notify_all();
        }
        result
    }
    fn bind(
        &self,
        fence: std::sync::Arc<crate::PresentationFence>,
        sink: std::sync::Arc<crate::AckSink>,
    ) {
        let queue = self.queue.clone();
        let state = self.state.clone();
        let self_placeholder = state.clone();
        std::thread::spawn(move || {
            let complete = move |ack| {
                sink(ack);
                *lock(&state.outstanding) -= 1;
                state.done.notify_all();
            };
            while let Some(job) = queue.next(&complete) {
                let ack = fence
                    .apply_if_current(job.command.id, || {
                        use crate::{PresentationKind::*, PresentationVisibility::*};
                        match &job.command.kind {
                            Show {
                                origin,
                                size_logical,
                                model,
                            } => {
                                lock(&self_placeholder.calls)
                                    .push(PopupCall::ShowAt(*origin, model.clone()));
                                lock(&self_placeholder.sizes).push(*size_logical);
                                if self_placeholder.fail_show.load(Ordering::SeqCst) {
                                    self_placeholder.visible.store(false, Ordering::SeqCst);
                                    return Err(PlatformError::Unsupported("fake show"));
                                }
                                self_placeholder.visible.store(true, Ordering::SeqCst);
                            }
                            Update { model } => {
                                lock(&self_placeholder.calls).push(PopupCall::Update(model.clone()))
                            }
                            Resize {
                                origin,
                                size_logical,
                            } => {
                                lock(&self_placeholder.calls)
                                    .push(PopupCall::Resize(*origin, *size_logical));
                                let hold = lock(&self_placeholder.resize_hold).take();
                                if let Some((entered, release)) = hold {
                                    let _ = entered.send(());
                                    let _ = release.recv();
                                }
                            }
                            Hide => {
                                lock(&self_placeholder.calls).push(PopupCall::Hide);
                                self_placeholder.visible.store(false, Ordering::SeqCst);
                            }
                        }
                        Ok(if self_placeholder.visible.load(Ordering::SeqCst) {
                            Visible
                        } else {
                            Hidden
                        })
                    })
                    .unwrap();
                job.complete(ack, &complete);
            }
        });
    }
}

/// Settings and snapshot behind a mutex, with setters.
pub struct FakeConfig {
    settings: Mutex<Settings>,
    snapshot: Mutex<ConfigSnapshot>,
}

impl FakeConfig {
    pub fn new(settings: Settings, snapshot: ConfigSnapshot) -> Self {
        Self {
            settings: Mutex::new(settings),
            snapshot: Mutex::new(snapshot),
        }
    }

    pub fn set_settings(&self, settings: Settings) {
        *lock(&self.settings) = settings;
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        f(&mut lock(&self.settings));
    }

    pub fn set_snapshot(&self, snapshot: ConfigSnapshot) {
        *lock(&self.snapshot) = snapshot;
    }
}

impl ConfigSource for FakeConfig {
    fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    fn snapshot(&self) -> ConfigSnapshot {
        lock(&self.snapshot).clone()
    }
}

/// Records what would be written to the history and usage stores.
#[derive(Default)]
pub struct FakePersist {
    history: Mutex<Vec<(HistoryEntry, u32)>>,
    usage: Mutex<Vec<UsageRecord>>,
}

impl FakePersist {
    pub fn new() -> Self {
        Self::default()
    }

    /// `(entry, max_count)` per `add_history` call, in order.
    pub fn history(&self) -> Vec<(HistoryEntry, u32)> {
        lock(&self.history).clone()
    }

    pub fn usage(&self) -> Vec<UsageRecord> {
        lock(&self.usage).clone()
    }
}

impl Persist for FakePersist {
    fn add_history(&self, entry: HistoryEntry, max_count: u32) {
        lock(&self.history).push((entry, max_count));
    }

    fn append_usage(&self, record: UsageRecord) {
        lock(&self.usage).push(record);
    }
}

/// A picker that returns a preset result and counts its calls.
pub struct FakePicker {
    blocked: std::sync::atomic::AtomicBool,
    cancelled: AtomicUsize,
    result: Mutex<Result<Option<Rect>, PlatformError>>,
    calls: AtomicUsize,
}

impl FakePicker {
    pub fn new(result: Result<Option<Rect>, PlatformError>) -> Self {
        Self {
            blocked: false.into(),
            cancelled: 0.into(),
            result: Mutex::new(result),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn set_result(&self, result: Result<Option<Rect>, PlatformError>) {
        *lock(&self.result) = result;
    }

    pub fn block_forever(&self) {
        self.blocked.store(true, Ordering::SeqCst);
    }
    pub fn cancelled(&self) -> usize {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl RegionPicker for FakePicker {
    async fn pick(&self) -> Result<Option<Rect>, PlatformError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.blocked.load(Ordering::SeqCst) {
            struct Cancel<'a>(&'a AtomicUsize);
            impl Drop for Cancel<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _guard = Cancel(&self.cancelled);
            std::future::pending::<()>().await;
        }
        lock(&self.result).clone()
    }
}

#[derive(Default)]
pub struct FakeStatus {
    statuses: Mutex<Vec<crate::TrayStatus>>,
}
impl FakeStatus {
    pub fn statuses(&self) -> Vec<crate::TrayStatus> {
        lock(&self.statuses).clone()
    }
}
impl crate::StatusSink for FakeStatus {
    fn status(&self, status: crate::TrayStatus) {
        lock(&self.statuses).push(status);
    }
}
