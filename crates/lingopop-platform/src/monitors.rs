//! Monitor geometry (physical virtual-screen pixels) and the pure popup placement.

use crate::error::PlatformError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

/// A rectangle; `right()` / `bottom()` are exclusive, so containment is half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn left(&self) -> i32 {
        self.x
    }

    pub fn top(&self) -> i32 {
        self.y
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Half-open: the left / top edge is inside, the right / bottom edge is not.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    /// The overlap, `None` when it would be empty.
    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
    }

    fn union(&self, other: Rect) -> Rect {
        let left = self.left().min(other.left());
        let top = self.top().min(other.top());
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(left, top, right - left, bottom - top)
    }

    /// Squared distance from `p` to the nearest pixel of the rectangle (0 when inside).
    fn distance_sq(&self, p: Point) -> i64 {
        let axis = |v: i32, lo: i32, hi: i32| -> i64 {
            if v < lo {
                i64::from(lo) - i64::from(v)
            } else if v >= hi {
                i64::from(v) - i64::from(hi) + 1
            } else {
                0
            }
        };
        let dx = axis(p.x, self.left(), self.right());
        let dy = axis(p.y, self.top(), self.bottom());
        dx * dx + dy * dy
    }
}

/// One display in physical virtual-screen pixels; `scale_factor` = effective DPI / 96.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorInfo {
    pub bounds: Rect,
    pub work_area: Rect,
    pub scale_factor: f64,
    pub primary: bool,
}

pub trait Monitors: Send + Sync {
    fn cursor_position(&self) -> Result<Point, PlatformError>;
    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError>;
}

/// The monitor whose bounds contain `p` (half-open), else the nearest one (the earlier on a tie).
pub fn monitor_at(p: Point, monitors: &[MonitorInfo]) -> Option<&MonitorInfo> {
    monitors
        .iter()
        .find(|m| m.bounds.contains(p))
        .or_else(|| monitors.iter().min_by_key(|m| m.bounds.distance_sq(p)))
}

/// A logical size in physical pixels, each side rounded up.
pub fn physical_size(logical_width: f64, logical_height: f64, scale_factor: f64) -> Size {
    Size {
        width: (logical_width * scale_factor).ceil() as i32,
        height: (logical_height * scale_factor).ceil() as i32,
    }
}

/// The top-left that keeps a popup of `size` (physical pixels of the monitor under `anchor`)
/// inside that monitor's work area. A popup larger than the work area pins to its top-left.
pub fn clamp_to_work_area(size: Size, anchor: Point, monitors: &[MonitorInfo]) -> Point {
    let Some(m) = monitor_at(anchor, monitors) else {
        return anchor;
    };
    let wa = m.work_area;
    Point {
        x: anchor
            .x
            .min(wa.right().saturating_sub(size.width))
            .max(wa.left()),
        y: anchor
            .y
            .min(wa.bottom().saturating_sub(size.height))
            .max(wa.top()),
    }
}

/// The union of all monitor bounds (an empty rectangle for no monitors).
pub fn virtual_screen(monitors: &[MonitorInfo]) -> Rect {
    monitors
        .iter()
        .map(|m| m.bounds)
        .reduce(|a, b| a.union(b))
        .unwrap_or(Rect::new(0, 0, 0, 0))
}

/// Checks a capture region against the monitors and returns the part inside the virtual screen
/// (a partly outside region is clipped). Empty, non-positive or fully outside = `InvalidRegion`.
pub fn validate_region(region: Rect, monitors: &[MonitorInfo]) -> Result<Rect, PlatformError> {
    if region.is_empty() {
        return Err(PlatformError::InvalidRegion);
    }
    region
        .intersect(virtual_screen(monitors))
        .ok_or(PlatformError::InvalidRegion)
}
