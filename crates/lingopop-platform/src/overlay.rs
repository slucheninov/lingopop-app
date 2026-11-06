//! Blocking native region selection in desktop space. Windows desktop units are physical
//! pixels (existing helpers below); Mac uses Quartz points and MacGeometry::selection_accepted
//! for the ten-backing-pixel threshold on the starting display (native overlay: Task 16).
use crate::{PlatformError, Point, Rect, Size};

pub trait RegionOverlay: Send + Sync {
    /// Disconnecting or sending on cancel ends the pick after native cleanup.
    fn pick(&self, cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError>;
    /// Like [`pick`](Self::pick), and also the display topology revision the selection was made
    /// against (macOS: `MacGeometry::topology_revision`), so the capture can refuse a layout
    /// that changed since. `None` = no revision (Windows has none; the default).
    fn pick_with_revision(
        &self,
        cancel: crossbeam_channel::Receiver<()>,
    ) -> Result<Option<(Rect, Option<u64>)>, PlatformError> {
        Ok(self.pick(cancel)?.map(|rect| (rect, None)))
    }
    /// Hands the overlay the main-thread dispatcher (macOS: native windows live there). Called
    /// once from shell setup, before the first pick; Windows ignores it.
    fn bind_main_thread(&self, _main: std::sync::Arc<dyn crate::MainThread>) {}
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

/// Ordered hook input, shared with portable batch-reduction tests.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum OverlayInput {
    Down(Point),
    Move(Point),
    Up(Point),
    Escape,
}

#[doc(hidden)]
#[derive(Debug, PartialEq, Eq)]
pub enum InputBatch {
    Unchanged,
    Redraw(Rect),
    Finish(Option<Rect>),
}

/// Fold in arrival order, keeping only the final pending geometry for one upload.
/// Down resets the anchor; Up uses its own (latest) point. Terminal events stop
/// the batch immediately, so later moves cannot change a finished selection.
/// A finishing overlay is removed without uploading an intermediate frame.
/// `rendered` is the selection rect of the last uploaded frame: a batch whose final selection
/// equals it is `Unchanged`, so identical moves never repaint or re-upload.
#[doc(hidden)]
pub fn fold_input_batch(
    start: &mut Option<Point>,
    rendered: Option<Rect>,
    events: impl IntoIterator<Item = OverlayInput>,
) -> InputBatch {
    fold_input_batch_with(start, rendered, events, selection_accepted)
}

/// [`fold_input_batch`] with the acceptance rule of a finishing `Up` injected: Windows uses
/// ten desktop units (physical pixels); macOS ten backing pixels on the display where the drag
/// started, which is not a function of the desktop-point size alone.
#[doc(hidden)]
pub fn fold_input_batch_with(
    start: &mut Option<Point>,
    rendered: Option<Rect>,
    events: impl IntoIterator<Item = OverlayInput>,
    accepted: impl Fn(Rect) -> bool,
) -> InputBatch {
    let mut pending = None;
    for event in events {
        let current = match event {
            OverlayInput::Escape => return InputBatch::Finish(None),
            OverlayInput::Down(p) => {
                *start = Some(p);
                p
            }
            OverlayInput::Move(p) => p,
            OverlayInput::Up(p) => {
                if let Some(anchor) = *start {
                    let r = selection_rect(anchor, p);
                    return InputBatch::Finish(accepted(r).then_some(r));
                }
                continue;
            }
        };
        if start.is_some() {
            pending = Some(current);
        }
    }
    match (*start, pending) {
        (Some(anchor), Some(current)) => {
            let r = selection_rect(anchor, current);
            if rendered == Some(r) {
                InputBatch::Unchanged
            } else {
                InputBatch::Redraw(r)
            }
        }
        _ => InputBatch::Unchanged,
    }
}
