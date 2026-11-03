use crate::{PlatformError, Rect, RegionOverlay};
pub struct MacOverlay;
impl RegionOverlay for MacOverlay {
    fn pick(&self, _: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError> {
        Err(PlatformError::Unsupported("region overlay on macOS"))
    }
}
