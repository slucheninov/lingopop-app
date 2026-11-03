//! System language preferences, URL launching and process lifetime.
use crate::PlatformError;
use std::time::Duration;
pub trait SystemInfo: Send + Sync {
    fn ui_languages(&self) -> Vec<String>;
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
    fn wait_for_process_exit(&self, pid: u32, timeout: Duration) -> bool;
}
