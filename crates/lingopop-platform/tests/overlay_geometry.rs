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

fn point(x: i32, y: i32) -> Point {
    Point { x, y }
}
#[test]
fn batch_coalesces_a_move_backlog_to_one_redraw() {
    let mut start = Some(point(0, 0));
    let moves = (1..=10_000).map(|n| OverlayInput::Move(point(n, n)));
    assert_eq!(
        fold_input_batch(&mut start, moves),
        InputBatch::Redraw(Rect::new(0, 0, 10_000, 10_000))
    );
    assert_eq!(fold_input_batch(&mut start, []), InputBatch::Unchanged);
}
#[test]
fn batch_preserves_down_order_and_up_position_across_batches() {
    use OverlayInput::*;
    let mut start = None;
    assert_eq!(
        fold_input_batch(&mut start, [Move(point(99, 99)), Up(point(99, 99))]),
        InputBatch::Unchanged
    );
    assert_eq!(
        fold_input_batch(
            &mut start,
            [
                Down(point(0, 0)),
                Move(point(80, 80)),
                Down(point(50, 50)),
                Move(point(30, 30))
            ]
        ),
        InputBatch::Redraw(Rect::new(30, 30, 20, 20))
    );
    assert_eq!(
        fold_input_batch(
            &mut start,
            [
                Move(point(20, 20)),
                Up(point(10, 10)),
                Move(point(99, 99)),
                Escape
            ]
        ),
        InputBatch::Finish(Some(Rect::new(10, 10, 40, 40)))
    );
}
#[test]
fn batch_terminal_events_discard_pending_redraw() {
    use OverlayInput::*;
    let mut start = Some(point(0, 0));
    assert_eq!(
        fold_input_batch(&mut start, [Move(point(50, 50)), Escape, Up(point(90, 90))]),
        InputBatch::Finish(None)
    );
    assert_eq!(
        fold_input_batch(&mut start, [Move(point(50, 50)), Up(point(9, 10))]),
        InputBatch::Finish(None)
    );
}
