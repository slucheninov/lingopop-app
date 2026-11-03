//! Blocking native region selection and physical-pixel geometry.
use crate::{PlatformError, Point, Rect, Size};

pub trait RegionOverlay: Send + Sync {
    /// Disconnecting or sending on cancel ends the pick after native cleanup.
    fn pick(&self, cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError>;
}
#[doc(hidden)]
pub const MIN_SELECTION_PX: i32 = 10;
#[doc(hidden)]
pub const DIM_ALPHA: u8 = 77;
#[doc(hidden)]
pub const BORDER_PT: f64 = 1.5;
#[doc(hidden)]
pub const LABEL_PT: f64 = 11.0;
#[doc(hidden)]
pub fn selection_rect(start: Point, current: Point) -> Rect {
    Rect::new(
        start.x.min(current.x),
        start.y.min(current.y),
        start.x.abs_diff(current.x).min(i32::MAX as u32) as i32,
        start.y.abs_diff(current.y).min(i32::MAX as u32) as i32,
    )
}
#[doc(hidden)]
pub fn selection_accepted(r: Rect) -> bool {
    r.width >= MIN_SELECTION_PX && r.height >= MIN_SELECTION_PX
}
#[doc(hidden)]
pub fn border_px(scale: f64) -> i32 {
    (BORDER_PT * scale).ceil() as i32
}
#[doc(hidden)]
pub fn label_origin(sel: Rect, label: Size, bounds: Rect) -> Point {
    Point {
        x: (sel.right().saturating_sub(label.width).saturating_sub(4))
            .min(bounds.right().saturating_sub(label.width))
            .max(bounds.x),
        y: (sel.bottom().saturating_sub(label.height).saturating_sub(4))
            .min(bounds.bottom().saturating_sub(label.height))
            .max(bounds.y),
    }
}
#[doc(hidden)]
pub fn size_label(r: Rect) -> String {
    format!("{} × {}", r.width, r.height)
}
