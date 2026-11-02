use crate::{LoginItem, PlatformError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct FakeLoginItem {
    calls: Mutex<Vec<(bool, PathBuf)>>,
    fail: AtomicBool,
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
        }
    }

    pub fn calls(&self) -> Vec<(bool, PathBuf)> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The next `set_enabled` is recorded, then fails.
    pub fn fail_next(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }
}

impl LoginItem for FakeLoginItem {
    fn set_enabled(&self, enabled: bool, exe: &Path) -> Result<(), PlatformError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((enabled, exe.to_path_buf()));
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
