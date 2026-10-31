//! `ScreenCapture` over GDI (`BitBlt` from the screen DC into a DIB), in physical pixels.

use std::mem::size_of;

use ::windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ, ROP_CODE, SRCCOPY,
};

use super::monitors::enumerate;
use super::util::{last_error, os_err, DpiGuard};
use crate::capture::{CapturedImage, ScreenCapture};
use crate::error::PlatformError;
use crate::monitors::{validate_region, Rect};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsCapture;

impl WindowsCapture {
    pub fn new() -> Self {
        Self
    }
}

/// The screen DC, released on drop.
struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from GetDC(None) and is released exactly once.
        let _ = unsafe { ReleaseDC(None, self.0) };
    }
}

/// A memory DC, deleted on drop.
struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from CreateCompatibleDC and is deleted exactly once.
        let _ = unsafe { DeleteDC(self.0) };
    }
}

/// A GDI bitmap, deleted on drop (it must not be selected into a DC by then).
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: the bitmap came from CreateCompatibleBitmap and is deleted exactly once.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0 .0)) };
    }
}

/// Keeps a bitmap selected into a DC and reselects the previous object on drop.
struct Selected {
    dc: HDC,
    previous: HGDIOBJ,
}

impl Drop for Selected {
    fn drop(&mut self) {
        // SAFETY: `previous` is what SelectObject returned for this DC, which is still alive
        // (this guard is declared after, so dropped before, the DC and the bitmap).
        let _ = unsafe { SelectObject(self.dc, self.previous) };
    }
}

fn capture(region: Rect) -> Result<CapturedImage, PlatformError> {
    let (w, h) = (region.width, region.height);
    let pixels = (w as usize)
        .checked_mul(h as usize)
        .ok_or(PlatformError::InvalidRegion)?;
    let mut bgra = vec![0u8; pixels.checked_mul(4).ok_or(PlatformError::InvalidRegion)?];

    // SAFETY: a null window means the whole screen; released by `ScreenDc`.
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return Err(os_err("GetDC", last_error()));
    }
    let screen = ScreenDc(screen);
    // SAFETY: `screen.0` is a live DC.
    let mem = unsafe { CreateCompatibleDC(Some(screen.0)) };
    if mem.is_invalid() {
        return Err(os_err("CreateCompatibleDC", last_error()));
    }
    let mem = MemoryDc(mem);
    // SAFETY: `screen.0` is a live DC; w and h are positive.
    let bitmap = unsafe { CreateCompatibleBitmap(screen.0, w, h) };
    if bitmap.is_invalid() {
        return Err(os_err("CreateCompatibleBitmap", last_error()));
    }
    let bitmap = Bitmap(bitmap);

    // SAFETY: both handles are live; the bitmap is deselected again by `Selected`.
    let previous = unsafe { SelectObject(mem.0, HGDIOBJ(bitmap.0 .0)) };
    if previous.is_invalid() {
        return Err(os_err("SelectObject", last_error()));
    }
    let selected = Selected {
        dc: mem.0,
        previous,
    };
    // SAFETY: the destination is the memory DC with a w x h bitmap selected, the source is the
    // screen DC; coordinates are virtual-screen pixels (negative values are valid).
    unsafe {
        BitBlt(
            mem.0,
            0,
            0,
            w,
            h,
            Some(screen.0),
            region.x,
            region.y,
            ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
        )
    }
    .map_err(|e| os_err("BitBlt", super::util::win32_code(&e)))?;
    // GetDIBits requires the bitmap not to be selected into a DC.
    drop(selected);

    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `bgra` holds w * h * 4 writable bytes, matching the requested top-down 32-bit
    // format with h scan lines; `info` is a valid BITMAPINFO.
    let lines = unsafe {
        GetDIBits(
            mem.0,
            bitmap.0,
            0,
            h as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    if lines != h {
        return Err(os_err("GetDIBits", last_error()));
    }

    for i in (0..bgra.len()).step_by(4) {
        bgra.swap(i, i + 2);
        bgra[i + 3] = 255; // GDI leaves the alpha byte 0
    }
    CapturedImage::new(w as u32, h as u32, bgra)
}

impl ScreenCapture for WindowsCapture {
    fn capture_region(&self, region: Rect) -> Result<CapturedImage, PlatformError> {
        let _dpi = DpiGuard::per_monitor_v2();
        let clipped = validate_region(region, &enumerate()?)?;
        capture(clipped)
    }
}
