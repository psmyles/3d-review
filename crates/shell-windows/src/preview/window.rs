//! The preview pane's child window: where Explorer's pane shows the model, and
//! where the pointer drives its camera. Every function here runs on the preview
//! window thread ([`super::ui`]), which owns every such window.
//!
//! The window owns its [`Pane`] — boxed, its pointer in `GWLP_USERDATA` from
//! `WM_NCCREATE` until `WM_NCDESTROY` takes it back — behind a `RefCell`. Several
//! of the calls a message handler makes (`SetCapture`, `ReleaseCapture`,
//! `SetFocus`) send this same window another message before they return, so a
//! handler never makes one while it holds the pane: it borrows, decides, lets
//! go, and only then calls out. A message that arrives while the pane is
//! borrowed anyway goes to `DefWindowProcW`.

#![allow(
    unsafe_code,
    reason = "invariant 9: the preview pane's Win32 window - its class, its procedure and the GDI it paints with"
)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Once;

use glam::Vec2;
use review_preview::{Button, Modifiers};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT,
    DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetStockObject, HBRUSH, HDC, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode,
    SetDIBitsToDevice, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW,
};
use windows::Win32::UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_HOME, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetWindowLongPtrW, HWND_TOP, IDC_ARROW, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, SWP_NOACTIVATE, SWP_NOZORDER, SetParent, SetTimer, SetWindowLongPtrW,
    SetWindowPos, WM_APP, WM_CAPTURECHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WM_TIMER,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOPARENTNOTIFY, WS_VISIBLE,
};
use windows::core::{Error, PCWSTR, Result, w};

use super::pane::{self, Pane};
use crate::guard::guarded;

const CLASS_NAME: PCWSTR = w!("3DReviewPreviewPane");

/// Posted by the load worker when the model is ready (or failed).
const WM_LOADED: u32 = WM_APP + 1;

/// The settle timer: a camera glide or the ambient occlusion converging.
const SETTLE_TIMER: usize = 1;
/// About a display refresh; the timer stops the moment nothing is changing.
const SETTLE_INTERVAL_MS: u32 = 16;

/// One wheel notch, in `WM_MOUSEWHEEL`'s units.
const WHEEL_DELTA: f32 = 120.0;

/// The `F` key, which frames the model as it does in the viewer.
const VK_F: u16 = b'F' as u16;

struct Window {
    pane: RefCell<Pane>,
}

/// Create the pane's window as a child of `parent`, covering `rect`, and start
/// loading `path` into it.
pub(super) fn create(parent: HWND, rect: RECT, path: PathBuf) -> Result<HWND> {
    let instance = module()?;
    register_class(instance)?;
    let window = Box::new(Window {
        pane: RefCell::new(Pane::new()),
    });
    let raw = Box::into_raw(window);
    // A window takes its DPI awareness from its thread's at creation, and a child
    // whose awareness differs from its parent's is placed through the system's
    // mixed-mode scaling: measured with a DPI-unaware host, the pane landed at
    // half the host window's screen position inside it, at 100% scaling. So the
    // window is made in its parent's context, whatever that is, and the thread
    // goes back to its own after.
    // SAFETY: plain queries and a context switch on this thread; `parent` is the
    // host's window handle.
    let previous = unsafe { SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(parent)) };
    // SAFETY: the class is registered on this module; `raw` is a live boxed
    // `Window` the procedure takes ownership of at WM_NCCREATE and frees at
    // WM_NCDESTROY.
    let created = unsafe {
        CreateWindowExW(
            // The parent is Explorer's window, in another process: a parent
            // notification would be a synchronous round trip into it for
            // nothing it uses.
            WS_EX_NOPARENTNOTIFY,
            CLASS_NAME,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(parent),
            None,
            Some(instance),
            Some(raw.cast_const().cast::<c_void>()),
        )
    };
    if !previous.is_invalid() {
        // SAFETY: restoring the context this thread had before the switch above.
        unsafe { SetThreadDpiAwarenessContext(previous) };
    }
    let hwnd = match created {
        Ok(hwnd) => hwnd,
        Err(error) => {
            // A failure after WM_NCCREATE freed the box at WM_NCDESTROY; one
            // before it (the class refused, out of handles) leaks it, which is
            // the price of never risking a double free.
            log::error!("could not create the preview window: {error}");
            return Err(error);
        }
    };
    // The worker carries the handle as a number: an `HWND` is not `Send`, and
    // posting to one that has since been destroyed fails harmlessly.
    let target = hwnd.0 as isize;
    with_pane(hwnd, |pane| {
        pane.resize(client_size(hwnd));
        pane.start_load(path, move || {
            // SAFETY: posting to a window handle is safe whether or not it still
            // exists; a destroyed one makes the call fail, which is ignored.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(target as *mut c_void)),
                    WM_LOADED,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        });
    });
    Ok(hwnd)
}

/// Move the window to `rect` in its parent.
pub(super) fn set_rect(hwnd: HWND, rect: RECT) {
    // SAFETY: plain window-management call on a window this thread created.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// Move the window under a new parent.
pub(super) fn set_parent(hwnd: HWND, parent: HWND) {
    // SAFETY: plain window-management call on a window this thread created.
    let _ = unsafe { SetParent(hwnd, Some(parent)) };
}

pub(super) fn focus(hwnd: HWND) {
    // SAFETY: plain focus call on a window this thread created.
    let _ = unsafe { SetFocus(Some(hwnd)) };
}

pub(super) fn destroy(hwnd: HWND) {
    // SAFETY: plain call on a window this thread created; WM_NCDESTROY frees
    // its pane.
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// This DLL's module handle — the window class is registered on it, not on
/// the host process.
fn module() -> Result<HINSTANCE> {
    let mut module = HMODULE::default();
    // SAFETY: the address is a function in this DLL; with UNCHANGED_REFCOUNT the
    // handle is borrowed and needs no FreeLibrary.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(window_procedure as *const () as *const u16),
            &mut module,
        )
    }?;
    Ok(module.into())
}

/// Register the window class once per process. The DLL is never unloaded
/// (`DllCanUnloadNow`), so the class never outlives its procedure.
fn register_class(instance: HINSTANCE) -> Result<()> {
    static REGISTERED: Once = Once::new();
    let mut result = Ok(());
    REGISTERED.call_once(|| {
        // SAFETY: loading a stock system cursor.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(window_procedure),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // SAFETY: `class` is fully initialized and its strings are static.
        if unsafe { RegisterClassExW(&class) } == 0 {
            result = Err(Error::from_thread());
        }
    });
    result
}

/// Borrow `hwnd`'s pane for `body`, unless it is already borrowed (a message
/// sent from inside another handler) or gone.
fn with_pane<T>(hwnd: HWND, body: impl FnOnce(&mut Pane) -> T) -> Option<T> {
    // SAFETY: reading this window's own userdata slot.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Window;
    if raw.is_null() {
        return None;
    }
    // SAFETY: the slot holds the `Window` boxed at creation, which lives until
    // WM_NCDESTROY clears the slot before freeing it; only a shared reference is
    // made here, and the `RefCell` arbitrates the mutable one.
    let window = unsafe { &*raw };
    let mut pane = window.pane.try_borrow_mut().ok()?;
    Some(body(&mut pane))
}

fn invalidate(hwnd: HWND) {
    // SAFETY: plain call on a window this thread created.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

/// Run the settle timer while the picture is still changing, and stop it when
/// it is not.
fn pace(hwnd: HWND, settling: bool) {
    // SAFETY: plain timer calls on a window this thread created.
    unsafe {
        if settling {
            SetTimer(Some(hwnd), SETTLE_TIMER, SETTLE_INTERVAL_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), SETTLE_TIMER);
        }
    }
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut rect = RECT::default();
    // SAFETY: plain query on a window this thread created.
    let _ = unsafe { GetClientRect(hwnd, &mut rect) };
    (
        (rect.right - rect.left).max(1) as u32,
        (rect.bottom - rect.top).max(1) as u32,
    )
}

/// The pointer position a mouse message carries, in client pixels.
fn pointer(lparam: LPARAM) -> Vec2 {
    let x = (lparam.0 & 0xFFFF) as u16 as i16;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
    Vec2::new(f32::from(x), f32::from(y))
}

fn modifiers() -> Modifiers {
    // SAFETY: reading the calling thread's key state; the high bit is "down".
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    Modifiers {
        shift: held(VK_SHIFT.0),
        alt: held(VK_MENU.0),
    }
}

/// A button went down: the drag starts, and the pane takes the keyboard (for
/// `F`) and the pointer (so a drag that leaves the pane keeps going).
fn press(hwnd: HWND, button: Button, lparam: LPARAM) {
    with_pane(hwnd, |pane| {
        pane.press(button, pointer(lparam), modifiers())
    });
    // SAFETY: plain focus and capture calls on a window this thread created.
    unsafe {
        let _ = SetFocus(Some(hwnd));
        SetCapture(hwnd);
    }
}

fn release(hwnd: HWND, button: Button) {
    let dragging = with_pane(hwnd, |pane| {
        pane.release(button);
        pane.is_dragging()
    });
    if dragging == Some(false) {
        // SAFETY: plain call; releasing a capture we hold.
        let _ = unsafe { ReleaseCapture() };
    }
}

/// Redraw after the pane drew a frame, and keep or stop the settle timer.
fn after_draw(hwnd: HWND, settling: Option<bool>) {
    if let Some(settling) = settling {
        invalidate(hwnd);
        pace(hwnd, settling);
    }
}

extern "system" fn window_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let handled = guarded("preview window", || {
        Ok(handle(hwnd, message, wparam, lparam))
    });
    match handled {
        Ok(Some(result)) => result,
        // SAFETY: the default procedure for a message we did not handle, with
        // the arguments we were given.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Handle `message`, or `None` to pass it to `DefWindowProcW`.
fn handle(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_NCCREATE => {
            // SAFETY: WM_NCCREATE's lparam is the CREATESTRUCTW, whose
            // `lpCreateParams` is the boxed `Window` `create` passed.
            let raw = unsafe { (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams };
            // SAFETY: storing the pointer in this window's own userdata slot.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize) };
            None
        }
        WM_NCDESTROY => {
            // SAFETY: reading then clearing this window's own userdata slot.
            let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Window;
            // SAFETY: as above.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            if !raw.is_null() {
                // SAFETY: the pointer came from `Box::into_raw` in `create`, the
                // slot no longer holds it, and this is the last message the
                // window receives — nothing can reach the box after this.
                drop(unsafe { Box::from_raw(raw) });
            }
            None
        }
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_PAINT => {
            paint(hwnd);
            Some(LRESULT(0))
        }
        WM_SIZE => {
            let size = (
                (lparam.0 & 0xFFFF) as u32,
                ((lparam.0 >> 16) & 0xFFFF) as u32,
            );
            let settling = with_pane(hwnd, |pane| {
                pane.resize(size);
                pane.draw();
                pane.is_settling()
            });
            after_draw(hwnd, settling);
            Some(LRESULT(0))
        }
        WM_LOADED => {
            let settling = with_pane(hwnd, |pane| {
                pane.finish_load().then(|| {
                    pane.resize(client_size(hwnd));
                    pane.draw();
                    pane.is_settling()
                })
            })
            .flatten();
            // A failure has no frame to show, but its status line still paints.
            invalidate(hwnd);
            if let Some(settling) = settling {
                pace(hwnd, settling);
            }
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            let button = match message {
                WM_LBUTTONDOWN => Button::Left,
                WM_RBUTTONDOWN => Button::Right,
                _ => Button::Middle,
            };
            press(hwnd, button, lparam);
            Some(LRESULT(0))
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            let button = match message {
                WM_LBUTTONUP => Button::Left,
                WM_RBUTTONUP => Button::Right,
                _ => Button::Middle,
            };
            release(hwnd, button);
            Some(LRESULT(0))
        }
        WM_CAPTURECHANGED => {
            with_pane(hwnd, Pane::cancel_drag);
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            let settling = with_pane(hwnd, |pane| {
                pane.pointer_moved(pointer(lparam))
                    .then(|| pane.is_settling())
            })
            .flatten();
            after_draw(hwnd, settling);
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            let settling = with_pane(hwnd, |pane| {
                pane.wheel(f32::from(delta) / WHEEL_DELTA);
                pane.is_settling()
            });
            after_draw(hwnd, settling);
            Some(LRESULT(0))
        }
        WM_LBUTTONDBLCLK => {
            let settling = with_pane(hwnd, |pane| {
                pane.reframe();
                pane.is_settling()
            });
            after_draw(hwnd, settling);
            Some(LRESULT(0))
        }
        WM_KEYDOWN if matches!(wparam.0 as u16, VK_F) || wparam.0 as u16 == VK_HOME.0 => {
            let settling = with_pane(hwnd, |pane| {
                pane.reframe();
                pane.is_settling()
            });
            after_draw(hwnd, settling);
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == SETTLE_TIMER => {
            let settling = with_pane(hwnd, |pane| {
                pane.tick();
                pane.is_settling()
            });
            after_draw(hwnd, settling.or(Some(false)));
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// Paint the last frame, or the background and the status line while there is
/// none.
fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: the WM_PAINT pairing of BeginPaint / EndPaint on this window.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    // A pane that is busy (a re-entrant paint) leaves the last picture up.
    with_pane(hwnd, |pane| match pane.frame() {
        Some(frame) => blit(hdc, frame.width, frame.height, &frame.bgra),
        None => {
            let mut rect = RECT::default();
            // SAFETY: plain query on this window.
            let _ = unsafe { GetClientRect(hwnd, &mut rect) };
            fill(hdc, &rect, pane::BACKGROUND);
            if let Some(text) = pane.status_text() {
                write(hdc, &mut rect, &text, pane::TEXT);
            }
        }
    });
    // SAFETY: closes the BeginPaint above.
    let _ = unsafe { EndPaint(hwnd, &paint) };
}

fn blit(hdc: HDC, width: u32, height: u32, bgra: &[u8]) {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `bgra` holds `width × height` 32-bit texels, top row first, as
    // `info` describes; GDI only reads them.
    unsafe {
        SetDIBitsToDevice(
            hdc,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            height,
            bgra.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
    }
}

fn colorref([r, g, b]: [u8; 3]) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

fn fill(hdc: HDC, rect: &RECT, color: [u8; 3]) {
    // SAFETY: a brush created, used and deleted here.
    unsafe {
        let brush: HBRUSH = CreateSolidBrush(colorref(color));
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

fn write(hdc: HDC, rect: &mut RECT, text: &str, color: [u8; 3]) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: GDI text calls on the paint DC; the stock font is not deleted.
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, colorref(color));
        let previous = SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
        DrawTextW(hdc, &mut wide, rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(hdc, previous);
    }
}
