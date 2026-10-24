//! A model loaded for previewing: the mesh, its textures decoded, and its rest
//! pose.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use review_import::ImportError;
use review_model::{AnimContext, Bounds, DeformPose, ModelData, Pose, anim};
use review_render::{LoadedTextures, imported_textures};

/// Where a preview's model comes from.
#[derive(Debug, Clone, Copy)]
pub enum SceneSource<'a> {
    /// A file on disk: textures resolve against its directory.
    Path(&'a Path),
    /// A file's bytes, as Explorer hands a thumbnail handler. `name` is the
    /// file's own path when the shell knows it, or only its name — then the
    /// only textures it can find are embedded ones and those at the absolute
    /// paths the file wrote down.
    Bytes { bytes: &'a [u8], name: &'a Path },
}

/// Why a scene could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Import(#[from] ImportError),
    /// The file loaded but has nothing to draw: a skeleton and its clips, saved
    /// apart from the character they move.
    #[error("the file has no mesh to draw")]
    NothingToDraw,
}

/// Every scene gets its own id, which is also the revision the renderer keys
/// its mesh buffers on — unique across every mesh a process hands it, as `app`'s
/// own revisions are.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A model ready to draw: the mesh, the textures its materials name (decoded),
/// and the pose a skinned model rests in.
#[derive(Debug)]
pub struct PreviewScene {
    pub(crate) id: u64,
    pub(crate) model: Arc<ModelData>,
    pub(crate) textures: LoadedTextures,
    /// The rest pose a skinned or blend-shaped model is shown in, as the viewer
    /// shows it; `None` for a static model, which draws its own buffers.
    pub(crate) pose: Option<DeformPose>,
}

impl PreviewScene {
    /// Load `source` and decode its textures, each halved to fit
    /// `max_texture_side` when one is given — a 256-pixel thumbnail has no use
    /// for a 4K map, and halving it on the CPU is cheaper than uploading and
    /// mipping the levels it will never sample.
    pub fn load(source: SceneSource<'_>, max_texture_side: Option<u32>) -> Result<Self, LoadError> {
        let (model, extras, model_path) = match source {
            SceneSource::Path(path) => {
                let (model, extras) = review_import::load_model_full(path)?;
                (model, extras, Some(path))
            }
            SceneSource::Bytes { bytes, name } => {
                let (model, extras) = review_import::load_model_full_from_bytes(bytes, name)?;
                (model, extras, name.is_absolute().then_some(name))
            }
        };
        if model.vertices.is_empty() || model.indices.is_empty() {
            return Err(LoadError::NothingToDraw);
        }
        // The capture is only needed to find the textures: everything else in it
        // is for an exporter, and it is dropped here rather than held beside the
        // decoded images for as long as the preview is up.
        let textures = extras.map_or_else(LoadedTextures::default, |extras| {
            imported_textures::load(&model.materials, &extras, model_path, max_texture_side)
        });
        for name in &textures.planned.missing {
            log::info!("{}: texture not found: {name}", model.name);
        }
        for (texture, error) in textures.failures() {
            log::warn!(
                "{}: texture {} did not decode: {error}",
                model.name,
                texture.key.display()
            );
        }
        let pose = model.needs_deform().then(|| rest_pose(&model));
        Ok(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            model: Arc::new(model),
            textures,
            pose,
        })
    }

    /// The model's name: its file name without the extension.
    pub fn name(&self) -> &str {
        &self.model.name
    }

    /// What framing fits: the model's bounds at its rest pose.
    pub fn bounds(&self) -> Option<Bounds> {
        self.model.bounds
    }

    /// The authored names of textures the model names that could not be found.
    pub fn missing_textures(&self) -> &[String] {
        &self.textures.planned.missing
    }
}

/// The palette that draws `model` in the pose it rests in — the file's default
/// pose, not the bind pose its buffers hold — exactly as the viewer shows it.
fn rest_pose(model: &ModelData) -> DeformPose {
    let context = AnimContext::new(model);
    let mut pose = Pose::new(model);
    anim::rest_pose(model, &context, &mut pose);
    let mut deform = DeformPose::default();
    anim::build_palette(model, &context, &pose, &mut deform);
    deform
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/test_models")
            .join(name)
    }

    /// Skip a fixture test in a checkout without fixtures — unless
    /// `REVIEW_REQUIRE_FIXTURES` says this run may not (`scripts/check`).
    fn present(path: &Path) -> bool {
        if path.exists() {
            return true;
        }
        assert!(
            !std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0"),
            "REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {} is missing",
            path.display()
        );
        false
    }

    /// The thumbnail's way in (bytes, named by the stream) and the preview's
    /// (the path) are the same model, ready to draw.
    #[test]
    fn a_scene_from_bytes_is_the_scene_from_the_path() {
        let path = fixture("SM_Wall_Break_4x3m.fbx");
        if !present(&path) {
            return;
        }
        let from_path = PreviewScene::load(SceneSource::Path(&path), None).expect("loads");
        let bytes = std::fs::read(&path).expect("read the fixture");
        let from_bytes = PreviewScene::load(
            SceneSource::Bytes {
                bytes: &bytes,
                name: Path::new("SM_Wall_Break_4x3m.fbx"),
            },
            Some(256),
        )
        .expect("loads from bytes");
        assert_eq!(from_bytes.model.vertices, from_path.model.vertices);
        assert_eq!(from_bytes.bounds(), from_path.bounds());
        assert_eq!(from_bytes.name(), "SM_Wall_Break_4x3m");
        assert_ne!(
            from_bytes.id, from_path.id,
            "every scene is its own mesh revision"
        );
    }

    #[test]
    fn a_file_that_is_not_an_fbx_does_not_load() {
        let result = PreviewScene::load(
            SceneSource::Bytes {
                bytes: b"not an fbx",
                name: Path::new("broken.fbx"),
            },
            None,
        );
        assert!(matches!(result, Err(LoadError::Import(_))));
    }
}
