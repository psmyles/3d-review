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

use std::path::Path;

use review_model::{AnimContext, ModelData, SourceExtras};
use thiserror::Error;

#[cfg(has_ufbx)]
mod ffi;
mod prof;
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
}

/// The stage an import is in, as reported to [`load_model_with_progress`].
///
/// These are the four measured phases of a load, in order; the numbers beside
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
    /// The user-facing verb for this stage, as it appears in the loading card.
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

/// The source-property capture as the C bridge left it: taken in the same
/// extraction call as the geometry (invariant 7), but marshaled into
/// [`SourceExtras`] only when [`PendingExtras::marshal`] is called — which
/// `app` does *after* it has published the model, so the viewport shows the
/// mesh first and the properties stream in behind it. The C buffers are freed
/// when this is marshaled or dropped, on every path.
pub struct PendingExtras {
    #[cfg(has_ufbx)]
    handle: Option<ffi::ExtrasHandle>,
}

impl PendingExtras {
    /// Nothing captured (a build without vendored ufbx, or a caller that
    /// declined the capture).
    pub fn none() -> Self {
        Self {
            #[cfg(has_ufbx)]
            handle: None,
        }
    }

    /// Marshal the capture against the model it was taken with. `Ok(None)` when
    /// nothing was captured; `Err` when the capture does not describe `model`
    /// (a drift the funnel guard [`SourceExtras::validate`] caught), in which
    /// case nothing is published.
    pub fn marshal(self, model: &ModelData) -> Result<Option<SourceExtras>, ImportError> {
        #[cfg(has_ufbx)]
        {
            let _z = crate::prof::zone!("Marshal Extras");
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
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx_staged(path, progress),
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

pub fn load_fbx(_path: &Path, _progress: ProgressSink<'_>) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _progress, false).map(|staged| staged.model)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

fn load_fbx_staged(_path: &Path, _progress: ProgressSink<'_>) -> Result<StagedImport, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _progress, true)
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
/// two things a caller could get wrong — using the file's own frame rate, and
/// building the [`AnimContext`] once rather than per clip — are decided once.
pub fn measure_clip_bounds(model: &ModelData) -> Vec<Option<review_model::Bounds>> {
    if model.animations.is_empty() {
        return Vec::new();
    }
    let ctx = AnimContext::new(model);
    let fps = model.frame_rate_or_default();
    model
        .animations
        .iter()
        .map(|clip| review_model::anim::clip_bounds(model, &ctx, clip, fps))
        .collect()
}
