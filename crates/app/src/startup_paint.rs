//! Paint the freshly-created window black before wgpu presents its first frame.
//!
//! winit creates the OS window with a white client area; there's a brief gap
//! between that and the first wgpu present (the egui/wgpu surface setup in
//! `resumed` isn't instant), which flashes white. Filling the client area black
//! the moment the window exists makes that gap black instead — the viewer's own
//! clear color — so startup reads as a single dark surface.
//!
//! **Sanctioned exception to invariant 9.** Invariant 9 funnels all `unsafe`/FFI
//! through `crates/import`. This module is the one explicit exception: the fill
//! has to run against the live winit `Window` at the instant it's created in
//! `app::resumed`, before the slow surface setup. Marshalling the raw `HWND` out
//! to `import` and back purely to call three GDI functions would be more
//! error-prone than keeping the small `unsafe` block beside its only caller. It
//! reads/writes no model or renderer state — it only paints the OS window — so
//! the boundary invariants 9 protects (no `unsafe` leaking into model/render
//! logic) still hold. See CLAUDE.md §"Rust-specific invariants".

/// Fill the window's client area with black using GDI. No-op on non-Windows.
#[cfg(windows)]
pub fn paint_window_black(window: &winit::window::Window) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        BLACK_BRUSH, FillRect, GetDC, GetStockObject, ReleaseDC,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get();

    // SAFETY: `hwnd` is a valid top-level window handle just returned by winit and
    // kept alive by the `&Window` borrow for the duration of this call. `GetDC`
    // hands back a device context owned by that window; we null-check it, query
    // the client rect, fill it with the process-wide stock black brush (a shared
    // object that must never be deleted — so we don't), and release the DC on the
    // same thread before returning. No handle escapes this function.
    unsafe {
        let hdc = GetDC(hwnd);
        if hdc == 0 {
            return;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetClientRect(hwnd, &mut rect) != 0 {
            FillRect(hdc, &rect, GetStockObject(BLACK_BRUSH));
        }
        ReleaseDC(hwnd, hdc);
    }
}

#[cfg(not(windows))]
pub fn paint_window_black(_window: &winit::window::Window) {}
