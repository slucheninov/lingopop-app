//! A capture that returns a solid colour and records the regions asked for.

use std::sync::{Arc, Mutex};

use crate::capture::{CapturedImage, ScreenCapture};
use crate::error::PlatformError;
use crate::monitors::Rect;
use crate::permissions::{PermissionKind, PermissionState};

use super::permissions::FakePermissions;

pub struct FakeCapture {
    colour: [u8; 4],
    regions: Mutex<Vec<Rect>>,
    permissions: Option<Arc<FakePermissions>>,
    failure: Mutex<Option<PlatformError>>,
}

impl FakeCapture {
    pub fn new(colour: [u8; 4]) -> Self {
        Self {
            colour,
            regions: Mutex::new(Vec::new()),
            permissions: None,
            failure: Mutex::new(None),
        }
    }

    /// Like macOS: while Screen Recording is not `Granted`, every capture fails with
    /// `PermissionDenied(ScreenRecording)` (the region is still recorded).
    pub fn with_permissions(mut self, permissions: Arc<FakePermissions>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// The next `capture_region` (after recording its region and the permission check) fails
    /// with `error`, for example `Os { api, code }` like a failed native capture.
    pub fn fail_next(&self, error: PlatformError) {
        *self.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
    }

    /// Every region passed to `capture_region`, in order (invalid ones included).
    pub fn regions(&self) -> Vec<Rect> {
        self.regions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl ScreenCapture for FakeCapture {
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError> {
        self.regions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(region);
        if let Some(p) = &self.permissions {
            if p.peek(PermissionKind::ScreenRecording) != PermissionState::Granted {
                return Err(PlatformError::PermissionDenied(
                    PermissionKind::ScreenRecording,
                ));
            }
        }
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            return Err(error);
        }
        if region.is_empty() {
            return Err(PlatformError::InvalidRegion);
        }
        let pixels = region.width as usize * region.height as usize;
        let rgba = self.colour.repeat(pixels);
        CapturedImage::new(region.width as u32, region.height as u32, rgba)
    }
}
