//! The scene texture pool + disk-auto-reload subsystem (an `impl App` block).
//!
//! Owns the import → decode → cache → pool → assign flow and the file-watcher
//! that re-decodes a bound texture when its source changes on disk. The five
//! texture fields stay on [`App`]; this module holds their logic. Background
//! decodes run off the main thread and post their result back through the winit
//! event loop ([`UserEvent`]) so all renderer mutation + redraw stays in `app`
//! (invariant 6). The subsystem Phases 4–7 (referenced/embedded textures, the Tex
//! viewport) extend, kept apart from the window/event-loop glue.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use review_render::{ChannelSelect, DecodedImage, TextureSlot, decode_image, suggested_channel};
use review_ui::{TexturePoolEntry, TextureSlotRef};
use winit::event_loop::EventLoopProxy;

use crate::dialog::Dialog;
use crate::events::UserEvent;
use crate::keys;
use crate::loading::file_label;
use crate::{App, prof};

/// A finished background texture decode, posted back to the event loop. Carries
/// what the decode was *for* ([`TextureDecodeRequest`]) so the main thread knows
/// how to apply the pixels (or report the failure).
#[derive(Debug, Clone)]
pub(crate) struct TextureDecode {
    request: TextureDecodeRequest,
    /// The scene generation this decode was started for. A result from a scene
    /// that has since been replaced describes textures nothing references any
    /// more, and is dropped exactly as a superseded model load is.
    generation: u64,
    /// Decoded pixels, or a human-readable error (shown as an error toast).
    result: Result<DecodedImage, String>,
}

/// Why a texture was being decoded off-thread — determines how the result is
/// applied once it returns.
#[derive(Debug, Clone)]
pub(crate) enum TextureDecodeRequest {
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

/// What to do with a request for a path a decode is already running for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Nothing is running for this path: start a worker.
    Spawn,
    /// A worker is already reading this path. Folded into it — either because
    /// the running decode will produce what was asked for (a second import), or
    /// by marking it dirty so one more decode follows it (the file changed
    /// again while it was being read).
    Coalesced,
}

/// What [`TextureSubsystem::finish_decode`] decided about a landed result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodeOutcome {
    /// Whether the pixels belong to the scene on screen.
    pub(crate) apply: bool,
    /// Whether the file changed again while this decode was running, so one more
    /// is owed.
    pub(crate) respawn: bool,
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

    /// Ask for images to add to the scene texture pool. The multi-select picker
    /// runs on a worker thread (`dialog.rs`); each chosen file reaches
    /// [`Self::import_texture_path`] when the answer comes back.
    pub(crate) fn import_textures(&mut self) {
        self.ask(Dialog::ImportTextures);
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
        let request = TextureDecodeRequest::Import { path };
        if self.textures.admit_decode(&request) == Admission::Coalesced {
            // Already being read — the running decode produces exactly what this
            // import wants, and it pools the file when it lands.
            return;
        }
        self.notifications
            .begin_activity(keys::app_notifications::decoding(file_label(
                request.path(),
            )));
        self.redraw.requested = true;
        self.spawn_decode(request);
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
            self.textures
                .finish_decode(self.textures.generation, request.path());
            self.notifications.end_activity();
            self.notifications
                .error(keys::app_notifications::texture_failed(file_label(
                    request.path(),
                )));
            return;
        };
        let generation = self.textures.generation;
        std::thread::spawn(move || {
            // Name the decode thread + time the decode in Tracy (both no-op unless
            // `--tracy`).
            prof::thread_name("texture-decode");
            let result = {
                let _z = prof::zone!("Decode Image");
                decode_image(request.path())
            };
            // A send failure only means the event loop has exited; nothing to do.
            let _ = proxy.send_event(UserEvent::TextureDecoded(TextureDecode {
                request,
                generation,
                result,
            }));
        });
    }

    /// Apply a finished background decode on the main thread: cache + upload the
    /// pixels and update the slot/binding, or report the failure. Always clears the
    /// activity toast it was paired with (in `import_texture_path` / `reload_texture_file`).
    pub(crate) fn handle_texture_decoded(&mut self, decode: TextureDecode) {
        // Balanced first and unconditionally: the activity counter is paired with
        // the *request*, so a result this scene no longer wants still has to
        // close the toast its request opened.
        self.notifications.end_activity();
        let TextureDecode {
            request,
            generation,
            result,
        } = decode;
        let path = request.path().to_path_buf();
        let name = file_label(&path);

        let outcome = self.textures.finish_decode(generation, &path);
        if outcome.respawn {
            // The file was written again while this decode was reading it, so
            // what just landed is already out of date. One more, which also
            // rescues the common case of an editor's first event arriving
            // mid-save: that read fails or reads a fragment, and this is the
            // read of the finished file.
            self.notifications
                .begin_activity(keys::app_notifications::reloading(name.clone()));
            self.spawn_decode(TextureDecodeRequest::Reload { path: path.clone() });
        }
        if !outcome.apply {
            prof::msg(&format!(
                "texture decode superseded, dropped: {}",
                path.display()
            ));
            return;
        }

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
                        self.notifications
                            .success(keys::app_notifications::texture_loaded(name.clone()));
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
                            self.notifications
                                .info(keys::app_notifications::texture_reloaded(name.clone()));
                        }
                    }
                }
            }
            Err(error) => {
                prof::msg(&format!(
                    "texture decode failed {}: {error}",
                    path.display()
                ));
                self.notifications
                    .error(keys::app_notifications::texture_failed(name.clone()));
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
                    self.notifications.warning(format!(
                        "{}\n{}",
                        review_l10n::tr(keys::app_notifications::WATCHER_UNAVAILABLE),
                        review_l10n::tr(keys::app_notifications::WATCHER_UNAVAILABLE_DESCRIPTION),
                    ));
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
                        .warning(keys::app_notifications::watch_failed(file_label(&dir)));
                    // Record the attempt so a failing directory isn't retried
                    // (and re-announced) on every texture it contains.
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
        let request = TextureDecodeRequest::Reload { path: bound };
        if self.textures.admit_decode(&request) == Admission::Coalesced {
            // A decode of this file is already running; it is now marked dirty,
            // so one more follows it. A single editor save emits several change
            // events, and each used to start its own full decode of the same
            // file.
            return;
        }
        self.notifications
            .begin_activity(keys::app_notifications::reloading(file_label(
                request.path(),
            )));
        self.spawn_decode(request);
    }

    /// Drop all texture-watching + decode state (model load / reset): the new
    /// model's materials carry no textures, so old watches / cached images no
    /// longer apply.
    pub(crate) fn reset_texture_state(&mut self) {
        self.textures.cache.clear();
        self.textures.pool.clear();
        self.textures.revision = self.textures.revision.wrapping_add(1);
        // Anything still on a decode worker was started for the scene being
        // replaced: bumping the generation is what drops its result when it
        // lands, and forgetting the in-flight set lets the new scene ask for the
        // same files without being coalesced into those workers.
        self.textures.generation = self.textures.generation.wrapping_add(1);
        self.textures.forget_in_flight();
        self.ui.texture_pool = Vec::new();
        self.textures.watched_dirs.clear();
        // Dropping the watcher unregisters every directory.
        self.textures.watcher = None;
    }
}

/// The scene texture pool + decode cache + disk-auto-reload subsystem's state,
/// grouped out of [`App`]; the logic lives in `texture_manager.rs`.
#[derive(Default)]
pub(crate) struct TextureSubsystem {
    /// Proxy used by the texture file-watcher thread to post reload events to the
    /// event loop (set in `main` before the loop runs).
    pub(crate) proxy: Option<EventLoopProxy<UserEvent>>,
    /// The disk-auto-reload watcher, created lazily on the first texture
    /// assignment. Dropping it stops watching (done on model load / reset).
    pub(crate) watcher: Option<RecommendedWatcher>,
    /// Directories the watcher is registered on (the parents of assigned textures),
    /// so each directory is watched at most once.
    pub(crate) watched_dirs: HashSet<PathBuf>,
    /// Decoded-image cache keyed by source path, so a packed map assigned to
    /// several slots / materials decodes once. Cleared on model load / reset.
    pub(crate) cache: HashMap<PathBuf, Arc<DecodedImage>>,
    /// The scene-wide texture pool: imported source paths in insertion order. The
    /// decoded pixels live in [`Self::cache`]; this is just the ordered set the
    /// Inspector's Texture files list + property dropdowns draw from (mirrored
    /// into `UiState::texture_pool` by `App::refresh_texture_pool`). Cleared on
    /// model load / reset.
    pub(crate) pool: Vec<PathBuf>,
    /// Monotonic change tag for the pool + cache, bumped on every mutation. Lets
    /// `App::capture_edit_state` detect pool changes (and share the pool snapshot
    /// `Arc` when unchanged) as cheaply as the renderer's `material_revision`
    /// does for the material table.
    pub(crate) revision: u64,
    /// Which scene the pool and cache describe, bumped whenever they are cleared
    /// for a new model. A decode carries the generation it was started for and is
    /// dropped when that no longer matches — the same guard `loading.rs` puts on
    /// a model load, and for the same reason: a slow PSD finishing after the user
    /// has opened another file would otherwise pool itself into the new scene.
    pub(crate) generation: u64,
    /// The paths a decode worker is currently reading, and whether the file has
    /// changed again since that worker started. One decode per path at a time:
    /// an editor's save emits several change events and a large PSD takes long
    /// enough that every one of them used to start its own full decode.
    in_flight: HashMap<PathBuf, bool>,
}

impl TextureSubsystem {
    /// The subsystem as `main` builds it: everything default except the proxy
    /// its workers post results back through.
    pub(crate) fn with_proxy(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy: Some(proxy),
            ..Self::default()
        }
    }

    /// Decide whether a request needs a worker of its own.
    ///
    /// This *is* the debounce, exactly as the Opt workspace's in-flight
    /// coalescing is (`crate::opt`): a second request for a path already being
    /// read folds into the running decode instead of starting a second one, and
    /// a reload marks it dirty so one more follows. No timer, and no guess at how
    /// long an editor takes to finish writing.
    pub(crate) fn admit_decode(&mut self, request: &TextureDecodeRequest) -> Admission {
        let path = request.path().to_path_buf();
        match self.in_flight.get_mut(&path) {
            Some(dirty) => {
                // An import is satisfied by whatever the running decode produces;
                // a reload means the bytes it is reading are already out of date.
                if matches!(request, TextureDecodeRequest::Reload { .. }) {
                    *dirty = true;
                }
                Admission::Coalesced
            }
            None => {
                self.in_flight.insert(path, false);
                Admission::Spawn
            }
        }
    }

    /// Retire a landed decode: whether to apply it, and whether the file changed
    /// again while it was being read.
    ///
    /// A result from a superseded scene leaves `in_flight` alone — the entry
    /// there belongs to whatever decode the *current* scene started for that
    /// path, and clearing it would let a second worker run beside it.
    pub(crate) fn finish_decode(&mut self, generation: u64, path: &Path) -> DecodeOutcome {
        if generation != self.generation {
            return DecodeOutcome {
                apply: false,
                respawn: false,
            };
        }
        let respawn = self.in_flight.remove(path).unwrap_or(false);
        DecodeOutcome {
            apply: true,
            respawn,
        }
    }

    /// Forget every in-flight decode, so the new scene's requests are not
    /// coalesced into workers reading for the old one. Their results are dropped
    /// by generation when they land.
    fn forget_in_flight(&mut self) {
        self.in_flight.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reload(path: &str) -> TextureDecodeRequest {
        TextureDecodeRequest::Reload {
            path: PathBuf::from(path),
        }
    }

    fn import(path: &str) -> TextureDecodeRequest {
        TextureDecodeRequest::Import {
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn a_burst_of_change_events_starts_one_decode() {
        let mut textures = TextureSubsystem::default();
        // What one editor save looks like from the watcher: several modify
        // events for the same file.
        assert_eq!(textures.admit_decode(&reload("a.psd")), Admission::Spawn);
        assert_eq!(
            textures.admit_decode(&reload("a.psd")),
            Admission::Coalesced
        );
        assert_eq!(
            textures.admit_decode(&reload("a.psd")),
            Admission::Coalesced
        );

        // One more decode is owed, since the later writes landed after the
        // running one started reading.
        let outcome = textures.finish_decode(0, Path::new("a.psd"));
        assert_eq!(
            outcome,
            DecodeOutcome {
                apply: true,
                respawn: true
            }
        );
    }

    #[test]
    fn a_quiet_decode_owes_nothing_further() {
        let mut textures = TextureSubsystem::default();
        assert_eq!(textures.admit_decode(&reload("a.psd")), Admission::Spawn);

        let outcome = textures.finish_decode(0, Path::new("a.psd"));
        assert_eq!(
            outcome,
            DecodeOutcome {
                apply: true,
                respawn: false
            }
        );
        // And the path is free again.
        assert_eq!(textures.admit_decode(&reload("a.psd")), Admission::Spawn);
    }

    #[test]
    fn a_second_import_of_one_path_does_not_mark_it_dirty() {
        let mut textures = TextureSubsystem::default();
        assert_eq!(textures.admit_decode(&import("a.png")), Admission::Spawn);
        assert_eq!(
            textures.admit_decode(&import("a.png")),
            Admission::Coalesced
        );

        let outcome = textures.finish_decode(0, Path::new("a.png"));
        assert!(
            !outcome.respawn,
            "the running decode already produces what the second import asked for"
        );
    }

    #[test]
    fn decodes_of_different_paths_run_side_by_side() {
        let mut textures = TextureSubsystem::default();
        assert_eq!(textures.admit_decode(&import("a.png")), Admission::Spawn);
        assert_eq!(textures.admit_decode(&import("b.png")), Admission::Spawn);
    }

    #[test]
    fn a_decode_from_a_replaced_scene_is_dropped() {
        let mut textures = TextureSubsystem::default();
        textures.admit_decode(&import("a.png"));
        let started_at = textures.generation;

        // The user opens another model while the decode is running.
        textures.generation = textures.generation.wrapping_add(1);
        textures.forget_in_flight();

        let outcome = textures.finish_decode(started_at, Path::new("a.png"));
        assert_eq!(
            outcome,
            DecodeOutcome {
                apply: false,
                respawn: false
            }
        );
    }

    #[test]
    fn a_stale_result_does_not_retire_the_new_scenes_decode() {
        let mut textures = TextureSubsystem::default();
        textures.admit_decode(&import("a.png"));
        let started_at = textures.generation;
        textures.generation = textures.generation.wrapping_add(1);
        textures.forget_in_flight();

        // The new scene asks for the same file, and *then* the old decode lands.
        assert_eq!(textures.admit_decode(&reload("a.png")), Admission::Spawn);
        textures.finish_decode(started_at, Path::new("a.png"));

        assert_eq!(
            textures.admit_decode(&reload("a.png")),
            Admission::Coalesced,
            "the current scene's decode is still running and must stay tracked"
        );
    }
}
