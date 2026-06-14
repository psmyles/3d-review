use std::path::Path;

use review_model::ModelData;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("FBX import is unavailable until third_party/ufbx contains ufbx.c and ufbx.h")]
    UfbxUnavailable,
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    pub triangulate: bool,
}

pub fn load_model(path: impl AsRef<Path>, options: LoadOptions) -> Result<ModelData, ImportError> {
    let path = path.as_ref();
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("fbx") => load_fbx(path, options),
        Some(extension) => Err(ImportError::UnsupportedExtension(extension.to_owned())),
        None => Err(ImportError::UnsupportedExtension("<none>".to_owned())),
    }
}

pub fn load_fbx(_path: &Path, _options: LoadOptions) -> Result<ModelData, ImportError> {
    #[cfg(has_ufbx)]
    {
        ffi::load_fbx(_path, _options)
    }

    #[cfg(not(has_ufbx))]
    {
        Err(ImportError::UfbxUnavailable)
    }
}

#[cfg(has_ufbx)]
mod ffi {
    use std::path::Path;

    use review_model::ModelData;

    use crate::{ImportError, LoadOptions};

    pub(super) fn load_fbx(_path: &Path, _options: LoadOptions) -> Result<ModelData, ImportError> {
        Err(ImportError::UfbxUnavailable)
    }
}
