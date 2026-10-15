//! Loading one model, as the viewer does.
//!
//! The same import funnel (invariant 7) and the same two steps after it: the
//! GPU-vertex count the stats card and the Opt baseline read, and the
//! source-property capture the export and the audit read. A capture that cannot
//! be marshaled costs the model nothing - it loads, the run goes on, and the
//! warning says what will not carry over - exactly as it is in the viewer.

use std::io::Read;
use std::path::Path;

use review_import::{ImportError, ImportProgress, ImportStage};
use review_model::{ModelData, SourceExtras};

use crate::keys::cli as k;

pub(crate) struct Loaded {
    pub(crate) model: ModelData,
    pub(crate) extras: Option<SourceExtras>,
    /// Things worth saying that did not stop the load.
    pub(crate) warnings: Vec<String>,
}

/// Load `path`, reporting how far through reading the file it is (`0.0..=1.0`).
pub(crate) fn load(path: &Path, progress: &dyn Fn(f32)) -> Result<Loaded, String> {
    let report = |update: ImportProgress| {
        if matches!(update.stage, ImportStage::Reading)
            && let Some(fraction) = update.fraction()
        {
            progress(fraction);
        }
    };
    let staged =
        review_import::load_model_staged(path, &report).map_err(|error| problem(path, &error))?;
    let mut model = staged.model;
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    let mut warnings = Vec::new();
    let extras = match staged.extras.marshal(&model) {
        Ok(extras) => extras,
        Err(error) => {
            warnings.push(k::properties_dropped(error.to_string()));
            None
        }
    };
    Ok(Loaded {
        model,
        extras,
        warnings,
    })
}

/// Why `path` did not load: the importer's own diagnostic (invariant 12 keeps a
/// diagnostic in its library's words), unless the file is a Git LFS pointer -
/// the usual reason an `.fbx` in a fresh checkout is not an FBX, and one the
/// reader can fix in one command.
fn problem(path: &Path, error: &ImportError) -> String {
    let mut head = [0u8; 64];
    let read = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .unwrap_or(0);
    if head[..read].starts_with(b"version https://git-lfs.github.com/spec/") {
        review_localization::tr(k::LFS_POINTER).into_owned()
    } else {
        error.to_string()
    }
}
