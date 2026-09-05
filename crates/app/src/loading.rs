//! The model-load funnel: everything that turns "a path arrived from somewhere"
//! into "a loaded model in the renderer".
//!
//! Split out of `main.rs` as its own `impl App` block, beside `input.rs` /
//! `shortcuts.rs`. Every entry point — drag-drop, `Ctrl+O` / the file dialog, a
//! double-click on the empty viewport, and the command-line / file-association
//! path taken at startup — lands on [`App::open_model_from_path`], so framing,
//! stats, texture watches, undo and redraw behave identically whichever way the
//! file was handed over. [`App::reset_to_start_state`] (Ctrl+N) is the same funnel
//! run with an empty model. Data still flows one way: these mutate `App`'s own
//! state and drive the renderer through its public API (invariant 2).
//!
//! The FBX parse itself runs on a worker thread and posts back through the
//! event-loop proxy ([`UserEvent::ModelLoaded`]), exactly as texture decodes do
//! ([`crate::texture_manager`]): a large file would otherwise hold the event loop
//! for its whole parse — at startup that meant a blank window until the model was
//! ready. The chrome comes up immediately over the empty viewport, a "Loading…"
//! toast shows, and the model plus its dependent UI (stats, outliner, framing)
//! appear together when the parse lands.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use review_import::{ImportError, load_model};
use review_model::ModelData;
use review_render::Renderer;
use review_ui::Selection;

use crate::dialog::Dialog;
use crate::{App, TEXTURE_EXTENSIONS, UserEvent, file_label, prof};

/// A second primary click counts as a double-click only within this interval…
const DOUBLE_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(450);
/// …and only if the pointer stayed within this many physical pixels of the first.
const DOUBLE_CLICK_MAX_DISTANCE_PX: f32 = 6.0;

/// A finished background model import, posted from the worker thread back to
/// the event loop.
#[derive(Debug)]
pub(crate) struct ModelLoaded {
    /// The load generation this import was started for. A result whose
    /// generation is no longer the current one — a newer open, or a Ctrl+N —
    /// is stale and dropped rather than shown.
    generation: u64,
    path: PathBuf,
    result: Result<ModelData, ImportError>,
}

/// Whether `path`'s extension is one the texture pool accepts (case-insensitive).
fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| TEXTURE_EXTENSIONS.contains(&ext.as_str()))
}

impl App {
    /// A file dropped on the window.
    pub(crate) fn handle_dropped_file(&mut self, path: PathBuf) {
        // An image dropped anywhere joins the scene texture pool (the
        // Inspector's "drop to add"); anything else is treated as a model.
        if is_image_path(&path) {
            self.import_texture_path(path);
        } else {
            self.open_model_from_path(&path);
        }
    }

    /// Ask for a model to open. The picker runs on a worker thread and comes back
    /// through the event loop (`dialog.rs`), so the answer lands in
    /// [`Self::open_model_from_path`] one turn of the loop later rather than in
    /// this call.
    /// A macOS menu item the viewer performs itself (`mac-port-plan.md` D15).
    ///
    /// Both land on the same handlers the primary-modifier chords in
    /// `shortcuts.rs` fire, which is the point: the menu is a second door onto the
    /// existing commands, never a second implementation of them.
    pub(crate) fn handle_menu_command(&mut self, command: review_shell_macos::MenuCommand) {
        match command {
            review_shell_macos::MenuCommand::Open => self.open_model_from_dialog(),
            review_shell_macos::MenuCommand::New => self.reset_to_start_state(),
        }
    }

    pub(crate) fn open_model_from_dialog(&mut self) {
        self.ask(Dialog::OpenModel);
    }

    pub(crate) fn open_model_from_path(&mut self, path: &Path) {
        // Loading a model (drag-drop, file dialog, or CLI arg) dismisses the
        // startup help overlay if it's still up.
        self.ui.show_help_overlay = false;

        // Each request supersedes the last: only a result carrying the current
        // generation is applied, so a slow parse can never overwrite a newer one.
        self.model_load_generation = self.model_load_generation.saturating_add(1);
        let generation = self.model_load_generation;

        // Parse on a worker thread so a large FBX can't hold the event loop —
        // at startup the window would otherwise stay blank until the model was
        // ready. The chrome keeps running; the result lands back on the main
        // thread via [`UserEvent::ModelLoaded`].
        if let Some(proxy) = self.textures.proxy.clone() {
            self.notifications
                .begin_activity(format!("Loading {}…", file_label(path)));
            self.redraw.requested = true;
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                prof::thread_name("model-import");
                let result = {
                    let _z = prof::zone!("Import Model");
                    load_model(&path)
                };
                // A send failure only means the event loop has exited.
                let _ = proxy.send_event(UserEvent::ModelLoaded(Box::new(ModelLoaded {
                    generation,
                    path,
                    result,
                })));
            });
        } else {
            // No proxy to post back through (never the case once `main` has
            // built the event loop) — load in place so the file still opens.
            let result = load_model(path);
            self.apply_loaded_model(path, result);
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Apply a finished background import on the main thread: end the activity
    /// toast its spawn began (the toast is refcounted, so every spawn balances),
    /// and show the model unless a newer request has superseded this one.
    pub(crate) fn handle_model_loaded(&mut self, message: ModelLoaded) {
        self.notifications.end_activity();

        if message.generation == self.model_load_generation {
            self.apply_loaded_model(&message.path, message.result);
        } else {
            prof::msg(&format!(
                "model load superseded, dropped: {}",
                message.path.display()
            ));
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Show a completed import — the loaded model, or its error toast — and
    /// re-point every piece of dependent state (framing, stats, materials, undo)
    /// at it. The one place both the worker path and the no-proxy fallback land.
    fn apply_loaded_model(&mut self, path: &Path, result: Result<ModelData, ImportError>) {
        // Loading into the empty viewport (first load, or after Ctrl+N) shows
        // the model already framed — a fly-in from the home view would only
        // delay it. Replacing an already-loaded model keeps the animated
        // re-frame so the view change reads as a transition.
        let animate_framing = !self.scene_model.vertices.is_empty();

        match result {
            Ok(model) => {
                let model = Arc::new(model);

                let (materials_snapshot, material_revision) =
                    if let Some(renderer) = self.renderer.as_mut() {
                        frame_camera_to_model(renderer, &model, animate_framing);
                        renderer.reset_uv_camera();
                        // Seed the editable material table from the import defaults.
                        renderer.set_model_materials(&model.materials);
                        (renderer.material_snapshot(), renderer.material_revision())
                    } else {
                        (Vec::new(), 0)
                    };
                self.ui.materials_snapshot = materials_snapshot;
                self.ui.material_revision = material_revision;
                self.reset_ui_for_new_model(&model);
                self.scene_model = model;
                self.scene_revision = self.next_model_revision();
                // Any Opt result (and any node override) describes the previous
                // model, so drop both before the new one is drawn.
                self.reset_opt_for_new_model();
                self.notifications
                    .success(format!("Loaded {}", file_label(path)));
                prof::msg(&format!("model loaded: {}", path.display()));
                // A gate run starts measuring from here — the first present with
                // the model actually on screen (`gate.rs`); no-op otherwise.
                self.gate_model_ready(path);
            }
            Err(error) => {
                // Surface the cause, not just the file name — without `--tracy`
                // the prof channel below is the user's only *hidden* diagnostic.
                self.notifications
                    .error(format!("Couldn't load {}: {error}", file_label(path)));
                prof::msg(&format!("model load failed: {} ({error})", path.display()));
            }
        }
        self.redraw.requested = true;
    }

    /// Return the viewer to its launch state (Ctrl+N): drop the loaded model so
    /// the empty viewport is shown again, reset the dependent UI, and animate the
    /// camera back to its home framing. Bumping the scene revision drops the
    /// previously-uploaded GPU geometry on the next paint.
    pub(crate) fn reset_to_start_state(&mut self) {
        // A load still on the worker describes a model the user has just
        // dismissed; bumping the generation drops its result when it lands
        // (its "Loading…" toast ends there, where its refcount is balanced).
        self.model_load_generation = self.model_load_generation.saturating_add(1);

        let empty = Arc::new(ModelData::default());

        let material_revision = if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_camera_to_home();
            renderer.reset_uv_camera();
            // Drop the previous model's editable materials.
            renderer.set_model_materials(&[]);
            renderer.material_revision()
        } else {
            0
        };
        self.ui.materials_snapshot = Vec::new();
        self.ui.material_revision = material_revision;
        self.reset_ui_for_new_model(&empty);
        self.scene_model = empty;
        self.scene_revision = self.next_model_revision();
        self.reset_opt_for_new_model();

        prof::msg("reset to start state");
        self.redraw.requested = true;
    }

    /// Re-point every piece of UI state that belongs to the *outgoing* model at
    /// `model`, the one about to be shown — an empty [`ModelData`] for the Ctrl+N
    /// start state. Both entry points go through here, since the steps are only
    /// correct as a set: leaving one out strands the UI on indices, watches or
    /// history that describe a model no longer on screen.
    ///
    /// Call it with the *new* materials already in [`UiState::materials_snapshot`]
    /// and before `scene_model` is replaced: the undo rebaseline at the end
    /// snapshots what everything above it has just set.
    ///
    /// [`UiState::materials_snapshot`]: review_ui::UiState::materials_snapshot
    fn reset_ui_for_new_model(&mut self, model: &ModelData) {
        // The Outliner selection, solo and hidden sets are node / material indices
        // into the old model; the skeleton state likewise.
        self.ui.selection = Selection::None;
        self.ui.solo = false;
        self.ui.hidden_meshes.clear();
        self.ui.reset_skeletal_state(model);
        // The clip selection and playback clock index the old model's clips.
        self.ui.reset_animation_state(model);
        self.reset_animation();
        // Every cached measurement is keyed on the outgoing model's node indices
        // or selection. Clearing the inputs above is not enough on its own: a new
        // model that reproduces a key — the same node index hidden or selected
        // again — would be served the previous model's box.
        self.ui.caches.reset();

        self.ui.stats = model.stats;
        self.ui.bounds = model.bounds;
        // Both UV pickers reset to channel 0 so neither points past the new
        // model's UV-set count; the view's dropdown labels come with it.
        self.ui.uv_checker.uv_channel = 0;
        self.ui.uv_sets = model.uv_set_labels();
        self.ui.uv_view_channel = 0;

        // The previous model's texture watches / decode cache no longer apply
        // (the fresh materials carry no slots).
        self.reset_texture_state();
        // The undo history references the old model's indices / materials / pool;
        // drop it and rebaseline to this state.
        self.reset_undo_history();
    }

    pub(crate) fn should_open_on_double_click(&self) -> bool {
        if !self.scene_model.vertices.is_empty() {
            return false;
        }

        let Some((last_click_time, last_click_position)) = self.last_primary_click else {
            return false;
        };
        let Some(current_position) = self.last_pointer_position else {
            return false;
        };

        last_click_time.elapsed() <= DOUBLE_CLICK_MAX_INTERVAL
            && current_position.distance(last_click_position) <= DOUBLE_CLICK_MAX_DISTANCE_PX
    }
}

fn frame_camera_to_model(renderer: &mut Renderer, model: &ModelData, animate: bool) {
    if let Some(bounds) = model.bounds {
        if animate {
            renderer.animate_camera_to_bounds(bounds);
        } else {
            renderer.snap_camera_to_bounds(bounds);
        }
    }
}
