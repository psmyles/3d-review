//! FBX import: the vendored-ufbx bridge behind a small, safe API.
//!
//! [`load_model`] is the whole of it for a batch caller; the viewer uses
//! [`load_model_staged`] so the mesh reaches the screen before the measurements
//! that do not affect it (each clip's motion envelope, the per-draw-group table)
//! have been made, and [`load_model_with_progress`] to report a stage as it goes.
//!
//! Everything that touches C lives in the `#[cfg(has_ufbx)]` `ffi` module, which
//! layers the `unsafe` into two small files and keeps the ~1500 lines of
//! marshalling above them provably safe (invariant 9). Without
//! `third_party/ufbx` the crate still builds and every load reports
//! [`ImportError::UfbxUnavailable`].

// Invariant 9, enforced: `unsafe` is refused crate-wide, and each leaf that
// needs it — the bridge call, the pointer leaves, the Windows launch hint and
// the Tracy allocator — opts in with a module-level `allow` that says why.
#![deny(unsafe_code)]

use std::path::Path;

use review_model::{AnimContext, ModelData, SourceExtras};
use thiserror::Error;

#[cfg(has_ufbx)]
mod ffi;
mod prof;
// The funnel's repair step is ordinary safe Rust over `ModelData`, so it builds
// (and its tests run) without the vendored ufbx; only `ffi` calls it.
#[cfg_attr(not(has_ufbx), allow(dead_code))]
mod repair;
mod startup;
mod tracy_alloc;

pub use startup::startup_show_maximized;
pub use tracy_alloc::TracyAllocator;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("FBX import is unavailable until third_party/ufbx contains ufbx.c and ufbx.h")]
    UfbxUnavailable,
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
    #[error("failed to load model: {0}")]
    LoadFailed(String),
    #[error("the import was cancelled")]
    Cancelled,
}

/// A shared "is this load still wanted?" flag, checked from inside the parse.
///
/// Dropping a superseded result when it arrives — which `app` has always done by
/// generation — keeps the wrong model off screen but recovers none of the work.
/// Parsing a 120 MB FBX is ~1.6 s and the measuring stages behind it another
/// ~3 s, all of it spent producing something nobody will look at, and dropping
/// one model on the viewer after another is an ordinary thing to do.
///
/// The counter is the caller's own request generation: the token is live while
/// it still reads the value the token was made with. That makes cancellation a
/// consequence of the check `app` already performs, rather than a second piece of
/// state to keep in step with it.
pub use review_model::CancelToken;

/// The stage an import is in, as reported to [`load_model_with_progress`].
///
/// These are the five measured phases of a load, in order; the numbers beside
/// them are a 2.8M-triangle, 120 MB FBX, which is the shape that made a progress
/// indicator worth having at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    /// ufbx parsing the file (~1.6 s). The one stage with a real denominator:
    /// `done`/`total` are bytes, straight from ufbx's own progress callback.
    Reading,
    /// Marshaling the bridge's flat arrays into `ModelData`, validating them, and
    /// the rest-pose bounds and tangents the first frame needs (~0.3 s). No
    /// denominator. The model is drawable at the end of this stage.
    Building,
    /// Marshaling the source-property capture ([`SourceExtras`]) — authored
    /// properties, textures, topology layers, curves. No denominator, and after
    /// the model is on screen — see [`PendingExtras`].
    Extras,
    /// Measuring each clip's motion envelope (~0.6 s); `done`/`total` count clips.
    /// Runs *after* the model is on screen — see [`measure_clip_bounds`].
    Measuring,
    /// The per-draw-group stats table (~2.6 s). No denominator, and also after the
    /// model is on screen — see [`ModelData::mesh_group_stats`].
    Finishing,
}

impl ImportStage {
    /// A stable English name for this stage, for logs and the profiler. Not
    /// user-facing: the loading card names a stage through `app`'s own catalog
    /// key for it (invariant 12).
    pub fn label(self) -> &'static str {
        match self {
            Self::Reading => "Reading",
            Self::Building => "Building mesh",
            Self::Extras => "Reading source properties",
            Self::Measuring => "Measuring animation",
            Self::Finishing => "Measuring stats",
        }
    }
}

/// One progress report from an import. `total` is 0 when the stage has no
/// denominator, in which case only the stage itself is meaningful.
#[derive(Debug, Clone, Copy)]
pub struct ImportProgress {
    pub stage: ImportStage,
    pub done: u64,
    pub total: u64,
}

impl ImportProgress {
    /// A stage with no denominator, or one that hasn't started counting.
    pub fn stage(stage: ImportStage) -> Self {
        Self {
            stage,
            done: 0,
            total: 0,
        }
    }

    /// How far through this stage the import is, `None` when it can't be known.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
}

/// What [`load_model_with_progress`] reports through. Called on the importing
/// thread — from inside the ufbx parse for [`ImportStage::Reading`] — so it must
/// be cheap and must not block: `app` throttles and forwards to the event loop.
pub type ProgressSink<'a> = &'a dyn Fn(ImportProgress);

/// Import `path` completely, reporting nothing: the drawable model plus every
/// measurement [`load_model_with_progress`] leaves for the caller. What a test or
/// a batch tool wants; the viewer takes the staged path instead, so it can put
/// the mesh on screen before the measuring is done.
pub fn load_model(path: impl AsRef<Path>) -> Result<ModelData, ImportError> {
    let mut model = load_model_with_progress(path, &|_| {})?;
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    Ok(model)
}

/// [`load_model`] plus the source-property capture: the complete import, for
/// tests, batch callers and anything that will re-export. `None` extras means
/// the capture was unavailable, never that the file had nothing to capture.
pub fn load_model_full(
    path: impl AsRef<Path>,
) -> Result<(ModelData, Option<SourceExtras>), ImportError> {
    let StagedImport { mut model, extras } = load_model_staged(path, &|_| {})?;
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    let extras = extras.marshal(&model)?;
    Ok((model, extras))
}

/// A drawable model plus the source-property capture still waiting to be
/// marshaled. See [`load_model_staged`].
pub struct StagedImport {
    pub model: ModelData,
    pub extras: PendingExtras,
}

impl StagedImport {
    /// What the import had to leave out to load the model at all — see
    /// [`PendingExtras::warnings`].
    pub fn warnings(&self) -> &[String] {
        self.extras.warnings()
    }
}

/// The source-property capture as the C bridge left it: taken in the same
/// extraction call as the geometry (invariant 7), but marshaled into
/// [`SourceExtras`] only when [`PendingExtras::marshal`] is called — which
/// `app` does *after* it has published the model, so the viewport shows the
/// mesh first and the properties stream in behind it. The C buffers are freed
/// when this is marshaled or dropped, on every path.
pub struct PendingExtras {
    #[cfg(has_ufbx)]
    handle: Option<ffi::ExtrasHandle>,
    /// Why the bridge dropped the capture, when it did: the model loaded, and
    /// `marshal` reports this rather than publishing nothing in silence.
    #[cfg(has_ufbx)]
    dropped: Option<String>,
    /// What the import left out so the model could load at all: an animation
    /// key, a blend shape or a skin binding whose values were not numbers, an
    /// optional mesh layer whose values could not be read. One sentence each.
    warnings: Vec<String>,
}

impl PendingExtras {
    /// Nothing captured (a build without vendored ufbx, or a caller that
    /// declined the capture).
    pub fn none() -> Self {
        Self {
            #[cfg(has_ufbx)]
            handle: None,
            #[cfg(has_ufbx)]
            dropped: None,
            warnings: Vec::new(),
        }
    }

    /// What the import had to leave out of the model (or of the capture) to
    /// load it at all — a malformed animation key, blend shape or skin binding is
    /// dropped on its own rather than failing the whole file, and this is the
    /// record of it, for the caller to show. Empty for a file that loaded whole.
    /// English diagnostics, like [`ImportError`]'s: a caller showing them passes
    /// each through as a message variable.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Marshal the capture against the model it was taken with. `Ok(None)` when
    /// nothing was captured; `Err` when the capture does not describe `model`
    /// (a drift the funnel guard [`SourceExtras::validate`] caught) or the
    /// bridge had to drop it (out of memory, a count past its limits), in which
    /// case nothing is published and the model stands without it.
    pub fn marshal(self, model: &ModelData) -> Result<Option<SourceExtras>, ImportError> {
        #[cfg(has_ufbx)]
        {
            let _z = crate::prof::zone!("Marshal Extras");
            if let Some(reason) = self.dropped {
                return Err(ImportError::LoadFailed(format!(
                    "source properties: {reason}"
                )));
            }
            match self.handle {
                Some(handle) => handle.marshal(model).map(Some),
                None => Ok(None),
            }
        }
        #[cfg(not(has_ufbx))]
        {
            let _ = model;
            Ok(None)
        }
    }
}

impl std::fmt::Debug for PendingExtras {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingExtras")
    }
}

/// [`load_model_with_progress`] that also keeps the source-property capture,
/// for the caller to marshal once the model is on screen. Same stages, same
/// stopping point for the model itself.
pub fn load_model_staged(
    path: impl AsRef<Path>,
    progress: ProgressSink<'_>,
) -> Result<StagedImport, ImportError> {
    load_model_staged_cancellable(path, progress, None)
}

/// [`load_model_staged`] that abandons the parse when `cancel` says the request
/// has been superseded, reporting [`ImportError::Cancelled`].
///
/// The check rides the progress callback, which ufbx already calls every 512 KB,
/// so a cancelled load stops within a fraction of a second rather than running to
/// completion for a result that will be thrown away.
pub fn load_model_staged_cancellable(
    path: impl AsRef<Path>,
    progress: ProgressSink<'_>,
    cancel: Option<&CancelToken>,
) -> Result<StagedImport, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => {
            load_fbx_staged(path, progress, cancel)
        }
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

/// Import `path` as far as *drawable*, reporting each stage to `progress` as it
/// goes: geometry, materials, the scene graph, the rest-pose bounds the camera
/// frames on, and tangents.
///
/// This is the one import funnel (invariant 7), and it stops where the viewport
/// stops caring. Two measurements are deliberately **not** made here, because
/// nothing on screen needs them and together they are the majority of a large
/// load — a 2.8M-triangle scene reaches this point in 1.8 s and takes another
/// 3.2 s to measure:
///
/// * each clip's motion envelope ([`measure_clip_bounds`]), which only framing
///   and the bounding box on a *selected* clip use;
/// * the per-draw-group table ([`ModelData::mesh_group_stats`]) behind the stats
///   card's GPU Verts row and its scoped columns — hence `stats.gpu_vertex_count`
///   is left `0`, which the card reads as "not measured yet" and simply omits.
///
/// The caller makes them when it wants them; [`load_model`] makes them straight
/// away, `app` makes them on the import worker after the model is on screen and
/// folds each into the UI as it lands.
pub fn load_model_with_progress(
    path: impl AsRef<Path>,
    progress: ProgressSink<'_>,
) -> Result<ModelData, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx(path, progress),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

/// [`load_model_full`] over a file already in memory — what a shell extension
/// is handed, which gets the file's bytes as a stream rather than its path.
/// `name` is the file's own path when the caller knows it, or just its name:
/// the model is named after it, and relative texture paths resolve against
/// its directory. The bytes are always FBX; there is no extension to check.
pub fn load_model_full_from_bytes(
    bytes: &[u8],
    name: &Path,
) -> Result<(ModelData, Option<SourceExtras>), ImportError> {
    #[cfg(has_ufbx)]
    {
        let StagedImport { mut model, extras } =
            ffi::load_fbx(ffi::FbxInput::Bytes { bytes, name }, &|_| {}, true, None)?;
        model.stats.gpu_vertex_count = model.count_gpu_vertices();
        let extras = extras.marshal(&model)?;
        Ok((model, extras))
    }

    #[cfg(not(has_ufbx))]
    {
        let _ = (bytes, name);
        Err(ImportError::UfbxUnavailable)
    }
}

pub fn load_fbx(_path: &Path, _progress: ProgressSink<'_>) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(ffi::FbxInput::Path(_path), _progress, false, None).map(|staged| staged.model)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

fn load_fbx_staged(
    _path: &Path,
    _progress: ProgressSink<'_>,
    _cancel: Option<&CancelToken>,
) -> Result<StagedImport, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(ffi::FbxInput::Path(_path), _progress, true, _cancel)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

/// Each clip's motion envelope — the union of the posed mesh's bounds over every
/// frame — parallel to [`ModelData::animations`]. `None` for a clip that moves
/// nothing measurable.
///
/// Split out of the import funnel so it can run *after* the model is drawn (see
/// [`load_model_with_progress`]); it lives here rather than in the caller so the
/// [`AnimContext`] is built once rather than per clip. Each clip is stepped at
/// its own frame rate.
pub fn measure_clip_bounds(model: &ModelData) -> Vec<Option<review_model::Bounds>> {
    measure_clip_bounds_of(model, &model.animations)
}

/// How many frames of one clip a thread measures at a time. Small enough that a
/// long clip is shared out among the cores; large enough that the rest pose each
/// piece measures again for the static corners stays a small part of it.
const ENVELOPE_FRAMES_PER_PIECE: usize = 128;

/// [`measure_clip_bounds`] for clips that are not `model`'s own but play on it —
/// an attached animation file's, already re-pointed at `model`.
///
/// Measured on every core. Each frame is a pose and a walk of the moving mesh,
/// independent of every other, and a rig's clips are many and long: nine body
/// clips on a 210k-triangle character were 9.8 s on one thread, and one of them
/// was over a third of the frames, so sharing out whole clips still left that one
/// on a single core for 4 s. The clips are therefore cut into pieces of
/// [`ENVELOPE_FRAMES_PER_PIECE`] frames, which the threads claim one at a time,
/// and each clip's pieces are united. That is exact — a box is a minimum and a
/// maximum, which do not depend on the order they were taken in — so the result
/// is the same on any number of threads.
pub fn measure_clip_bounds_of(
    model: &ModelData,
    clips: &[review_model::AnimationClip],
) -> Vec<Option<review_model::Bounds>> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    if clips.is_empty() {
        return Vec::new();
    }
    let ctx = AnimContext::new(model);
    let pieces: Vec<(usize, std::ops::Range<usize>)> = clips
        .iter()
        .enumerate()
        .flat_map(|(index, clip)| {
            let frames = clip.frame_count();
            (0..frames)
                .step_by(ENVELOPE_FRAMES_PER_PIECE)
                .map(move |start| {
                    (
                        index,
                        start..(start + ENVELOPE_FRAMES_PER_PIECE).min(frames),
                    )
                })
        })
        .collect();
    let measure = |(index, frames): &(usize, std::ops::Range<usize>)| {
        review_model::anim::clip_bounds_over(model, &ctx, &clips[*index], frames.clone())
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(pieces.len());
    let next = AtomicUsize::new(0);
    let mut bounds: Vec<Option<review_model::Bounds>> = vec![None; clips.len()];
    let mut unite = |index: usize, piece: Option<review_model::Bounds>| {
        if let Some(piece) = piece {
            let whole = bounds[index].get_or_insert(review_model::Bounds::EMPTY);
            whole.include_point(piece.min);
            whole.include_point(piece.max);
        }
    };
    if threads <= 1 {
        for piece in &pieces {
            unite(piece.0, measure(piece));
        }
        return bounds;
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut measured = Vec::new();
                    while let Some(piece) = pieces.get(next.fetch_add(1, Ordering::Relaxed)) {
                        measured.push((piece.0, measure(piece)));
                    }
                    measured
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(measured) => {
                    for (index, piece) in measured {
                        unite(index, piece);
                    }
                }
                // A panic in a measurement is a bug; carry it on rather than
                // report an envelope that was never measured.
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    bounds
}
