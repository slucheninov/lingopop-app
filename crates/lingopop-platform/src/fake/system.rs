use crate::{PlatformError, SystemInfo};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
pub struct FakeSystem {
    languages: Vec<String>,
    running: Mutex<Result<bool, PlatformError>>,
    opened: Mutex<Vec<String>>,
    exits: AtomicBool,
}
impl FakeSystem {
    pub fn new(languages: Vec<String>) -> Self {
        Self {
            languages,
            running: Mutex::new(Ok(false)),
            opened: Mutex::new(vec![]),
            exits: AtomicBool::new(true),
        }
    }
    pub fn set_bundle_running(&self, result: Result<bool, PlatformError>) {
        *self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = result;
    }
    pub fn opened(&self) -> Vec<String> {
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn set_process_exits(&self, exits: bool) {
        self.exits.store(exits, Ordering::SeqCst);
    }
}
impl SystemInfo for FakeSystem {
    fn is_bundle_running(&self, bundle_id: &str) -> Result<bool, PlatformError> {
        assert_eq!(bundle_id, "com.lingopop.app");
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn ui_languages(&self) -> Vec<String> {
        self.languages.clone()
    }
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(url.into());
        Ok(())
    }
    fn wait_for_process_exit(&self, _pid: u32, _timeout: Duration) -> bool {
        self.exits.load(Ordering::SeqCst)
    }
}
