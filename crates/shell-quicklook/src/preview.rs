//! The interactive preview: a [`Session`] behind an opaque handle, driven by the
//! extension's view from its main thread.
//!
//! The view owns everything AppKit: the layer it shows frames in, the settle
//! timer, and the conversion of mouse, trackpad and key events into the pixel
//! positions and buttons these functions take.

#![allow(
    unsafe_code,
    reason = "invariant 9: the preview handle and the pointers the extension passes in"
)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;

use glam::Vec2;
use review_preview::{Button, Modifiers, Session, Status};

use crate::guard::guarded;
use crate::image::ReviewQlImage;

/// What the model is laid over (sRGB): the viewer's own default backdrop
/// (`ViewportBackground::Black`), so the preview shows a model as the viewer
/// would, in either appearance — the Explorer pane's choice too.
const BACKGROUND: [u8; 3] = [0, 0, 0];

/// The header's `ReviewQlPreview`.
pub struct ReviewQlPreview {
    session: Session,
    /// The last status text handed out, kept alive until the next call.
    status: Option<CString>,
}

/// The header's `ReviewQlLoaded`.
type Loaded = unsafe extern "C" fn(*mut c_void);

/// The extension's callback (if it gave one) and its context, carried to the
/// loading worker.
struct Notify {
    loaded: Option<Loaded>,
    context: *mut c_void,
}

// SAFETY: the header documents that `loaded` is called on a worker thread, so
// the extension hands over a callback and a context that may be used from one;
// this type only carries them there.
unsafe impl Send for Notify {}

impl Notify {
    fn call(self) {
        if let Some(loaded) = self.loaded {
            // SAFETY: the extension's own callback with its own context, called
            // the one time the header promises.
            unsafe { loaded(self.context) };
        }
    }
}

/// Run `body` on the preview behind `preview`, or answer `fallback` for a null
/// handle or a panic.
///
/// # Safety
///
/// `preview` is null or a live handle from [`review_ql_preview_open`], used by
/// no other thread during the call.
unsafe fn with_preview<T>(
    preview: *mut ReviewQlPreview,
    what: &str,
    fallback: T,
    body: impl FnOnce(&mut ReviewQlPreview) -> T,
) -> T {
    // SAFETY: the caller's contract.
    match unsafe { preview.as_mut() } {
        Some(preview) => guarded(what, fallback, || body(preview)),
        None => fallback,
    }
}

/// Start loading `path`. See the header.
///
/// # Safety
///
/// `path` is null or a NUL-terminated string; `loaded` may be called with
/// `context` from another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_open(
    path: *const c_char,
    loaded: Option<Loaded>,
    context: *mut c_void,
) -> *mut ReviewQlPreview {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: non-null, and NUL-terminated by the caller's contract.
    let path = PathBuf::from(
        unsafe { CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    );
    let notify = Notify { loaded, context };
    let opened = guarded("preview open", None, move || {
        let mut session = Session::new(BACKGROUND);
        session.start_load(path, move || notify.call());
        Some(Box::new(ReviewQlPreview {
            session,
            status: None,
        }))
    });
    opened.map_or(std::ptr::null_mut(), Box::into_raw)
}

/// Release the preview. See the header.
///
/// # Safety
///
/// `preview` is null or a live handle from [`review_ql_preview_open`], not used
/// again afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_close(preview: *mut ReviewQlPreview) {
    if preview.is_null() {
        return;
    }
    // SAFETY: a handle `review_ql_preview_open` made with `Box::into_raw`, and the
    // caller's contract says this is its last use.
    let preview = unsafe { Box::from_raw(preview) };
    guarded("preview close", (), move || drop(preview));
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_finish_load(preview: *mut ReviewQlPreview) -> bool {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview finish_load", false, |preview| {
            preview.session.finish_load()
        })
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_status_text(
    preview: *mut ReviewQlPreview,
) -> *const c_char {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview status", std::ptr::null(), |preview| {
            let key = match preview.session.status() {
                Status::Loading => crate::keys::shell_preview::LOADING,
                Status::Failed => crate::keys::shell_preview::FAILED,
                Status::Ready => {
                    preview.status = None;
                    return std::ptr::null();
                }
            };
            let text = review_localization::tr(key).replace('\0', "");
            let text = preview
                .status
                .insert(CString::new(text).unwrap_or_default());
            text.as_ptr()
        })
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_resize(
    preview: *mut ReviewQlPreview,
    width: u32,
    height: u32,
) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview resize", (), |preview| {
            preview.session.resize((width, height));
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_draw(preview: *mut ReviewQlPreview) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview draw", (), |preview| {
            preview.session.draw()
        })
    }
}

/// # Safety
///
/// `preview` as for [`with_preview`]; `out` is null or points to a writable
/// `ReviewQlImage`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_frame(
    preview: *const ReviewQlPreview,
    out: *mut ReviewQlImage,
) -> bool {
    // SAFETY: the caller's contract for `out`.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return false;
    };
    *out = ReviewQlImage::EMPTY;
    // SAFETY: forwarded contract; the frame is only read.
    let frame = unsafe {
        with_preview(preview.cast_mut(), "preview frame", None, |preview| {
            preview.session.frame().map(ReviewQlImage::borrowed)
        })
    };
    match frame {
        Some(frame) => {
            *out = frame;
            true
        }
        None => false,
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_is_settling(preview: *const ReviewQlPreview) -> bool {
    // SAFETY: forwarded contract; only read.
    unsafe {
        with_preview(
            preview.cast_mut(),
            "preview is_settling",
            false,
            |preview| preview.session.is_settling(),
        )
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_tick(preview: *mut ReviewQlPreview) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview tick", (), |preview| {
            preview.session.tick()
        })
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_is_dragging(preview: *const ReviewQlPreview) -> bool {
    // SAFETY: forwarded contract; only read.
    unsafe {
        with_preview(
            preview.cast_mut(),
            "preview is_dragging",
            false,
            |preview| preview.session.is_dragging(),
        )
    }
}

/// The header's `REVIEW_QL_BUTTON_*`.
fn button(code: u32) -> Option<Button> {
    match code {
        0 => Some(Button::Left),
        1 => Some(Button::Right),
        2 => Some(Button::Middle),
        _ => None,
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_press(
    preview: *mut ReviewQlPreview,
    code: u32,
    x: f32,
    y: f32,
    shift: bool,
    alt: bool,
) {
    let Some(button) = button(code) else {
        return;
    };
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview press", (), |preview| {
            preview
                .session
                .press(button, Vec2::new(x, y), Modifiers { shift, alt });
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_release(preview: *mut ReviewQlPreview, code: u32) {
    let Some(button) = button(code) else {
        return;
    };
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview release", (), |preview| {
            preview.session.release(button);
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_cancel_drag(preview: *mut ReviewQlPreview) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview cancel_drag", (), |preview| {
            preview.session.cancel_drag();
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_pointer_moved(
    preview: *mut ReviewQlPreview,
    x: f32,
    y: f32,
) -> bool {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview pointer_moved", false, |preview| {
            preview.session.pointer_moved(Vec2::new(x, y))
        })
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_scroll_lines(
    preview: *mut ReviewQlPreview,
    notches: f32,
) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview scroll_lines", (), |preview| {
            preview.session.wheel_notches(notches);
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_scroll_pixels(
    preview: *mut ReviewQlPreview,
    pixels: f32,
) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview scroll_pixels", (), |preview| {
            preview.session.wheel_pixels(pixels);
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_pinch(preview: *mut ReviewQlPreview, scale: f32) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview pinch", (), |preview| {
            preview.session.pinch(scale);
        });
    }
}

/// # Safety
///
/// As for [`with_preview`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn review_ql_preview_reframe(preview: *mut ReviewQlPreview) {
    // SAFETY: forwarded contract.
    unsafe {
        with_preview(preview, "preview reframe", (), |preview| {
            preview.session.reframe();
        });
    }
}
