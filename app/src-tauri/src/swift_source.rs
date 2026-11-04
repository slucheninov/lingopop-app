//! Read-only detection of the frozen Swift app; also reused by import.
use lingopop_platform::{PlatformError, SystemInfo};
use std::sync::Arc;
#[derive(Clone)]
pub struct SwiftSource(Arc<dyn SystemInfo>);
impl SwiftSource {
    pub fn new(system: Arc<dyn SystemInfo>) -> Self {
        Self(system)
    }
    pub fn is_running(&self) -> Result<bool, PlatformError> {
        self.0.is_bundle_running("com.lingopop.app")
    }
}
