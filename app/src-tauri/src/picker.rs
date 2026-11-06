//! Cancellation-safe bridge to the blocking native overlay.
use async_trait::async_trait;
use lingopop_orchestrator::RegionPicker;
use lingopop_platform::{PlatformError, Rect, RegionOverlay};
use std::sync::Arc;

pub struct OverlayPicker {
    overlay: Arc<dyn RegionOverlay>,
    done: Arc<tokio::sync::Mutex<()>>,
}
impl OverlayPicker {
    pub fn new(overlay: Arc<dyn RegionOverlay>) -> Self {
        Self {
            overlay,
            done: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}
#[async_trait]
impl RegionPicker for OverlayPicker {
    async fn pick(&self) -> Result<Option<Rect>, PlatformError> {
        Ok(self.pick_with_revision().await?.map(|(rect, _)| rect))
    }

    async fn pick_with_revision(&self) -> Result<Option<(Rect, Option<u64>)>, PlatformError> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let done = self.done.clone().lock_owned().await;
        let overlay = self.overlay.clone();
        let task = tauri::async_runtime::spawn_blocking(move || {
            // The worker owns the lock even when its awaiting future is aborted.
            let _done = done;
            overlay.pick_with_revision(rx)
        });
        let out = task.await.map_err(|_| PlatformError::Os {
            api: "overlay worker join",
            code: 0,
        })?;
        drop(tx);
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use lingopop_platform::fake::FakeOverlay;
    use std::time::Duration;
    async fn wait_for(f: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !f() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    struct PanickingOverlay;
    impl RegionOverlay for PanickingOverlay {
        fn pick(&self, _: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError> {
            panic!("test overlay worker panic");
        }
    }
    #[tokio::test]
    async fn worker_panic_is_a_platform_error() {
        let picker = OverlayPicker::new(Arc::new(PanickingOverlay));
        assert_eq!(
            picker.pick().await,
            Err(PlatformError::Os {
                api: "overlay worker join",
                code: 0
            })
        );
    }
    #[tokio::test]
    async fn overlay_os_error_is_preserved() {
        let error = PlatformError::Os {
            api: "overlay thread",
            code: 0,
        };
        let picker = OverlayPicker::new(Arc::new(FakeOverlay::new(Err(error.clone()))));
        assert_eq!(picker.pick().await, Err(error));
    }
    #[tokio::test]
    async fn overlay_pick_returns_none_when_cancel_sender_drops() {
        let overlay = Arc::new(FakeOverlay::new(Ok(None)));
        overlay.block_until_cancel(true);
        let picker = Arc::new(OverlayPicker::new(overlay.clone()));
        let task = tokio::spawn(async move { picker.pick().await });
        wait_for(|| overlay.active() == 1).await;
        task.abort();
        let _ = task.await;
        wait_for(|| overlay.active() == 0).await;
    }
    #[tokio::test]
    async fn rapid_restarts_keep_one_native_pick_active() {
        let overlay = Arc::new(FakeOverlay::new(Ok(None)));
        overlay.block_until_cancel(true);
        let picker = Arc::new(OverlayPicker::new(overlay.clone()));
        for n in 1..=3 {
            let picker = picker.clone();
            let task = tokio::spawn(async move { picker.pick().await });
            wait_for(|| overlay.picks() == n).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            task.abort();
            let _ = task.await;
        }
        wait_for(|| overlay.active() == 0).await;
        assert_eq!(overlay.picks(), 3);
        assert_eq!(overlay.max_active(), 1);
    }
    /// The real Mac lifecycle (`MacOverlay`) over a fake native backend, driven through the
    /// shell bridge: an aborted pick keeps the serial guard until the native cleanup finished.
    #[tokio::test]
    async fn aborted_pick_restart_waits_for_native_cleanup() {
        use lingopop_platform::fake::{FakeMainThread, FakeOverlayBackend, FakeSleeper};
        use lingopop_platform::macos_geometry::{
            MacDisplay, MacGeometry, NativeRect, TopologyCache,
        };
        use lingopop_platform::macos_overlay::MacOverlay;
        let bounds = NativeRect::new(0.0, 0.0, 1440.0, 900.0);
        let cache = TopologyCache::new(
            MacGeometry::new(
                1,
                900.0,
                vec![MacDisplay {
                    id: 1,
                    desktop_bounds: bounds,
                    work_area: bounds,
                    backing_scale: 1.0,
                    primary: true,
                }],
            )
            .unwrap(),
        );
        let main = FakeMainThread::new();
        let backend = FakeOverlayBackend::new();
        let overlay = Arc::new(MacOverlay::new(
            backend.clone(),
            Arc::new(cache),
            Arc::new(FakeSleeper::new()),
        ));
        overlay.bind_main_thread(main.clone());
        let pump = {
            let m = main.clone();
            std::thread::spawn(move || m.pump())
        };
        backend.hold_cleanup(true);
        let picker = Arc::new(OverlayPicker::new(overlay));
        let first = {
            let picker = picker.clone();
            tokio::spawn(async move { picker.pick().await })
        };
        wait_for(|| backend.creates() == 1).await;
        first.abort(); // dropping the future drops the cancel sender
        let _ = first.await;
        backend.wait_for_cleanups_started(1);
        let second = {
            let picker = picker.clone();
            tokio::spawn(async move { picker.pick().await })
        };
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(backend.creates(), 1, "no second overlay during cleanup");
        backend.hold_cleanup(false);
        wait_for(|| backend.creates() == 2).await;
        assert_eq!(backend.max_native(), 1);
        assert!(backend.cleanup_seq(1).unwrap() < backend.create_seq(2).unwrap());
        second.abort();
        let _ = second.await;
        wait_for(|| backend.native_active() == 0).await;
        main.close();
        pump.join().unwrap();
    }
}
