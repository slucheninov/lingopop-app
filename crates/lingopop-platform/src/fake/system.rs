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
    opened: Mutex<Vec<String>>,
    exits: AtomicBool,
}
impl FakeSystem {
    pub fn new(languages: Vec<String>) -> Self {
        Self {
            languages,
            opened: Mutex::new(vec![]),
            exits: AtomicBool::new(true),
        }
    }
    pub fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
    }
    pub fn set_process_exits(&self, exits: bool) {
        self.exits.store(exits, Ordering::SeqCst);
    }
}
impl SystemInfo for FakeSystem {
    fn ui_languages(&self) -> Vec<String> {
        self.languages.clone()
    }
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.opened.lock().unwrap().push(url.into());
        Ok(())
    }
    fn wait_for_process_exit(&self, _pid: u32, _timeout: Duration) -> bool {
        self.exits.load(Ordering::SeqCst)
    }
}
