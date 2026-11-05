use lingopop_core::operation::AIOperation;
use lingopop_orchestrator::presentation::*;
use lingopop_orchestrator::PopupViewModel;
use lingopop_platform::Point;

fn show(id: PresentationId) -> PresentationCommand {
    PresentationCommand {
        id,
        kind: PresentationKind::Show {
            origin: Point { x: 0, y: 0 },
            size_logical: (360., 100.),
            model: PopupViewModel::loading(AIOperation::Translate, "fixture"),
        },
    }
}
#[tokio::test]
async fn stale_queue_job_has_zero_effects() {
    let fence = PresentationFence::default();
    let queue = PresentationQueue::default();
    let id = PresentationId {
        generation: 1,
        sequence: 1,
    };
    fence.advance(id);
    let rx = queue.submit(show(id)).unwrap();
    fence.advance(PresentationId {
        generation: 2,
        sequence: 2,
    });
    let job = queue.try_next().unwrap();
    let mut stale_native_effect_count = 0;
    let mut native_visible = false;
    let ack = fence
        .apply_if_current(job.command.id, || {
            stale_native_effect_count += 1;
            native_visible = true;
            Ok(PresentationVisibility::Visible)
        })
        .unwrap();
    job.complete(ack.clone(), &|_| {});
    let old_show_ack = rx.await.unwrap();
    assert_eq!(stale_native_effect_count, 0);
    assert_eq!(old_show_ack.result, Ok(PresentationVisibility::Superseded));
    assert!(!native_visible);
}
#[tokio::test]
async fn hide_bypasses_capacity_and_supersedes_pending() {
    let q = PresentationQueue::default();
    let id = |sequence| PresentationId {
        generation: 1,
        sequence,
    };
    // Update and Resize first: all three kinds are pending independently, rather than
    // having merged into a tail Show before the Hide arrives.
    let update = q
        .submit(PresentationCommand {
            id: id(1),
            kind: PresentationKind::Update {
                model: PopupViewModel::loading(AIOperation::Translate, "fixture"),
            },
        })
        .unwrap();
    let resize = q
        .submit(PresentationCommand {
            id: id(2),
            kind: PresentationKind::Resize {
                origin: Point { x: 0, y: 0 },
                size_logical: (400., 200.),
            },
        })
        .unwrap();
    let shown = q.submit(show(id(3))).unwrap();
    let mut other = Vec::new();
    for sequence in 4..=32 {
        other.push(
            q.submit(show(PresentationId {
                generation: 2,
                sequence,
            }))
            .unwrap(),
        );
    }
    assert_eq!(
        q.submit(show(id(33))).unwrap_err(),
        PresentationQueueError::Full
    );
    let hide = q
        .submit(PresentationCommand {
            id: id(34),
            kind: PresentationKind::Hide,
        })
        .unwrap();
    q.complete_superseded(&|_| {});
    // These completions are from Hide cancellation, before close is ever called.
    for rx in [shown, update, resize] {
        assert_eq!(
            rx.await.unwrap().result,
            Ok(PresentationVisibility::Superseded)
        );
    }
    let mut native_hides = 0;
    while let Some(job) = q.try_next() {
        if matches!(job.command.kind, PresentationKind::Hide) {
            native_hides += 1;
        }
        let ack = PresentationAck {
            id: job.command.id,
            result: Ok(PresentationVisibility::Hidden),
            effect_left_visible: false,
        };
        job.complete(ack, &|_| {});
    }
    assert_eq!(native_hides, 1);
    assert_eq!(
        hide.await.unwrap().result,
        Ok(PresentationVisibility::Hidden)
    );
    q.close(&|_| {});
    assert_eq!(
        q.submit(show(id(35))).unwrap_err(),
        PresentationQueueError::Closed
    );
}

#[tokio::test]
async fn pending_hides_coalesce_to_one_native_hide_and_resolve_both() {
    let q = PresentationQueue::default();
    let id = |sequence| PresentationId {
        generation: 1,
        sequence,
    };
    let first = q
        .submit(PresentationCommand {
            id: id(1),
            kind: PresentationKind::Hide,
        })
        .unwrap();
    let second = q
        .submit(PresentationCommand {
            id: id(2),
            kind: PresentationKind::Hide,
        })
        .unwrap();
    q.complete_superseded(&|_| {});
    assert_eq!(
        first.await.unwrap().result,
        Ok(PresentationVisibility::Superseded)
    );
    let job = q.try_next().unwrap();
    assert!(matches!(job.command.kind, PresentationKind::Hide));
    job.complete(
        PresentationAck {
            id: id(2),
            result: Ok(PresentationVisibility::Hidden),
            effect_left_visible: false,
        },
        &|_| {},
    );
    assert!(q.try_next().is_none());
    assert_eq!(
        second.await.unwrap().result,
        Ok(PresentationVisibility::Hidden)
    );
}
#[test]
fn stale_after_effect_is_superseded_and_reentrant() {
    let fence = PresentationFence::default();
    let id = PresentationId {
        generation: 1,
        sequence: 1,
    };
    fence.advance(id);
    let ack = fence
        .apply_if_current(id, || {
            fence.advance(PresentationId {
                generation: 2,
                sequence: 2,
            });
            Ok(PresentationVisibility::Visible)
        })
        .unwrap();
    assert_eq!(ack.result, Ok(PresentationVisibility::Superseded));
}

#[allow(dead_code, unused_imports)]
mod integration {
    include!("presentation_fixture.rs.inc");
}

#[tokio::test]
async fn update_and_resize_merge_show_without_passing_hide() {
    let q = PresentationQueue::default();
    let id = |sequence| PresentationId {
        generation: 1,
        sequence,
    };
    let original = q.submit(show(id(1))).unwrap();
    let newest = PopupViewModel::loading(AIOperation::Rewrite, "newest");
    let update = q
        .submit(PresentationCommand {
            id: id(2),
            kind: PresentationKind::Update {
                model: newest.clone(),
            },
        })
        .unwrap();
    let resize = q
        .submit(PresentationCommand {
            id: id(3),
            kind: PresentationKind::Resize {
                origin: Point { x: 40, y: 50 },
                size_logical: (500., 200.),
            },
        })
        .unwrap();
    q.complete_superseded(&|_| {});
    assert_eq!(
        original.await.unwrap().result,
        Ok(PresentationVisibility::Superseded)
    );
    assert_eq!(
        update.await.unwrap().result,
        Ok(PresentationVisibility::Superseded)
    );
    let job = q.try_next().unwrap();
    assert!(
        matches!(&job.command.kind, PresentationKind::Show { origin, size_logical, model } if *origin == Point { x: 40, y: 50 } && *size_logical == (500., 200.) && model == &newest)
    );
    job.complete(
        PresentationAck {
            id: id(3),
            result: Ok(PresentationVisibility::Visible),
            effect_left_visible: false,
        },
        &|_| {},
    );
    assert_eq!(
        resize.await.unwrap().result,
        Ok(PresentationVisibility::Visible)
    );
    let _hide = q
        .submit(PresentationCommand {
            id: id(4),
            kind: PresentationKind::Hide,
        })
        .unwrap();
    let _show = q.submit(show(id(5))).unwrap();
    assert!(matches!(
        q.try_next().unwrap().command.kind,
        PresentationKind::Hide
    ));
    assert!(matches!(
        q.try_next().unwrap().command.kind,
        PresentationKind::Show { .. }
    ));
}

#[test]
fn failed_worker_notification_rejects_without_accepting() {
    let q = PresentationQueue::default();
    let id = PresentationId {
        generation: 1,
        sequence: 1,
    };
    assert_eq!(
        q.submit_notified(show(id), || false).unwrap_err(),
        PresentationQueueError::Closed
    );
    assert!(
        q.try_next().is_none(),
        "rejected job must never be accepted"
    );
    assert_eq!(
        q.submit(show(id)).unwrap_err(),
        PresentationQueueError::Closed
    );
}
