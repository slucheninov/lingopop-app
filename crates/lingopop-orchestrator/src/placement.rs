//! Popup placement (port of `Core/PopupPlacement.swift`). Coordinates are physical pixels of the
//! virtual screen with y growing DOWN, so "below the cursor" is a larger y.

use lingopop_platform::monitors::{monitor_at, physical_size, MonitorInfo, Point};

/// Logical distance between the cursor and the popup.
pub const OFFSET: f64 = 12.0;
/// Logical margin kept free inside the work area.
pub const INSET: f64 = 8.0;

/// A logical length in physical pixels of a monitor with `scale`; rounded up like
/// `physical_size`.
fn scaled(logical: f64, scale: f64) -> i32 {
    (logical * scale).ceil() as i32
}

/// Top-left (physical px) of a popup of `size_logical` shown near `cursor`: right of the cursor
/// and below it, flipped above when it does not fit below the inset work area, and on the side
/// with more room when neither fits. The result is clamped into the inset work area (a popup
/// larger than the area pins to its top-left). No monitors: the cursor plus the offset at scale 1.
pub fn popup_origin(size_logical: (f64, f64), cursor: Point, monitors: &[MonitorInfo]) -> Point {
    let Some(m) = monitor_at(cursor, monitors) else {
        let off = scaled(OFFSET, 1.0);
        return Point {
            x: cursor.x.saturating_add(off),
            y: cursor.y.saturating_add(off),
        };
    };
    let size = physical_size(size_logical.0, size_logical.1, m.scale_factor);
    let off = scaled(OFFSET, m.scale_factor);
    let inset = scaled(INSET, m.scale_factor);
    let wa = m.work_area;
    let (left, top) = (wa.left() + inset, wa.top() + inset);
    let (right, bottom) = (wa.right() - inset, wa.bottom() - inset);

    let x = cursor.x.saturating_add(off);
    let below_y = cursor.y.saturating_add(off);
    let above_y = cursor.y.saturating_sub(off).saturating_sub(size.height);
    let y = if below_y.saturating_add(size.height) <= bottom {
        below_y
    } else if above_y >= top {
        above_y
    } else {
        let room_below = bottom - below_y;
        let room_above = cursor.y.saturating_sub(off) - top;
        if room_below >= room_above {
            below_y
        } else {
            above_y
        }
    };
    // Same order as the mac clamp: max last, so an oversized popup pins to the left / top.
    Point {
        x: x.min(right.saturating_sub(size.width)).max(left),
        y: y.min(bottom.saturating_sub(size.height)).max(top),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lingopop_platform::monitors::Rect;

    fn mon(x: i32, y: i32, w: i32, h: i32, scale: f64) -> MonitorInfo {
        MonitorInfo {
            bounds: Rect::new(x, y, w, h),
            work_area: Rect::new(x, y, w, h),
            scale_factor: scale,
            primary: x == 0,
        }
    }
    fn pt(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    #[test]
    fn place_below_right_of_cursor() {
        let m = [mon(0, 0, 1920, 1080, 1.0)];
        assert_eq!(popup_origin((300.0, 200.0), pt(100, 100), &m), pt(112, 112));
    }

    #[test]
    fn place_flips_above_when_no_room_below() {
        let m = [mon(0, 0, 1920, 1080, 1.0)];
        assert_eq!(
            popup_origin((300.0, 200.0), pt(100, 1000), &m),
            pt(112, 1000 - 12 - 200)
        );
    }

    #[test]
    fn place_uses_side_with_more_room_when_neither_fits() {
        // Height 400 on a 300-high monitor; cursor low: more room above (then clamped to top).
        let m = [mon(0, 0, 1920, 300, 1.0)];
        assert_eq!(popup_origin((300.0, 400.0), pt(100, 250), &m), pt(112, 8));
        // Cursor high: more room below, the popup is clamped to the top edge too.
        assert_eq!(popup_origin((300.0, 400.0), pt(100, 20), &m), pt(112, 8));
        // Cursor at 160 with height 280: below room 300-8-172=120, above room 148-8=140 -> above.
        assert_eq!(popup_origin((300.0, 280.0), pt(100, 160), &m), pt(112, 8));
    }

    #[test]
    fn place_clamps_right_edge_and_work_area() {
        let mut m = mon(0, 0, 1920, 1080, 1.0);
        m.work_area = Rect::new(0, 0, 1920, 1040);
        assert_eq!(
            popup_origin((300.0, 200.0), pt(1900, 500), &[m]),
            pt(1920 - 8 - 300, 512)
        );
        assert_eq!(
            popup_origin((300.0, 200.0), pt(100, 1035), &[m]),
            pt(112, 1035 - 12 - 200)
        );
    }

    #[test]
    fn place_on_left_monitor_negative_coords() {
        let m = [mon(-1920, 0, 1920, 1080, 1.0), mon(0, 0, 1920, 1080, 1.0)];
        let p = popup_origin((300.0, 200.0), pt(-500, 300), &m);
        assert_eq!(p, pt(-488, 312));
        let p = popup_origin((300.0, 200.0), pt(-5, 300), &m);
        assert!(p.x >= -1920 + 8 && p.x + 300 <= -8, "{p:?}");
        let p = popup_origin((300.0, 200.0), pt(-1919, 300), &m);
        assert!(p.x >= -1920 + 8 && p.x < 0, "{p:?}");
    }

    #[test]
    fn place_mixed_dpi_scales_by_monitor_under_cursor() {
        let m = [mon(0, 0, 1920, 1080, 1.0), mon(1920, 0, 3840, 2160, 2.0)];
        let p1 = popup_origin((300.0, 200.0), pt(100, 100), &m);
        let p2 = popup_origin((300.0, 200.0), pt(2020, 100), &m);
        assert_eq!(p1, pt(112, 112));
        assert_eq!(p2, pt(2020 + 24, 100 + 24));
        // Physical size doubles: it flips above by 2 * 200 + 24 at the bottom of the 4K monitor.
        let p3 = popup_origin((300.0, 200.0), pt(2020, 2100), &m);
        assert_eq!(p3, pt(2044, 2100 - 24 - 400));
    }

    #[test]
    fn place_without_monitors_returns_offset_cursor() {
        assert_eq!(popup_origin((300.0, 200.0), pt(50, 60), &[]), pt(62, 72));
    }
}
