//! Phase 4a stub of the Screen Translate region picker.

use async_trait::async_trait;
use lingopop_orchestrator::RegionPicker;
use lingopop_platform::{PlatformError, Rect};

/// Always "cancelled". The capture overlay window arrives with Phase 4b, and the Screen
/// Translate hotkey is not registered until then, so this is never reached by the user.
pub struct NoPicker;

#[async_trait]
impl RegionPicker for NoPicker {
    async fn pick(&self) -> Result<Option<Rect>, PlatformError> {
        Ok(None)
    }
}
