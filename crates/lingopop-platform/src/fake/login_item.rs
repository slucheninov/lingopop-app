use crate::{LoginItem, PlatformError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct FakeLoginItem {
    calls: Mutex<Vec<(bool, PathBuf)>>,
    fail: AtomicBool,
    next_error: Mutex<Option<PlatformError>>,
    settings_opened: AtomicUsize,
}

impl Default for FakeLoginItem {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeLoginItem {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(vec![]),
            fail: AtomicBool::new(false),
            next_error: Mutex::new(None),
            settings_opened: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> Vec<(bool, PathBuf)> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The next `set_enabled` or `open_settings` is recorded, then fails.
    pub fn fail_next(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }

    /// The next `set_enabled` or `open_settings` is recorded, then fails with `error`.
    pub fn fail_next_with(&self, error: PlatformError) {
        *self
            .next_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
    }

    /// How many times `open_settings` was called (also the failed ones).
    pub fn settings_opened(&self) -> usize {
        self.settings_opened.load(Ordering::SeqCst)
    }

    fn failure(&self) -> Result<(), PlatformError> {
        let scripted = self
            .next_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(error) = scripted {
            return Err(error);
        }
        if self.fail.swap(false, Ordering::SeqCst) {
            Err(PlatformError::Os {
                api: "RegSetValueExW",
                code: 5,
            })
        } else {
            Ok(())
        }
    }
}

impl LoginItem for FakeLoginItem {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((enabled, exe.to_path_buf()));
        self.failure()
    }

    fn open_settings(&self) -> Result<(), PlatformError> {
        self.settings_opened.fetch_add(1, Ordering::SeqCst);
        self.failure()
    }
}
