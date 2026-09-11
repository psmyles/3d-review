//! The call into the C bridge, and the ownership protocol around it.
//!
//! Everything that talks to C lives here: the `extern "C"` declarations, the
//! progress trampoline, and [`load_fbx`] itself. The scene is freed on **both**
//! the success and the error path (invariant 9), and the capture is handed to an
//! [`ExtrasHandle`] that frees it on drop.

use std::ffi::{CString, c_void};
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::ptr::NonNull;

use crate::{ImportError, PendingExtras, StagedImport};

use super::marshal_model::model_from_bridge_scene;
use super::raw::read_error_message;
use super::raw_extras::ReviewImportExtras;
use super::raw_scene::{ReviewImportError, ReviewImportScene};

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

/// The bridge's progress hook: `user` is a `*const ProgressSink`.
pub(super) type ReviewImportProgressFn = unsafe extern "C" fn(*mut c_void, u64, u64);

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
/// `&ProgressSink` handed to [`load_fbx`], which outlives the whole call.
///
/// A panic here would unwind through C, so the sink is called inside
/// `catch_unwind` and a panicking one is simply ignored — a broken progress
/// indicator must not take the import down with it.
unsafe extern "C" fn report_read_progress(user: *mut c_void, done: u64, total: u64) {
    let Some(sink) = NonNull::new(user.cast::<crate::ProgressSink<'_>>()) else {
        return;
    };
    // SAFETY: `user` is the `&ProgressSink` `load_fbx` passed to the bridge,
    // which borrows it for no longer than the `review_import_load_fbx` call
    // this callback is made from; the pointer is therefore live and aligned,
    // and nothing else aliases it mutably.
    let sink = unsafe { sink.as_ref() };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sink(crate::ImportProgress {
            stage: crate::ImportStage::Reading,
            done,
            total,
        });
    }));
}

pub(crate) fn load_fbx(
    path: &Path,
    progress: crate::ProgressSink<'_>,
    capture_extras: bool,
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
        let mut sink = progress;
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
                (&raw mut sink).cast::<c_void>(),
            )
        }
    };

    if loaded == 0 {
        // The bridge freed both outputs on its failure path; the zeroed box
        // is dropped here without a free.
        return Err(ImportError::LoadFailed(read_error_message(&error)));
    }
    // From here the capture is owned by a handle that frees it on drop.
    let extras = extras.map(|raw| ExtrasHandle { raw });

    // SAFETY: `loaded != 0` means the bridge fully initialized `scene`, so the
    // `MaybeUninit` now holds a valid `ReviewImportScene`.
    let mut scene = unsafe { scene.assume_init() };
    let model = {
        // Walk the flat bridge arrays into our `ModelData` (slices, bounds, BVH).
        let _z = crate::prof::zone!("Build ModelData");
        model_from_bridge_scene(path, &scene, progress)
    };
    // SAFETY: `scene` is the bridge-allocated scene we own; this frees its C-side
    // buffers exactly once, on both the success and error paths of the extraction
    // above (we still return `model` afterwards). The bridge tolerates the zeroed
    // fields a partial parse may leave. No further access to `scene` follows.
    unsafe {
        review_import_free_scene(&mut scene);
    }
    Ok(StagedImport {
        model: model?,
        extras: PendingExtras { handle: extras },
    })
}
