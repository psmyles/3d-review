//! The call into the C bridge, and the ownership protocol around it.
//!
//! Everything that talks to C lives here: the `extern "C"` declarations, the
//! progress trampoline, and [`load_fbx`] itself. Both C allocations are handed to
//! a handle that frees them on drop — [`SceneHandle`] for the geometry,
//! [`ExtrasHandle`] for the source-property capture — so they are released on the
//! success path, on every error path, and on an unwind (invariant 9).
//!
//! ## Panics from a progress sink
//!
//! The sink is the caller's code, and it is called from two places. The C
//! trampoline catches its panics, because unwinding through C is undefined; the
//! Rust-side call in the marshal does not, because there the panic unwinds
//! through ordinary Rust and the handles above free everything on the way out.
//! One policy, two mechanisms, because the two sites genuinely differ.

use std::ffi::{CString, c_void};
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::ptr::NonNull;

use crate::{CancelToken, ImportError, PendingExtras, StagedImport};

/// `REVIEW_IMPORT_CANCELLED` from `ufbx_bridge.h`.
const CANCELLED: c_int = -1;

use super::marshal_model::model_from_bridge_scene;
use super::raw::read_error_message;
use super::raw_extras::ReviewImportExtras;
use super::raw_scene::{ReviewImportError, ReviewImportScene};

/// The bridge's geometry scene, owned for as long as anything reads it.
///
/// The free used to be a statement after the marshal. That is correct for every
/// `Result` path — the marshal's error is deliberately deferred past it — but not
/// for an unwind: a progress sink that panics during the Building stage would
/// skip it and strand the whole C-side scene. [`ExtrasHandle`] below already had
/// this shape, as does `review-psd`'s document handle; the scene was the one
/// bridge resource still freed by hand.
struct SceneHandle {
    raw: ReviewImportScene,
}

impl Drop for SceneHandle {
    fn drop(&mut self) {
        // SAFETY: `raw` was filled by a successful `review_import_load_fbx` and
        // is freed exactly once, here. The bridge tolerates the zeroed fields a
        // partial parse may leave, and nothing reads the scene afterwards —
        // every slice taken from it borrows this handle.
        unsafe {
            review_import_free_scene(&mut self.raw);
        }
    }
}

/// The C-side capture, owned until marshaled. It holds only heap buffers
/// the bridge allocated for this call — nothing borrowed from ufbx, whose
/// scene is already freed — so moving it to another thread is sound.
pub(crate) struct ExtrasHandle {
    pub(super) raw: Box<ReviewImportExtras>,
}

// SAFETY: the handle exclusively owns C heap allocations that no other
// thread references (the bridge hands them over and never touches them
// again); the pointers inside are plain data until `review_import_free_extras`.
unsafe impl Send for ExtrasHandle {}

impl Drop for ExtrasHandle {
    fn drop(&mut self) {
        // SAFETY: `raw` was filled by `review_import_load_fbx` and is freed
        // exactly once, here; the bridge's free tolerates a zeroed struct.
        unsafe {
            review_import_free_extras(&mut *self.raw);
        }
    }
}

/// The bridge's progress hook: `user` is a `*const ProgressCallback`. Returns
/// non-zero to carry on, 0 to abandon the parse.
pub(super) type ReviewImportProgressFn = unsafe extern "C" fn(*mut c_void, u64, u64) -> c_int;

/// What the trampoline is handed: the caller's sink, and the token that says
/// whether the load is still wanted. One struct so the bridge keeps a single
/// `void *`.
struct ProgressCallback<'a> {
    sink: crate::ProgressSink<'a>,
    cancel: Option<&'a CancelToken>,
}

unsafe extern "C" {
    fn review_import_load_fbx(
        path: *const c_char,
        out_scene: *mut ReviewImportScene,
        out_extras: *mut ReviewImportExtras,
        out_error: *mut ReviewImportError,
        progress: Option<ReviewImportProgressFn>,
        progress_user: *mut c_void,
    ) -> c_int;

    fn review_import_free_scene(scene: *mut ReviewImportScene);

    fn review_import_free_extras(extras: *mut ReviewImportExtras);
}

/// The trampoline the bridge calls from inside the ufbx parse. `user` is the
/// [`ProgressCallback`] handed to [`load_fbx`], which outlives the whole call.
/// The return value is ufbx's continue/cancel answer.
///
/// A panic here would unwind through C, so the sink is called inside
/// `catch_unwind` and a panicking one is simply ignored — a broken progress
/// indicator must not take the import down with it, and it must not be read as a
/// request to cancel either.
unsafe extern "C" fn report_read_progress(user: *mut c_void, done: u64, total: u64) -> c_int {
    const CONTINUE: c_int = 1;
    const CANCEL: c_int = 0;

    let Some(callback) = NonNull::new(user.cast::<ProgressCallback<'_>>()) else {
        return CONTINUE;
    };
    // SAFETY: `user` is the `&ProgressCallback` `load_fbx` passed to the bridge,
    // which borrows it for no longer than the `review_import_load_fbx` call
    // this callback is made from; the pointer is therefore live and aligned,
    // and nothing else aliases it mutably.
    let callback = unsafe { callback.as_ref() };
    if callback.cancel.is_some_and(|cancel| cancel.is_cancelled()) {
        return CANCEL;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (callback.sink)(crate::ImportProgress {
            stage: crate::ImportStage::Reading,
            done,
            total,
        });
    }));
    CONTINUE
}

pub(crate) fn load_fbx(
    path: &Path,
    progress: crate::ProgressSink<'_>,
    capture_extras: bool,
    cancel: Option<&CancelToken>,
) -> Result<StagedImport, ImportError> {
    let _z = crate::prof::zone!("Load FBX");
    let path_string = path.to_string_lossy();
    let c_path = CString::new(path_string.as_bytes())
        .map_err(|_| ImportError::LoadFailed("path contains embedded NUL byte".to_owned()))?;
    let mut scene = MaybeUninit::<ReviewImportScene>::zeroed();
    let mut error = ReviewImportError { message: [0; 256] };
    // Boxed so the handle that outlives this call never moves the struct
    // the bridge wrote pointers into. Zeroed is a valid "nothing captured"
    // value the free tolerates.
    // SAFETY: `ReviewImportExtras` is all raw pointers and integers, for
    // which the all-zero bit pattern is a valid value.
    let mut extras: Option<Box<ReviewImportExtras>> =
        capture_extras.then(|| Box::new(unsafe { std::mem::zeroed() }));

    let loaded = {
        // The ufbx C parse + the bridge's two-pass extraction (the bulk of a
        // load), measured as one GPU-free CPU zone.
        let _z = crate::prof::zone!("ufbx Parse");
        // SAFETY: `c_path` is a valid NUL-terminated C string that outlives the
        // call; `error` is a live stack value; `scene` is a zeroed,
        // correctly-sized `ReviewImportScene` the bridge fully writes on
        // success (return != 0) or frees + re-zeroes itself on failure (its
        // `cleanup:` path calls `review_import_free_scene`, so no C buffers
        // leak and no free is needed here on the error return below). All
        // three pointers are non-null and valid for the duration of the call.
        // `progress_user` is a pointer to this stack borrow of the caller's
        // sink, which the bridge only dereferences from within this call.
        // `extras` is either null (no capture) or a live boxed struct the
        // bridge fills and this call's handle then owns.
        let mut callback = ProgressCallback {
            sink: progress,
            cancel,
        };
        let extras_ptr = extras
            .as_deref_mut()
            .map_or(std::ptr::null_mut(), |extras| {
                extras as *mut ReviewImportExtras
            });
        unsafe {
            review_import_load_fbx(
                c_path.as_ptr(),
                scene.as_mut_ptr(),
                extras_ptr,
                &mut error,
                Some(report_read_progress),
                (&raw mut callback).cast::<c_void>(),
            )
        }
    };

    // `REVIEW_IMPORT_CANCELLED`: the parse stopped because the trampoline said
    // this load is superseded. Not an error — the caller asked for it, and its
    // generation check would have discarded the result anyway.
    if loaded == CANCELLED {
        return Err(ImportError::Cancelled);
    }
    if loaded == 0 {
        // The bridge freed both outputs on its failure path; the zeroed box
        // is dropped here without a free.
        return Err(ImportError::LoadFailed(read_error_message(&error)));
    }
    // From here the capture is owned by a handle that frees it on drop.
    let extras = extras.map(|raw| ExtrasHandle { raw });

    // SAFETY: `loaded != 0` means the bridge fully initialized `scene`, so the
    // `MaybeUninit` now holds a valid `ReviewImportScene`.
    // From here the scene is owned by a handle that frees it on drop — including
    // on an unwind out of the marshal below, which the old manual free could not
    // cover.
    let scene = SceneHandle {
        raw: unsafe { scene.assume_init() },
    };
    let model = {
        // Walk the flat bridge arrays into our `ModelData` (slices, bounds, BVH).
        let _z = crate::prof::zone!("Build ModelData");
        model_from_bridge_scene(path, &scene.raw, progress)
    };
    // The scene's buffers are freed when `scene` drops at the end of this
    // function — after the marshal has finished reading them, on every path.
    Ok(StagedImport {
        model: model?,
        extras: PendingExtras { handle: extras },
    })
}
