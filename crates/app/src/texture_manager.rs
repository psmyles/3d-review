//! The scene texture pool + disk-auto-reload subsystem (an `impl App` block).
//!
//! Owns the import → decode → cache → pool → assign flow and the file-watcher
//! that re-decodes a bound texture when its source changes on disk. The five
//! texture fields stay on [`App`]; this module holds their logic. Background
//! decodes run off the main thread and post their result back through the winit
//! event loop ([`UserEvent`]) so all renderer mutation + redraw stays in `app`
//! (invariant 6). The subsystem Phases 4–7 (referenced/embedded textures, the Tex
//! viewport) extend, kept apart from the window/event-loop glue.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use review_render::{ChannelSelect, DecodedImage, TextureSlot, decode_image, suggested_channel};
use review_ui::{TexturePoolEntry, TextureSlotRef};

use crate::{App, TEXTURE_EXTENSIONS, UserEvent, file_label, prof};

/// A finished background texture decode, posted back to the event loop. Carries
/// what the decode was *for* ([`TextureDecodeRequest`]) so the main thread knows
/// how to apply the pixels (or report the failure).
#[derive(Debug, Clone)]
pub(crate) struct TextureDecode {
    request: TextureDecodeRequest,
    /// Decoded pixels, or a human-readable error (shown as an error toast).
    result: Result<DecodedImage, String>,
}

/// Why a texture was being decoded off-thread — determines how the result is
/// applied once it returns.
#[derive(Debug, Clone)]
enum TextureDecodeRequest {
    /// The user imported `path` into the scene texture pool; on completion it
    /// joins the pool (and the disk watcher) so material properties can bind it.
    Import { path: PathBuf },
    /// A watched file changed on disk; re-upload every binding using `path`.
    Reload { path: PathBuf },
}

impl TextureDecodeRequest {
    /// The source path this decode reads.
    pub(crate) fn path(&self) -> &Path {
        match self {
            TextureDecodeRequest::Import { path } | TextureDecodeRequest::Reload { path } => path,
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl App {
    pub(crate) fn refresh_texture_pool(&mut self) {
        self.ui.texture_pool = self
            .textures
            .pool
            .iter()
            .filter_map(|path| {
                self.textures.cache.get(path).map(|image| TexturePoolEntry {
                    path: path.clone(),
                    image: Arc::clone(image),
                    // On-disk size for the Tex viewport's stats panel; 0 (shown as
                    // "—") when the file can't be stat'd.
                    file_size: std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
                })
            })
            .collect();
    }

    /// Open the multi-select image picker and import each chosen file into the
    /// scene texture pool. Done from `apply_ui_output` (the modal blocks the loop).
    pub(crate) fn import_textures(&mut self) {
        let files = rfd::FileDialog::new()
            .add_filter("Image", &TEXTURE_EXTENSIONS)
            .set_title("Import Textures")
            .pick_files();
        if let Some(files) = files {
            for path in files {
                self.import_texture_path(path);
            }
        }
    }

    /// Import `path` into the scene texture pool. An already-decoded file (cache
    /// hit, or a drop of one already pooled) joins immediately; otherwise the
    /// decode runs on a background thread (so a slow PSD / large image can't freeze
    /// the event loop) and a "Decoding…" toast shows until it lands back via
    /// [`UserEvent::TextureDecoded`].
    pub(crate) fn import_texture_path(&mut self, path: PathBuf) {
        if self.textures.pool.contains(&path) {
            return;
        }
        // Already decoded (a prior import that was removed, say): pool on the spot.
        if self.textures.cache.contains_key(&path) {
            self.textures.pool.push(path.clone());
            self.textures.revision = self.textures.revision.wrapping_add(1);
            self.watch_texture(&path);
            self.refresh_texture_pool();
            self.redraw.requested = true;
            return;
        }
        self.notifications
            .begin_activity(format!("Decoding {}…", file_label(&path)));
        self.redraw.requested = true;
        self.spawn_decode(TextureDecodeRequest::Import { path });
    }

    /// Bind an already-pooled texture to a material slot, auto-detecting the
    /// channel routing from its filename. The image is in the pool (decoded), so
    /// this applies immediately.
    pub(crate) fn assign_pooled_texture(&mut self, slot_ref: TextureSlotRef, path: PathBuf) {
        let Some(slot) = TextureSlot::from_index(slot_ref.slot) else {
            return;
        };
        let Some(image) = self.textures.cache.get(&path).cloned() else {
            prof::msg(&format!(
                "assign of a texture not in the pool: {}",
                path.display()
            ));
            return;
        };
        let channel = suggested_channel(&path, slot);
        self.apply_assigned_texture(slot_ref, &path, image, channel);
    }

    /// Remove a texture from the pool and unbind every material slot that
    /// referenced it (reverting those slots to their neutral fallback).
    pub(crate) fn remove_texture(&mut self, path: &Path) {
        if let Some(renderer) = self.renderer.as_mut() {
            // `material_snapshot` is an owned copy, so iterating it while mutating
            // the renderer's slots below is fine.
            let snapshot = renderer.material_snapshot();
            for (material_index, material) in snapshot.iter().enumerate() {
                for slot in TextureSlot::ALL {
                    let references = material.state.textures[slot.index()]
                        .as_ref()
                        .is_some_and(|binding| binding.path == path);
                    if references {
                        renderer.clear_texture_slot(material_index, slot);
                    }
                }
            }
        }
        self.textures.pool.retain(|pooled| pooled != path);
        self.textures.cache.remove(path);
        self.textures.revision = self.textures.revision.wrapping_add(1);
        self.refresh_materials();
        self.refresh_texture_pool();
    }

    /// Apply an already-decoded image to a material slot: upload it, register the
    /// file for disk-auto-reload, and refresh the UI. Used by both the cache-hit
    /// path and the background-decode completion.
    pub(crate) fn apply_assigned_texture(
        &mut self,
        slot_ref: TextureSlotRef,
        path: &Path,
        image: Arc<DecodedImage>,
        channel: ChannelSelect,
    ) {
        let Some(slot) = TextureSlot::from_index(slot_ref.slot) else {
            return;
        };
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_texture_slot(slot_ref.material, slot, path.to_path_buf(), image, channel);
        }
        self.watch_texture(path);
        self.refresh_materials();
        self.redraw.requested = true;
    }

    /// Spawn a background thread that decodes `request`'s source image and posts the
    /// result back to the event loop. Decoding (especially a large PSD composite or
    /// a 4K image) can take a while, so it must never run on the main thread.
    fn spawn_decode(&mut self, request: TextureDecodeRequest) {
        let Some(proxy) = self.textures.proxy.clone() else {
            // Without a proxy the decode can never land back: end the paired
            // "Decoding…" / "Reloading…" activity toast (it would otherwise hang
            // forever) and surface the failure instead of silently dropping it.
            prof::msg("no event-loop proxy; cannot decode texture off-thread");
            self.notifications.end_activity();
            self.notifications
                .error(format!("Couldn't load {}", file_label(request.path())));
            return;
        };
        std::thread::spawn(move || {
            // Name the decode thread + time the decode in Tracy (both no-op unless
            // `--tracy`).
            prof::thread_name("texture-decode");
            let result = {
                let _z = prof::zone!("Decode Image");
                decode_image(request.path())
            };
            // A send failure only means the event loop has exited; nothing to do.
            let _ = proxy.send_event(UserEvent::TextureDecoded(TextureDecode { request, result }));
        });
    }

    /// Apply a finished background decode on the main thread: cache + upload the
    /// pixels and update the slot/binding, or report the failure. Always clears the
    /// activity toast it was paired with (in `import_texture_path` / `reload_texture_file`).
    pub(crate) fn handle_texture_decoded(&mut self, decode: TextureDecode) {
        self.notifications.end_activity();
        let TextureDecode { request, result } = decode;
        let path = request.path().to_path_buf();
        let name = file_label(&path);
        match result {
            Ok(image) => {
                let image = Arc::new(image);
                self.textures.cache.insert(path.clone(), Arc::clone(&image));
                // The pool/cache changed (an import landed, or a watched file
                // re-decoded), so bump the undo system's texture change tag.
                self.textures.revision = self.textures.revision.wrapping_add(1);
                match request {
                    TextureDecodeRequest::Import { .. } => {
                        if !self.textures.pool.contains(&path) {
                            self.textures.pool.push(path.clone());
                        }
                        self.watch_texture(&path);
                        self.refresh_texture_pool();
                        self.redraw.requested = true;
                        self.notifications.success(format!("Loaded {name}"));
                    }
                    TextureDecodeRequest::Reload { .. } => {
                        let updated = self
                            .renderer
                            .as_mut()
                            .is_some_and(|renderer| renderer.reload_texture(&path, image));
                        // Refresh the pool regardless so the thumbnail picks up the
                        // re-decoded image even if the file isn't bound to a slot.
                        self.refresh_texture_pool();
                        if updated {
                            self.refresh_materials();
                            self.redraw.requested = true;
                            self.notifications.info(format!("Reloaded {name}"));
                        }
                    }
                }
            }
            Err(error) => {
                prof::msg(&format!(
                    "texture decode failed {}: {error}",
                    path.display()
                ));
                self.notifications.error(format!("Couldn't load {name}"));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Register a texture file's parent directory with the disk watcher (created
    /// lazily). Watching the directory — not the file — tolerates editors that save
    /// via an atomic rename. Each directory is watched at most once.
    pub(crate) fn watch_texture(&mut self, path: &Path) {
        let Some(dir) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        if self.textures.watched_dirs.contains(&dir) {
            return;
        }
        if self.textures.watcher.is_none() {
            let Some(proxy) = self.textures.proxy.clone() else {
                return;
            };
            let handler = move |result: notify::Result<Event>| {
                match result {
                    Ok(event) => {
                        // Only content writes / creates matter (a save is one or both).
                        if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            for changed in event.paths {
                                let _ = proxy.send_event(UserEvent::TextureChanged(changed));
                            }
                        }
                    }
                    // A backend error stops reload events for its watch; leave a
                    // trace instead of dropping it without a word.
                    Err(error) => prof::msg(&format!("texture watcher error: {error}")),
                }
            };
            match RecommendedWatcher::new(handler, notify::Config::default()) {
                Ok(watcher) => self.textures.watcher = Some(watcher),
                Err(error) => {
                    prof::msg(&format!("failed to create texture watcher: {error}"));
                    self.notifications
                        .info("Texture auto-reload unavailable (file watcher failed)");
                    return;
                }
            }
        }
        if let Some(watcher) = self.textures.watcher.as_mut() {
            match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.textures.watched_dirs.insert(dir);
                }
                Err(error) => {
                    prof::msg(&format!(
                        "failed to watch texture directory {}: {error}",
                        dir.display()
                    ));
                    self.notifications
                        .info(format!("Auto-reload unavailable for {}", file_label(&dir)));
                    // Record the attempt so a failing directory isn't retried
                    // (and re-toasted) on every texture it contains.
                    self.textures.watched_dirs.insert(dir);
                }
            }
        }
    }

    /// Re-decode + re-upload a texture that changed on disk (the watcher event).
    /// Matches the event path against the bound texture paths (canonicalizing to
    /// tolerate path-form differences), then decodes off-thread (like an assign) so
    /// a slow re-decode of an edited PSD never freezes the loop; the reload is
    /// applied in `handle_texture_decoded`.
    pub(crate) fn reload_texture_file(&mut self, changed: &Path) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let Some(bound) = renderer
            .texture_paths()
            .into_iter()
            .find(|bound| same_path(bound, changed))
        else {
            return;
        };
        self.notifications
            .begin_activity(format!("Reloading {}…", file_label(&bound)));
        self.spawn_decode(TextureDecodeRequest::Reload { path: bound });
    }

    /// Drop all texture-watching + decode state (model load / reset): the new
    /// model's materials carry no textures, so old watches / cached images no
    /// longer apply.
    pub(crate) fn reset_texture_state(&mut self) {
        self.textures.cache.clear();
        self.textures.pool.clear();
        self.textures.revision = self.textures.revision.wrapping_add(1);
        self.ui.texture_pool = Vec::new();
        self.textures.watched_dirs.clear();
        // Dropping the watcher unregisters every directory.
        self.textures.watcher = None;
    }
}
