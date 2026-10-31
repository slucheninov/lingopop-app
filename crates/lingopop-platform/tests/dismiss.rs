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

#[test]
fn fake_dismiss_records_calls_and_only_fires_while_active() {
    use lingopop_platform::{
        fake::{DismissCall, FakeDismissWatcher},
        DismissWatcher,
    };
    let fake = FakeDismissWatcher::default();
    let events = fake.events();
    fake.fire(DismissEvent::Escape);
    assert!(events.try_recv().is_err());
    let rect = Rect::new(1, 2, 3, 4);
    fake.start(rect).unwrap();
    fake.fire(DismissEvent::Escape);
    assert_eq!(events.try_recv().unwrap(), DismissEvent::Escape);
    fake.update_rect(rect);
    fake.stop();
    fake.fire(DismissEvent::ClickOutside);
    assert!(events.try_recv().is_err());
    assert_eq!(
        fake.calls(),
        [
            DismissCall::Start(rect),
            DismissCall::UpdateRect(rect),
            DismissCall::Stop
        ]
    );
}
#[test]
fn fake_popup_records_visibility_calls_in_order() {
    use lingopop_platform::{
        fake::{FakePopupWindow, PopupWindowCall},
        NoActivateWindow, WindowHandle,
    };
    let fake = FakePopupWindow::default();
    let window = WindowHandle(12);
    let rect = Rect::new(-10, 20, 30, 40);
    fake.show_at(window, rect).unwrap();
    fake.hide(window).unwrap();
    assert_eq!(
        fake.calls(),
        [
            PopupWindowCall::ShowAt(window, rect),
            PopupWindowCall::Hide(window)
        ]
    );
}
