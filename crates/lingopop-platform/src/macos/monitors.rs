//! `Monitors` over Core Graphics. Coordinates are POINTS, not physical pixels (dev only);
//! `scale_factor` is the backing scale (pixels per point).

use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use crate::error::PlatformError;
use crate::monitors::{MonitorInfo, Monitors, Point, Rect};

/// The whole display bounds stand in for the work area (menu bar and Dock are not subtracted).
#[derive(Debug, Default, Clone, Copy)]
pub struct MacMonitors;

impl MacMonitors {
    pub fn new() -> Self {
        Self
    }
}

impl Monitors for MacMonitors {
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        let err = PlatformError::Os {
            api: "CGEventCreate",
            code: 0,
        };
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| err.clone())?;
        let p = CGEvent::new(source).map_err(|_| err)?.location();
        Ok(Point {
            x: p.x.floor() as i32,
            y: p.y.floor() as i32,
        })
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        let ids = CGDisplay::active_displays().map_err(|code| PlatformError::Os {
            api: "CGGetActiveDisplayList",
            code,
        })?;
        Ok(ids
            .into_iter()
            .map(|id| {
                let display = CGDisplay::new(id);
                let b = display.bounds();
                let bounds = Rect::new(
                    b.origin.x.round() as i32,
                    b.origin.y.round() as i32,
                    b.size.width.round() as i32,
                    b.size.height.round() as i32,
                );
                // `CGDisplayPixelsWide` reports the scaled (logical) width on Retina panels, so
                // the backing pixel width comes from the current display mode when available.
                let pixels = display
                    .display_mode()
                    .map(|mode| mode.pixel_width())
                    .filter(|&w| w > 0)
                    .unwrap_or_else(|| display.pixels_wide());
                let scale_factor = if b.size.width > 0.0 {
                    (pixels as f64 / b.size.width).max(1.0)
                } else {
                    1.0
                };
                MonitorInfo {
                    bounds,
                    work_area: bounds,
                    scale_factor,
                    primary: display.is_main(),
                }
            })
            .collect())
    }
}
