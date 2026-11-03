use lingopop_platform::{overlay::*, Point, Rect, Size};

#[test]
fn selection_rect_normalizes_drag_direction() {
    assert_eq!(
        selection_rect(Point { x: 100, y: 100 }, Point { x: 20, y: 40 }),
        Rect::new(20, 40, 80, 60)
    );
}
#[test]
fn selection_accepted_threshold() {
    assert!(!selection_accepted(Rect::new(0, 0, 9, 10)));
    assert!(selection_accepted(Rect::new(0, 0, 10, 10)));
}
#[test]
fn border_px_scales() {
    assert_eq!([border_px(1.0), border_px(1.5), border_px(2.0)], [2, 3, 3]);
}
#[test]
fn label_origin_stays_inside_bounds() {
    for bounds in [Rect::new(0, 0, 200, 100), Rect::new(-200, -100, 200, 100)] {
        let size = Size {
            width: 50,
            height: 20,
        };
        let p = label_origin(bounds, size, bounds);
        assert_eq!(
            p,
            Point {
                x: bounds.right() - 54,
                y: bounds.bottom() - 24
            }
        );
        let p = label_origin(Rect::new(bounds.x, bounds.y, 2, 2), size, bounds);
        assert_eq!(
            p,
            Point {
                x: bounds.x,
                y: bounds.y
            }
        );
    }
}
#[test]
fn size_label_uses_multiplication_sign() {
    assert_eq!(size_label(Rect::new(0, 0, 120, 40)), "120 × 40");
}
