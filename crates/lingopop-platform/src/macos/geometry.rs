//! Pure macOS geometry, also compiled on other hosts for contract tests.
//! Desktop space is Quartz global points (top-left, Y-down); backing pixels are local to a
//! display. No native calls or display access occur here.
use crate::{MonitorInfo, PlatformError, Rect};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl NativeRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    /// Floor origins and ceil far edges: native displays never shrink when exposed as integers.
    pub fn enclosing_rect(self) -> Result<Rect, PlatformError> {
        let right = (self.x + self.width).ceil();
        let bottom = (self.y + self.height).ceil();
        let x = self.x.floor();
        let y = self.y.floor();
        if ![x, y, right, bottom, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
            || x < i32::MIN as f64
            || y < i32::MIN as f64
            || right > i32::MAX as f64
            || bottom > i32::MAX as f64
            || right - x > i32::MAX as f64
            || bottom - y > i32::MAX as f64
        {
            return Err(PlatformError::InvalidRegion);
        }
        Ok(Rect::new(
            x as i32,
            y as i32,
            (right - x) as i32,
            (bottom - y) as i32,
        ))
    }
    /// Work areas round inward so popup clamps cannot cover the menu bar or Dock.
    pub fn contained_rect(self) -> Result<Rect, PlatformError> {
        self.enclosing_rect()?;
        let x = self.x.ceil();
        let y = self.y.ceil();
        let right = (self.x + self.width).floor();
        let bottom = (self.y + self.height).floor();
        if right <= x || bottom <= y {
            return Err(PlatformError::InvalidRegion);
        }
        Ok(Rect::new(
            x as i32,
            y as i32,
            (right - x) as i32,
            (bottom - y) as i32,
        ))
    }
    fn intersect(self, b: Self) -> Option<Self> {
        let x = self.x.max(b.x);
        let y = self.y.max(b.y);
        let r = (self.x + self.width).min(b.x + b.width);
        let d = (self.y + self.height).min(b.y + b.height);
        (r > x && d > y).then(|| Self::new(x, y, r - x, d - y))
    }
}
impl From<Rect> for NativeRect {
    fn from(r: Rect) -> Self {
        Self::new(r.x.into(), r.y.into(), r.width.into(), r.height.into())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct MacDisplay {
    pub id: u32,
    pub desktop_bounds: NativeRect,
    pub work_area: NativeRect,
    pub backing_scale: f64,
    pub primary: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureTile {
    pub display_id: u32,
    /// Exact desktop overlap; use this to constrain sampling at fractional pixel edges.
    pub desktop_rect: NativeRect,
    /// Display-local backing pixels, rounded outward. Never capture another display's contents.
    pub pixel_crop: Rect,
    /// Output pixel rectangle. Resample the crop into this rectangle, clipping to desktop_rect.
    pub destination: Rect,
    pub backing_scale: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CapturePlan {
    pub topology_revision: u64,
    pub desktop_rect: Rect,
    pub output_scale: f64,
    pub output_size: (u32, u32),
    pub tiles: Vec<CaptureTile>,
    /// Initialize the entire output to opaque black; gaps must never expose unrelated content.
    pub background_rgba: [u8; 4],
}
#[derive(Debug, Clone)]
pub struct MacGeometry {
    revision: u64,
    primary_height: f64,
    displays: Vec<MacDisplay>,
}
impl MacGeometry {
    pub fn new(
        revision: u64,
        primary_height: f64,
        displays: Vec<MacDisplay>,
    ) -> Result<Self, PlatformError> {
        if !primary_height.is_finite() || primary_height < 0.0 {
            return Err(PlatformError::InvalidRegion);
        }
        for (i, d) in displays.iter().enumerate() {
            d.desktop_bounds.enclosing_rect()?;
            d.work_area.contained_rect()?;
            if !d.backing_scale.is_finite()
                || d.backing_scale <= 0.0
                || displays[..i].iter().any(|p| p.id == d.id)
            {
                return Err(PlatformError::InvalidRegion);
            }
        }
        Ok(Self {
            revision,
            primary_height,
            displays,
        })
    }
    pub fn topology_revision(&self) -> u64 {
        self.revision
    }
    pub fn displays(&self) -> &[MacDisplay] {
        &self.displays
    }
    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.displays
            .iter()
            .map(|d| MonitorInfo {
                bounds: d
                    .desktop_bounds
                    .enclosing_rect()
                    .expect("validated display"),
                work_area: d.work_area.contained_rect().expect("validated work area"),
                scale_factor: 1.0,
                primary: d.primary,
            })
            .collect()
    }
    pub fn desktop_to_appkit(&self, r: Rect) -> Result<NativeRect, PlatformError> {
        if r.is_empty() {
            return Err(PlatformError::InvalidRegion);
        }
        Ok(NativeRect::new(
            r.x.into(),
            self.primary_height - f64::from(r.y) - f64::from(r.height),
            r.width.into(),
            r.height.into(),
        ))
    }
    /// Exact AppKit (bottom-left, Y-up) frame to Quartz (top-left, Y-down) points.
    /// Used for both NSScreen.frame and visibleFrame, before integer rounding.
    pub fn appkit_frame_to_desktop(primary_height: f64, r: NativeRect) -> NativeRect {
        NativeRect::new(r.x, primary_height - r.y - r.height, r.width, r.height)
    }
    pub fn appkit_to_desktop(&self, r: NativeRect) -> Result<Rect, PlatformError> {
        Self::appkit_frame_to_desktop(self.primary_height, r).enclosing_rect()
    }
    /// Task 15 must call this immediately before every native capture. Revision changes fail
    /// closed even if the same display IDs remain (scale/work-area/origin may have changed).
    pub fn ensure_current(&self, plan: &CapturePlan) -> Result<(), PlatformError> {
        if self.revision != plan.topology_revision
            || plan
                .tiles
                .iter()
                .any(|t| !self.displays.iter().any(|d| d.id == t.display_id))
        {
            Err(PlatformError::InvalidRegion)
        } else {
            Ok(())
        }
    }
    /// Ten backing pixels on the display where the drag started (Task 16's overlay).
    pub fn selection_accepted(&self, start_display: u32, r: Rect) -> Result<bool, PlatformError> {
        let d = self
            .displays
            .iter()
            .find(|d| d.id == start_display)
            .ok_or(PlatformError::InvalidRegion)?;
        Ok(f64::from(r.width) * d.backing_scale >= 10.0
            && f64::from(r.height) * d.backing_scale >= 10.0)
    }
    pub fn capture_plan(&self, r: Rect) -> Result<CapturePlan, PlatformError> {
        if r.is_empty() {
            return Err(PlatformError::InvalidRegion);
        }
        let overlaps: Vec<_> = self
            .displays
            .iter()
            .filter_map(|d| d.desktop_bounds.intersect(r.into()).map(|v| (d, v)))
            .collect();
        let scale = overlaps
            .iter()
            .map(|(d, _)| d.backing_scale)
            .reduce(f64::max)
            .ok_or(PlatformError::InvalidRegion)?;
        let output = NativeRect::new(
            0.0,
            0.0,
            f64::from(r.width) * scale,
            f64::from(r.height) * scale,
        )
        .enclosing_rect()?;
        let mut tiles = Vec::new();
        for (d, v) in overlaps {
            let crop = NativeRect::new(
                (v.x - d.desktop_bounds.x) * d.backing_scale,
                (v.y - d.desktop_bounds.y) * d.backing_scale,
                v.width * d.backing_scale,
                v.height * d.backing_scale,
            )
            .enclosing_rect()?;
            let destination = NativeRect::new(
                (v.x - f64::from(r.x)) * scale,
                (v.y - f64::from(r.y)) * scale,
                v.width * scale,
                v.height * scale,
            )
            .enclosing_rect()?;
            tiles.push(CaptureTile {
                display_id: d.id,
                desktop_rect: v,
                pixel_crop: crop,
                destination,
                backing_scale: d.backing_scale,
            });
        }
        Ok(CapturePlan {
            topology_revision: self.revision,
            desktop_rect: r,
            output_scale: scale,
            output_size: (output.width as u32, output.height as u32),
            tiles,
            background_rgba: [0, 0, 0, 255],
        })
    }
}
/// Shared immutable snapshots. Native publication occurs on the main thread; reads never
/// dispatch or require a main-thread marker. No lock is held while using a snapshot.
#[derive(Debug, Clone)]
pub struct TopologyCache(Arc<RwLock<MacGeometry>>);
impl TopologyCache {
    pub fn new(geometry: MacGeometry) -> Self {
        Self(Arc::new(RwLock::new(geometry)))
    }
    pub fn snapshot(&self) -> MacGeometry {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    pub fn topology_revision(&self) -> u64 {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .revision
    }
    /// Register first so even a failed initial enumeration can recover on a notification.
    /// The injected operations also let display-independent tests exercise startup ordering.
    pub fn initialize_with(
        &self,
        register_observer: impl FnOnce(),
        read_topology: impl FnOnce() -> Result<(f64, Vec<MacDisplay>), PlatformError>,
        log: &impl Fn(&str),
    ) {
        register_observer();
        self.refresh_with(read_topology, log);
    }
    /// Failed reads or invalid geometry invalidate old plans, without failing app setup.
    pub fn refresh_with(
        &self,
        read_topology: impl FnOnce() -> Result<(f64, Vec<MacDisplay>), PlatformError>,
        log: &impl Fn(&str),
    ) {
        if read_topology()
            .and_then(|(height, displays)| self.publish(height, displays))
            .is_err()
        {
            let _ = self.publish(0.0, vec![]);
            log("display topology refresh failed");
        }
    }
    pub fn publish(
        &self,
        primary_height: f64,
        displays: Vec<MacDisplay>,
    ) -> Result<(), PlatformError> {
        let mut current = self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current = MacGeometry::new(
            current
                .revision
                .checked_add(1)
                .ok_or(PlatformError::InvalidRegion)?,
            primary_height,
            displays,
        )?;
        Ok(())
    }
}
