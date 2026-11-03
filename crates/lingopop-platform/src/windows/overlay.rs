//! One physical virtual-screen layered window; all input comes from thread-owned LL hooks.
use super::{
    monitors::enumerate,
    util::{last_error, map_err, os_err, DpiGuard},
};
use crate::{
    monitor_at, overlay::*, virtual_screen, MonitorInfo, PlatformError, Point, Rect, RegionOverlay,
    Size,
};
use ::windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::{Dwm::DwmFlush, Gdi::*},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{Input::KeyboardAndMouse::VK_ESCAPE, WindowsAndMessaging::*},
    },
};
use std::{cell::RefCell, sync::Mutex, time::Duration};

#[derive(Default)]
pub struct WindowsOverlay {
    serial: Mutex<()>,
}
impl WindowsOverlay {
    pub fn new() -> Self {
        Self::default()
    }
}
impl RegionOverlay for WindowsOverlay {
    fn pick(&self, cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError> {
        let _serial = self.serial.lock().unwrap_or_else(|e| e.into_inner());
        std::thread::Builder::new()
            .name("lingopop-overlay".into())
            .spawn(move || {
                let _dpi = DpiGuard::per_monitor_v2();
                let _flush = FlushAfterCleanup;
                run(cancel)
            })
            .map_err(|e| PlatformError::Os {
                api: "overlay thread spawn",
                code: e.raw_os_error().unwrap_or(0),
            })?
            .join()
            .map_err(|_| PlatformError::Os {
                api: "overlay thread panicked",
                code: 0,
            })?
    }
}
// Declared outside run so native guards unwind before this one, including on panic.
struct FlushAfterCleanup;
impl Drop for FlushAfterCleanup {
    fn drop(&mut self) {
        // SAFETY: no pointers or thread-affinity restrictions.
        let _ = unsafe { DwmFlush() };
        std::thread::sleep(Duration::from_millis(50));
    }
}
thread_local! { static INPUT: RefCell<Vec<OverlayInput>> = const { RefCell::new(Vec::new()) }; }
fn queue_input(event: OverlayInput) {
    INPUT.with(|q| q.borrow_mut().push(event));
    // SAFETY: LL hooks execute on the installing thread, whose message queue already exists.
    // Wake GetMessageW even when the hook swallows the native input; timer remains a fallback.
    let _ = unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_APP, WPARAM(0), LPARAM(0)) };
}
unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: Windows provides this structure for nonnegative low-level hook codes.
        let m = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
        let p = Point {
            x: m.pt.x,
            y: m.pt.y,
        };
        let event = match w.0 as u32 {
            WM_LBUTTONDOWN => Some(OverlayInput::Down(p)),
            WM_MOUSEMOVE => {
                // SAFETY: standard system cursor, no owned resource.
                unsafe {
                    SetCursor(LoadCursorW(None, IDC_CROSS).ok());
                }
                Some(OverlayInput::Move(p))
            }
            WM_LBUTTONUP => Some(OverlayInput::Up(p)),
            _ => None,
        };
        if let Some(event) = event {
            queue_input(event);
            // Mouse moves must pass through so relative mice can move the system cursor.
            if w.0 as u32 != WM_MOUSEMOVE {
                return LRESULT(1);
            }
        }
    }
    // SAFETY: forwards unchanged hook parameters.
    unsafe { CallNextHookEx(None, code, w, l) }
}
/// Escape cancels and is swallowed even with modifiers (including Ctrl+Esc and Alt+Esc).
unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: Windows provides a valid keyboard record for nonnegative codes.
        let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if k.vkCode == u32::from(VK_ESCAPE.0) {
            if matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
                queue_input(OverlayInput::Escape);
            }
            return LRESULT(1);
        }
    }
    // SAFETY: forwards unchanged hook parameters.
    unsafe { CallNextHookEx(None, code, w, l) }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_SETCURSOR {
        // SAFETY: uses a shared standard cursor.
        unsafe {
            SetCursor(LoadCursorW(None, IDC_CROSS).ok());
        }
        return LRESULT(1);
    }
    // SAFETY: forwards the window procedure's arguments to the default handler.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
struct Hook(HHOOK);
impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: uniquely owned live hook, removed on installing thread.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: timer and window belong to this thread and this guard.
        unsafe {
            let _ = KillTimer(Some(self.0), 1);
            let _ = DestroyWindow(self.0);
        }
    }
}
struct SelectedFont {
    dc: HDC,
    font: HFONT,
    previous: HGDIOBJ,
}
impl Drop for SelectedFont {
    fn drop(&mut self) {
        // SAFETY: restore the DC's prior object before deleting the uniquely owned font.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.font.into());
        }
    }
}
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    bounds: Rect,
}
impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: restore original selection before freeing bitmap and DC, each uniquely owned.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
impl Surface {
    fn new(bounds: Rect) -> Result<Self, PlatformError> {
        // SAFETY: compatible with the screen, freed on all paths below.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return Err(os_err("CreateCompatibleDC", last_error()));
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bounds.width,
                biHeight: -bounds.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: valid BITMAPINFO and writable pointer; top-down 32-bit DIB.
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: dc is not yet owned by a Surface.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err(map_err("CreateDIBSection")(e));
            }
        };
        // SAFETY: valid bitmap and DC; previous object restored on drop.
        let old = unsafe { SelectObject(dc, bitmap.into()) };
        let mut out = Self {
            dc,
            bitmap,
            old,
            bits: bits.cast(),
            bounds,
        };
        out.fill(bounds, (DIM_ALPHA as u32) << 24);
        Ok(out)
    }
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: DIB owns exactly width * height writable u32 pixels and outlives this slice.
        unsafe {
            std::slice::from_raw_parts_mut(
                self.bits,
                self.bounds.width as usize * self.bounds.height as usize,
            )
        }
    }
    fn fill(&mut self, r: Rect, color: u32) {
        let b = self.bounds;
        if let Some(r) = r.intersect(b) {
            let pixels = self.pixels();
            for y in r.y..r.bottom() {
                let i = ((y - b.y) as usize * b.width as usize) + (r.x - b.x) as usize;
                pixels[i..i + r.width as usize].fill(color);
            }
        }
    }
    fn extent(&self, r: Rect, scale: f64) -> Rect {
        // Includes labels even for a selection smaller than the label.
        let pad = (200.0 * scale).ceil() as i32;
        Rect::new(r.x - pad, r.y - pad, r.width + 2 * pad, r.height + 2 * pad)
    }
    fn render(&mut self, old: Option<Rect>, r: Rect, scale: f64) {
        let now = self.extent(r, scale);
        let dirty = if let Some(old) = old {
            let x = now.x.min(old.x);
            let y = now.y.min(old.y);
            Rect::new(
                x,
                y,
                now.right().max(old.right()) - x,
                now.bottom().max(old.bottom()) - y,
            )
        } else {
            now
        };
        self.fill(dirty, (DIM_ALPHA as u32) << 24);
        if r.is_empty() {
            return;
        }
        self.fill(r, 1 << 24); // Alpha 1 keeps the hole hit-testable.
        let n = border_px(scale);
        self.fill(Rect::new(r.x, r.y, r.width, n), 0xffffffff);
        self.fill(Rect::new(r.x, r.bottom() - n, r.width, n), 0xffffffff);
        self.fill(Rect::new(r.x, r.y, n, r.height), 0xffffffff);
        self.fill(Rect::new(r.right() - n, r.y, n, r.height), 0xffffffff);
        self.label(r, scale);
    }
    fn label(&mut self, r: Rect, scale: f64) {
        let mut text: Vec<u16> = size_label(r).encode_utf16().collect();
        // SAFETY: system font creation; owned handle is released below after deselection.
        let font = unsafe {
            CreateFontW(
                -(LABEL_PT * scale).ceil() as i32,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        if font.is_invalid() {
            return;
        }
        // SAFETY: DC and font are live; all GDI operations stay on this thread.
        unsafe {
            let _font = SelectedFont {
                dc: self.dc,
                font,
                previous: SelectObject(self.dc, font.into()),
            };
            let mut measured = RECT::default();
            DrawTextW(
                self.dc,
                &mut text,
                &mut measured,
                DT_CALCRECT | DT_SINGLELINE,
            );
            let size = Size {
                width: measured.right + 1,
                height: measured.bottom + 1,
            };
            let p = label_origin(r, size, self.bounds);
            let box_r = Rect::new(p.x, p.y, size.width, size.height);
            if let Some(box_r) = box_r.intersect(self.bounds) {
                // Render a grayscale glyph mask, then composite shadow/white into premultiplied pixels.
                let b = self.bounds;
                let mut saved = Vec::new();
                for y in box_r.y..box_r.bottom() {
                    for x in box_r.x..box_r.right() {
                        let i = ((y - b.y) * b.width + x - b.x) as usize;
                        saved.push((i, self.pixels()[i]));
                    }
                }
                self.fill(box_r, 0);
                SetBkMode(self.dc, TRANSPARENT);
                SetTextColor(self.dc, COLORREF(0xffffff));
                let mut dest = RECT {
                    left: p.x - b.x,
                    top: p.y - b.y,
                    right: p.x - b.x + size.width,
                    bottom: p.y - b.y + size.height,
                };
                DrawTextW(self.dc, &mut text, &mut dest, DT_SINGLELINE | DT_NOPREFIX);
                let _ = GdiFlush();
                let mask: Vec<_> = saved
                    .iter()
                    .map(|(i, _)| (*i, (self.pixels()[*i] & 255) as u8))
                    .collect();
                for (i, v) in saved {
                    self.pixels()[i] = v;
                }
                for shadow in [true, false] {
                    for &(i, a) in &mask {
                        if a == 0 {
                            continue;
                        }
                        let j = if shadow { i + b.width as usize + 1 } else { i };
                        if j >= self.pixels().len()
                            || (shadow && i % b.width as usize == b.width as usize - 1)
                        {
                            continue;
                        }
                        let bg = self.pixels()[j];
                        let a = a as u32;
                        let inv = 255 - a;
                        let mut v = (a + ((bg >> 24) * inv / 255)) << 24;
                        for shift in [0, 8, 16] {
                            let channel = if shadow { 0 } else { a };
                            v |= (channel + ((bg >> shift) & 255) * inv / 255) << shift;
                        }
                        self.pixels()[j] = v;
                    }
                }
            }
        }
    }
    fn push(&self, hwnd: HWND, initial: bool) -> Result<(), PlatformError> {
        let dst = POINT {
            x: self.bounds.x,
            y: self.bounds.y,
        };
        let size = SIZE {
            cx: self.bounds.width,
            cy: self.bounds.height,
        };
        let src = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: live window/DC, valid source DIB and stack parameters for this synchronous call.
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                if initial { Some(&dst) } else { None },
                Some(&size),
                Some(self.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .map_err(map_err("UpdateLayeredWindow"))
    }
}
fn run(cancel: crossbeam_channel::Receiver<()>) -> Result<Option<Rect>, PlatformError> {
    let monitors: Vec<MonitorInfo> = enumerate()?;
    let bounds = virtual_screen(&monitors);
    if bounds.is_empty() {
        return Err(PlatformError::InvalidRegion);
    }
    INPUT.with(|q| q.borrow_mut().clear());
    // SAFETY: querying the executable module; window class and callbacks have static lifetimes.
    let module =
        unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(map_err("GetModuleHandleW"))?;
    let instance = HINSTANCE::from(module);
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: w!("LingoPopRegionOverlay"),
        ..Default::default()
    };
    // SAFETY: registration uses static class name and procedure; already registered is harmless.
    unsafe {
        RegisterClassW(&class);
    }
    // SAFETY: all pointers valid; one unowned, nonactivating window covering the full virtual screen.
    let window = Window(
        unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("LingoPopRegionOverlay"),
                w!(""),
                WS_POPUP,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                None,
                None,
                Some(instance),
                None,
            )
        }
        .map_err(map_err("CreateWindowExW"))?,
    );
    let mut surface = Surface::new(bounds)?;
    surface.push(window.0, true)?;
    // SAFETY: hooks installed globally on this pumping thread, guarded on every exit.
    let _mouse = Hook(
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(instance), 0) }
            .map_err(map_err("SetWindowsHookExW"))?,
    );
    // SAFETY: same ownership and thread rules as mouse hook.
    let _keyboard = Hook(
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(instance), 0) }
            .map_err(map_err("SetWindowsHookExW"))?,
    );
    // SAFETY: live window and thread-owned timer; Window destroys both.
    unsafe {
        if SetTimer(Some(window.0), 1, 50, None) == 0 {
            return Err(os_err("SetTimer", last_error()));
        }
        let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
        SetCursor(LoadCursorW(None, IDC_CROSS).ok());
    }
    let mut start = None;
    let mut previous = None;
    let mut rendered = None;
    loop {
        if !matches!(
            cancel.try_recv(),
            Err(crossbeam_channel::TryRecvError::Empty)
        ) {
            return Ok(None);
        }
        let mut msg = MSG::default();
        // SAFETY: valid message buffer; this thread owns window and hooks and pumps them until cleanup.
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
        if result < 0 {
            return Err(os_err("GetMessageW", last_error()));
        }
        if result == 0 {
            return Ok(None);
        }
        // Only a THREAD message (no window) with WM_APP is our wake-up; drain INPUT below without
        // dispatch. A WM_APP addressed to a window is dispatched like any other message.
        if !(msg.message == WM_APP && msg.hwnd.0.is_null()) {
            // SAFETY: dispatches this thread's retrieved message.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let events = INPUT.with(|q| std::mem::take(&mut *q.borrow_mut()));
        match fold_input_batch(&mut start, rendered, events) {
            InputBatch::Finish(result) => return Ok(result),
            InputBatch::Unchanged => {}
            InputBatch::Redraw(r) => {
                let scale = monitor_at(
                    Point {
                        x: r.right(),
                        y: r.bottom(),
                    },
                    &monitors,
                )
                .map_or(1.0, |m| m.scale_factor);
                surface.render(previous, r, scale);
                surface.push(window.0, false)?;
                previous = Some(surface.extent(r, scale));
                rendered = Some(r);
            }
        }
    }
}
