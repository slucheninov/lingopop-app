use lingopop_orchestrator::presentation::*;
use lingopop_platform::Point;
use lingopop_orchestrator::PopupViewModel;
use lingopop_core::operation::AIOperation;

fn show(id: PresentationId) -> PresentationCommand {
    PresentationCommand { id, kind: PresentationKind::Show { origin: Point { x: 0, y: 0 }, size_logical: (360., 100.), model: PopupViewModel::loading(AIOperation::Translate, "fixture") } }
}
#[tokio::test]
async fn old_show_queued_during_new_capture_has_zero_effects() {
    let fence = PresentationFence::default();
    let queue = PresentationQueue::default();
    let id = PresentationId { generation: 1, sequence: 1 };
    fence.advance(id);
    let rx = queue.submit(show(id)).unwrap();
    fence.advance(PresentationId { generation: 2, sequence: 2 });
    let job = queue.try_next().unwrap();
    let mut stale_native_effect_count = 0;
    let ack = fence.apply_if_current(job.command.id, || { stale_native_effect_count += 1; Ok(PresentationVisibility::Visible) }).unwrap();
    job.complete(ack.clone(), &|_| {});
    let old_show_ack = rx.await.unwrap();
    let visibility = Visibility::Hidden;
    assert_eq!(stale_native_effect_count, 0);
    assert_eq!(old_show_ack.result, Ok(PresentationVisibility::Superseded));
    assert_eq!(visibility, Visibility::Hidden);
}
#[tokio::test]
async fn hide_bypasses_capacity_and_supersedes_pending() {
    let q = PresentationQueue::default();
    let mut receivers = Vec::new();
    for sequence in 1..=32 { receivers.push(q.submit(show(PresentationId { generation: sequence, sequence })).unwrap()); }
    assert_eq!(q.submit(show(PresentationId { generation: 40, sequence: 40 })).unwrap_err(), PresentationQueueError::Full);
    let hide = q.submit(PresentationCommand { id: PresentationId { generation: 1, sequence: 41 }, kind: PresentationKind::Hide }).unwrap();
    q.close(&|_| {});
    for rx in receivers { assert_eq!(rx.await.unwrap().result, Ok(PresentationVisibility::Superseded)); }
    assert_eq!(hide.await.unwrap().result, Ok(PresentationVisibility::Superseded));
    assert_eq!(q.submit(show(PresentationId { generation: 50, sequence: 50 })).unwrap_err(), PresentationQueueError::Closed);
}
#[test]
fn stale_after_effect_is_superseded_and_reentrant() {
    let fence = PresentationFence::default();
    let id = PresentationId { generation: 1, sequence: 1 }; fence.advance(id);
    let ack = fence.apply_if_current(id, || { fence.advance(PresentationId { generation: 2, sequence: 2 }); Ok(PresentationVisibility::Visible) }).unwrap();
    assert_eq!(ack.result, Ok(PresentationVisibility::Superseded));
}
