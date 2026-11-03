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
        let (tx, rx) = crossbeam_channel::bounded(1);
        let done = self.done.clone().lock_owned().await;
        let overlay = self.overlay.clone();
        let task = tauri::async_runtime::spawn_blocking(move || {
            // The worker owns the lock even when its awaiting future is aborted.
            let _done = done;
            overlay.pick(rx)
        });
        let out = task
            .await
            .map_err(|_| PlatformError::Unsupported("overlay worker unavailable"))?;
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
}
