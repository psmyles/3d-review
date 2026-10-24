//! The textures the model's own materials name, found, decoded and bound as it
//! loads (an `impl App` block).
//!
//! The capture that names them (`review_model::SourceExtras`) is marshaled on
//! the import worker just after the mesh is on screen, so this starts when that
//! lands: a worker of its own plans the textures (embedded bytes, or the first
//! authored path that exists — `review_render::imported_textures`), decodes
//! them in parallel, and posts them back in one message. Each image then joins
//! the scene texture pool exactly as an imported one would — so the Inspector
//! lists it, the Tex viewport shows it, and a file on disk reloads when it is
//! saved — and is bound to every slot the materials use it for.
//!
//! They are part of the file rather than an edit, so binding them resets the
//! undo history to a baseline that has them: an undo entry from the moment
//! before they landed would otherwise hold a material table without them, and
//! undoing a color nudged in that second would strip every texture off the
//! model. What resets is at most a second of history, from a model the user
//! has only just opened.

use std::path::PathBuf;
use std::sync::Arc;

use review_model::{MaterialMap, ModelData, SourceExtras};
use review_render::{DecodedImage, ImportedTexture, imported_textures};
use review_ui::{ActivityId, NoticeKind};

use crate::events::UserEvent;
use crate::loading::file_label;
use crate::{App, keys, prof};

/// The model's textures, found and decoded on a worker.
#[derive(Debug)]
pub(crate) struct ImportedTexturesLoaded {
    /// The texture generation (`TextureSubsystem::generation`) the load started
    /// under: a result for a scene since replaced is dropped.
    generation: u64,
    textures: Vec<(ImportedTexture, Result<Arc<DecodedImage>, String>)>,
    /// The authored names of textures that were named but not found.
    missing: Vec<String>,
    activity: ActivityId,
}

/// Whether any material of `model` names a texture at all — a model that names
/// none starts no worker and raises no card.
fn names_textures(model: &ModelData) -> bool {
    model.materials.iter().any(|material| {
        MaterialMap::ALL
            .iter()
            .any(|&map| material.map(map).is_some())
    })
}

impl App {
    /// Find and decode the textures the model on screen names, now that the
    /// capture naming them has landed.
    pub(crate) fn load_imported_textures(&mut self) {
        let Some(extras) = self.scene_extras.clone() else {
            return;
        };
        let model = Arc::clone(&self.scene_model);
        if !names_textures(&model) {
            return;
        }
        let model_path = self.scene_files.first().cloned();
        let generation = self.textures.generation;
        let activity = self.notifications.begin_activity(review_localization::tr(
            keys::app_notifications::LOADING_TEXTURES,
        ));
        let Some(proxy) = self.textures.proxy.clone() else {
            // No event loop to post back through (a headless run): load in place.
            let loaded = load(&model, &extras, model_path, generation, activity);
            self.handle_imported_textures_loaded(loaded);
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("model-textures".into())
            .spawn(move || {
                prof::thread_name("model-textures");
                let loaded = load(&model, &extras, model_path, generation, activity);
                // A send failure only means the event loop has exited.
                let _ = proxy.send_event(UserEvent::ImportedTexturesLoaded(Box::new(loaded)));
            });
        if let Err(error) = spawned {
            log::error!("could not start the model texture thread: {error}");
            self.notifications.end_activity(activity);
        }
    }

    /// Pool and bind the model's textures, and say which could not be loaded.
    pub(crate) fn handle_imported_textures_loaded(&mut self, message: ImportedTexturesLoaded) {
        let ImportedTexturesLoaded {
            generation,
            textures,
            missing,
            activity,
        } = message;
        self.notifications.end_activity(activity);
        if generation != self.textures.generation {
            log::debug!("model textures superseded, dropped");
            return;
        }

        let mut unreadable = Vec::new();
        let mut bound = 0usize;
        for (texture, image) in textures {
            let image = match image {
                Ok(image) => image,
                Err(error) => {
                    log::warn!(
                        "model texture {} could not be read: {error}",
                        texture.key.display()
                    );
                    unreadable.push((file_label(&texture.key), error));
                    continue;
                }
            };
            let key = texture.key.clone();
            self.textures.cache.insert(key.clone(), Arc::clone(&image));
            if !self.textures.pool.contains(&key) {
                self.textures.pool.push(key.clone());
            }
            if texture.source.is_embedded() {
                // Its key names no file, so there is nothing to watch.
                self.textures.embedded.insert(key.clone());
            } else {
                self.watch_texture(&key);
            }
            if let Some(renderer) = self.renderer.as_mut() {
                for texture_use in &texture.uses {
                    if let Some(binding) = texture_use.binding(&key, &image) {
                        renderer.bind_imported_texture(&binding, key.clone(), Arc::clone(&image));
                        bound += 1;
                    }
                }
            }
        }
        log::info!(
            "model textures: {bound} binding(s), {} not found, {} unreadable",
            missing.len(),
            unreadable.len()
        );
        self.textures.revision = self.textures.revision.wrapping_add(1);
        self.refresh_texture_pool();
        self.refresh_materials();
        // Part of the file, not an edit: the history starts again with them in
        // it (see the module docs).
        self.reset_undo_history();
        self.request_redraw();
        self.report_textures_not_loaded(&missing, unreadable);
    }

    /// One card listing every texture the model names that could not be loaded.
    fn report_textures_not_loaded(
        &mut self,
        missing: &[String],
        unreadable: Vec<(String, String)>,
    ) {
        let count = missing.len() + unreadable.len();
        if count == 0 {
            return;
        }
        let model = self
            .scene_files
            .first()
            .map(|path| file_label(path))
            .unwrap_or_default();
        let lines =
            missing
                .iter()
                .map(|name| keys::app_notifications::texture_not_found(name.clone()))
                .chain(unreadable.into_iter().map(|(name, detail)| {
                    keys::app_notifications::texture_unreadable(detail, name)
                }))
                .collect();
        self.notifications.report(
            NoticeKind::Warning,
            keys::app_notifications::textures_not_loaded(count as f64, model),
            lines,
        );
    }
}

/// Plan and decode `model`'s textures. Runs on the worker.
fn load(
    model: &ModelData,
    extras: &SourceExtras,
    model_path: Option<PathBuf>,
    generation: u64,
    activity: ActivityId,
) -> ImportedTexturesLoaded {
    let _z = prof::zone!("Load Model Textures");
    let loaded = imported_textures::load(&model.materials, extras, model_path.as_deref(), None);
    for name in &loaded.planned.missing {
        log::warn!("model texture not found: {name}");
    }
    ImportedTexturesLoaded {
        generation,
        textures: loaded
            .planned
            .textures
            .into_iter()
            .zip(loaded.images)
            .collect(),
        missing: loaded.planned.missing,
        activity,
    }
}
