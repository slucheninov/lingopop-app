//! `Monitors` over `GetCursorPos` / `EnumDisplayMonitors`, always in physical pixels.

use std::mem::size_of;

use ::windows::core::BOOL;
use ::windows::Win32::Foundation::{LPARAM, POINT, RECT};
use ::windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use ::windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::util::{last_error, map_err, os_err, DpiGuard};
use crate::error::PlatformError;
use crate::monitors::{MonitorInfo, Monitors, Point, Rect};

/// `MONITORINFOF_PRIMARY`.
const PRIMARY: u32 = 1;

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsMonitors;

impl WindowsMonitors {
    pub fn new() -> Self {
        Self
    }
}

fn rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// Collects monitors during `EnumDisplayMonitors`; the first failure stops the enumeration.
#[derive(Default)]
struct Collected {
    monitors: Vec<MonitorInfo>,
    error: Option<PlatformError>,
}

fn describe(handle: HMONITOR) -> Result<MonitorInfo, PlatformError> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with cbSize set; `handle` comes from the enumeration.
    if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        return Err(os_err("GetMonitorInfoW", last_error()));
    }
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: both out-pointers are valid for the call.
    if unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        dpi_x = 96;
    }
    Ok(MonitorInfo {
        bounds: rect(info.rcMonitor),
        work_area: rect(info.rcWork),
        scale_factor: f64::from(dpi_x) / 96.0,
        primary: info.dwFlags & PRIMARY != 0,
    })
}

unsafe extern "system" fn collect(
    handle: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `&mut Collected` passed by `enumerate`, alive for the whole enumeration.
    let out = unsafe { &mut *(data.0 as *mut Collected) };
    match describe(handle) {
        Ok(m) => {
            out.monitors.push(m);
            BOOL(1)
        }
        Err(e) => {
            out.error = Some(e);
            BOOL(0)
        }
    }
}

pub(crate) fn enumerate() -> Result<Vec<MonitorInfo>, PlatformError> {
    let _dpi = DpiGuard::per_monitor_v2();
    let mut out = Collected::default();
    // SAFETY: the callback only runs during this call and receives a pointer to `out`, which
    // outlives it; a null DC and clip enumerate every monitor of the virtual screen.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut out as *mut Collected as isize),
        )
    };
    if let Some(e) = out.error {
        return Err(e);
    }
    if !ok.as_bool() {
        return Err(os_err("EnumDisplayMonitors", last_error()));
    }
    Ok(out.monitors)
}

impl Monitors for WindowsMonitors {
    fn cursor_position(&self) -> Result<Point, PlatformError> {
        let _dpi = DpiGuard::per_monitor_v2();
        let mut p = POINT::default();
        // SAFETY: `p` is a valid out-pointer.
        unsafe { GetCursorPos(&mut p) }.map_err(map_err("GetCursorPos"))?;
        Ok(Point { x: p.x, y: p.y })
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        enumerate()
    }
}
