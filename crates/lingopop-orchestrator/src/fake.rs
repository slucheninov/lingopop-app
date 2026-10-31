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
    /// A visible popup re-placed at its origin with the given logical size.
    Resize((f64, f64)),
    Hide,
}

/// Records every call; `show_at` makes it visible, `hide` invisible.
#[derive(Default)]
pub struct FakePopup {
    calls: Mutex<Vec<PopupCall>>,
    visible: AtomicBool,
    origin: Mutex<Option<Point>>,
}

impl FakePopup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> Vec<PopupCall> {
        lock(&self.calls).clone()
    }
}

impl PopupView for FakePopup {
    fn show_at(&self, origin: Point, model: &PopupViewModel) {
        lock(&self.calls).push(PopupCall::ShowAt(origin, model.clone()));
        *lock(&self.origin) = Some(origin);
        self.visible.store(true, Ordering::SeqCst);
    }

    fn update(&self, model: &PopupViewModel) {
        lock(&self.calls).push(PopupCall::Update(model.clone()));
    }

    /// Visible: records `Resize` and returns the rect at the last origin with the size rounded
    /// up (scale 1). Hidden: `None`, nothing recorded.
    fn resize(&self, size: (f64, f64)) -> Option<Rect> {
        if !self.is_visible() {
            return None;
        }
        let origin = (*lock(&self.origin))?;
        lock(&self.calls).push(PopupCall::Resize(size));
        Some(Rect::new(
            origin.x,
            origin.y,
            size.0.ceil() as i32,
            size.1.ceil() as i32,
        ))
    }

    fn hide(&self) {
        lock(&self.calls).push(PopupCall::Hide);
        self.visible.store(false, Ordering::SeqCst);
    }

    fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
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
    result: Mutex<Result<Option<Rect>, PlatformError>>,
    calls: AtomicUsize,
}

impl FakePicker {
    pub fn new(result: Result<Option<Rect>, PlatformError>) -> Self {
        Self {
            result: Mutex::new(result),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn set_result(&self, result: Result<Option<Rect>, PlatformError>) {
        *lock(&self.result) = result;
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl RegionPicker for FakePicker {
    async fn pick(&self) -> Result<Option<Rect>, PlatformError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        lock(&self.result).clone()
    }
}
