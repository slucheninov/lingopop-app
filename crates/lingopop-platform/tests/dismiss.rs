use lingopop_platform::{classify_click, DismissEvent, Point, Rect};
#[test]
fn dismiss_click_inside_rect_is_ignored() {
    for (x, y) in [(10, 20), (39, 59)] {
        assert_eq!(
            classify_click(Point { x, y }, Rect::new(10, 20, 30, 40)),
            None
        );
    }
}
#[test]
fn dismiss_click_outside_rect_is_reported() {
    for (x, y) in [(9, 20), (10, 19), (40, 20), (10, 60)] {
        assert_eq!(
            classify_click(Point { x, y }, Rect::new(10, 20, 30, 40)),
            Some(DismissEvent::ClickOutside)
        );
    }
}
