//! The model-load funnel: everything that turns "some paths arrived from
//! somewhere" into a model in the renderer, with any animation files played on it.
//!
//! Split by concern, one file each: the parse worker ([`parse`]), the pure rules
//! deciding which file is the model and which attach to it ([`classify`]),
//! putting the result on screen ([`session`]), and the measurements made once it
//! is there ([`measure`]). This file holds the entry points and the request queue
//! between them.
//!
//! **Every way of opening a file is a request with an intent**, and lands in
//! [`App::open_files`]: File > Open, the command line and the OS file association
//! open a *new session* of the files; File > Add Animations *attaches* them to
//! the model on screen; a drop on the window and File > Open Recent are a *drop*,
//! which attaches what binds to the model and replaces it otherwise
//! (`docs/animation-sources-plan.md` has the rules). So framing, stats, texture
//! watches, undo and redraw behave identically whichever way a file arrives.
//! [`App::reset_to_start_state`] (Ctrl+N) is the same reset run with an empty
//! model. Data still flows one way: these mutate `App`'s own state and drive the
//! renderer through its public API (invariant 2).
//!
//! **A request's files are parsed together, on a worker, and decided together.**
//! Which file is the rig is a property of the whole batch, so nothing is shown
//! until every file of it is read; the parse runs off the event loop as texture
//! decodes do ([`crate::texture_manager`]), so the chrome keeps running and a
//! loading card reports what the import is doing.
//!
//! **One request is parsed at a time.** A request that arrives while another is
//! being read waits for it, and is then decided against what that one put on
//! screen — so animation files dropped while their rig is still loading attach to
//! it rather than replacing it. Opening a new session is the exception: it
//! supersedes whatever is in flight, as opening a file always has.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use review_import::ImportError;
use review_model::ModelData;
use review_render::Renderer;

use crate::dialog::Dialog;
use crate::{APP_NAME, App, keys};

mod classify;
mod measure;
mod parse;
mod session;

pub(crate) use measure::{ModelMeasured, SourceExtrasReady};
use parse::ParsedFile;
pub(crate) use parse::{FilesParsed, ModelLoadProgress};

/// A second primary click counts as a double-click only within this interval…
const DOUBLE_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(450);
/// …and only if the pointer stayed within this many physical pixels of the first.
const DOUBLE_CLICK_MAX_DISTANCE_PX: f32 = 6.0;

/// What the user meant by handing the viewer some files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenIntent {
    /// Replace whatever is open with these files (File > Open, the command line,
    /// the OS file association).
    NewSession,
    /// Attach these to the model on screen; a file that does not bind to it is
    /// not loaded (File > Add Animations).
    Attach,
    /// Attach what binds to the model on screen, and replace it otherwise (a drop
    /// on the window, File > Open Recent).
    Drop,
}

/// Files to open, and what opening them means.
#[derive(Debug)]
pub(crate) struct LoadRequest {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) intent: OpenIntent,
}

/// The requests waiting their turn, and the one being parsed.
#[derive(Debug, Default)]
pub(crate) struct LoadQueue {
    /// The id of the request whose files are being parsed, if any.
    parsing: Option<u64>,
    /// The id the next request gets.
    next_id: u64,
    waiting: VecDeque<LoadRequest>,
    /// Model files dropped on the window this turn of the event loop. A drop of
    /// several files arrives as one event per file, so they are gathered here and
    /// opened together once the loop goes idle ([`App::flush_dropped_files`]).
    dropped: Vec<PathBuf>,
}

/// Whether `path`'s extension is one the texture pool accepts (case-insensitive).
fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| TEXTURE_EXTENSIONS.contains(&ext.as_str()))
}

impl App {
    /// A file dropped on the window. An image joins the scene texture pool at once
    /// (the Inspector's "drop to add"); anything else is held until every file of
    /// the drop has arrived, then opened with them.
    pub(crate) fn handle_dropped_file(&mut self, path: PathBuf) {
        if is_image_path(&path) {
            self.import_texture_path(path);
        } else {
            self.loads.dropped.push(path);
        }
    }

    /// Open the model files dropped since the last call as one request. Called
    /// when the event loop goes idle, by which time every file of a drop has been
    /// delivered.
    pub(crate) fn flush_dropped_files(&mut self) {
        if !self.loads.dropped.is_empty() {
            let paths = std::mem::take(&mut self.loads.dropped);
            self.open_files(paths, OpenIntent::Drop);
        }
    }

    /// Ask for files to open as a new session. The picker runs on a worker thread
    /// and comes back through the event loop (`dialog.rs`), so the answer lands in
    /// [`Self::open_files`] one turn of the loop later rather than in this call.
    pub(crate) fn open_model_from_dialog(&mut self) {
        self.ask(Dialog::OpenModel);
    }

    /// Ask for animation files to attach to the model on screen.
    pub(crate) fn add_animations_from_dialog(&mut self) {
        self.ask(Dialog::AddAnimations);
    }

    /// Open `path` as a new session — the command line, the OS file association.
    pub(crate) fn open_model_from_path(&mut self, path: &Path) {
        self.open_files(vec![path.to_path_buf()], OpenIntent::NewSession);
    }

    /// Open `paths` the way `intent` says. A new session starts at once,
    /// superseding any parse in flight (the model on screen goes on being
    /// measured until the new one replaces it); anything else waits for the
    /// request being parsed, so it is decided against what that one puts on
    /// screen.
    pub(crate) fn open_files(&mut self, mut paths: Vec<PathBuf>, intent: OpenIntent) {
        // The same file twice in one request would only be attached to itself.
        let mut seen: Vec<PathBuf> = Vec::with_capacity(paths.len());
        paths.retain(|path| {
            let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            if seen
                .iter()
                .any(|other| crate::paths::same_file(other, &absolute))
            {
                return false;
            }
            seen.push(absolute);
            true
        });
        if paths.is_empty() {
            return;
        }
        let request = LoadRequest { paths, intent };
        if intent == OpenIntent::NewSession {
            self.supersede_loads();
            self.loads.waiting.clear();
            self.start_request_now(request);
        } else if self.loads.parsing.is_some() {
            self.loads.waiting.push_back(request);
        } else {
            self.start_request_now(request);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Start parsing `request` now, whatever else is waiting.
    fn start_request_now(&mut self, request: LoadRequest) {
        let id = self.loads.next_id;
        self.loads.next_id = id.wrapping_add(1);
        self.loads.parsing = Some(id);
        self.spawn_parse(id, request);
    }

    /// Request `id` has finished parsing (or failed to start). The next waiting
    /// request is started by [`Self::start_next_request`] once its result is on
    /// screen.
    fn finish_request(&mut self, id: u64) {
        if self.loads.parsing == Some(id) {
            self.loads.parsing = None;
        }
    }

    /// Start the next waiting request, if nothing is being parsed.
    fn start_next_request(&mut self) {
        if self.loads.parsing.is_none()
            && let Some(request) = self.loads.waiting.pop_front()
        {
            self.start_request_now(request);
        }
    }

    /// The newest model-load request's generation.
    pub(crate) fn model_load_generation(&self) -> u64 {
        self.model_load_generation.load(Ordering::Relaxed)
    }

    /// Supersede every parse in flight, and return the new generation.
    ///
    /// One store does both jobs: a result carrying an older generation is dropped
    /// when it lands, and a worker reading the same counter through its
    /// [`review_import::CancelToken`] sees that it has been superseded and stops.
    ///
    /// The model on screen is not touched: its measuring runs on a counter of its
    /// own ([`Self::supersede_measuring`]), because a request can still fail, and
    /// the model it would have replaced has to stay measured if it does.
    fn supersede_loads(&mut self) -> u64 {
        let next = self.model_load_generation().saturating_add(1);
        self.model_load_generation.store(next, Ordering::Relaxed);
        next
    }

    /// The generation of the model on screen, as its measurements are matched.
    pub(crate) fn measure_generation(&self) -> u64 {
        self.measure_generation.load(Ordering::Relaxed)
    }

    /// Stop measuring the model on screen, because it is being replaced, and
    /// return the generation the next one is measured under.
    ///
    /// Only ever called with the replacement in hand: superseding when the
    /// request is *made* cancelled the measuring of a model that then stayed on
    /// screen whenever the request failed, and left it without a pick index,
    /// clip envelopes, a source capture or an audit for the rest of the session.
    fn supersede_measuring(&mut self) -> u64 {
        let next = self.measure_generation().saturating_add(1);
        self.measure_generation.store(next, Ordering::Relaxed);
        next
    }

    /// A request's files, parsed: report the ones that failed, decide what the
    /// rest become, and put that on screen.
    pub(crate) fn handle_files_parsed(&mut self, message: FilesParsed) {
        let FilesParsed {
            request,
            generation,
            intent,
            started,
            activity,
            files,
        } = message;
        self.finish_request(request);
        if generation != self.model_load_generation() {
            // A newer open superseded it while it was being read.
            log::debug!("load superseded, dropped: {} files", files.len());
            self.notifications.end_activity(activity);
            self.start_next_request();
            return;
        }

        let mut parsed = Vec::with_capacity(files.len());
        for ParsedFile { path, result } in files {
            match result {
                Ok(staged) => parsed.push((path, staged)),
                // Not a failure: the parse stopped because something newer
                // superseded it, which says nothing the user needs to hear.
                Err(ImportError::Cancelled) => {
                    log::debug!("model load cancelled mid-parse: {}", path.display());
                }
                Err(error) => {
                    // Surface the cause, not just the file name.
                    self.notifications
                        .error(keys::app_notifications::couldnt_load(
                            file_label(&path),
                            error.to_string(),
                        ));
                    log::error!("model load failed: {} ({error})", path.display());
                    self.forget_missing_recent_file(&path);
                }
            }
        }

        match self.apply_parsed(intent, parsed, started, activity) {
            Some(job) => self.spawn_measure(job),
            None => self.notifications.end_activity(activity),
        }
        self.start_next_request();
        self.request_redraw();
    }

    /// Name the loaded model in the window title — `Barrel.fbx — 3D Review`
    /// — so the title bar, Alt+Tab and the taskbar button say which file is
    /// open. `None` is the empty start state, which restores the bare product name.
    fn set_window_title(&self, model: Option<&str>) {
        if let Some(window) = self.window.as_ref() {
            match model {
                Some(name) => {
                    window.set_title(&keys::app_window::title_with_model(name, APP_NAME));
                }
                None => window.set_title(APP_NAME),
            }
        }
    }

    /// Ask for one more frame, from a background result that changed what is on
    /// screen.
    pub(crate) fn request_redraw(&mut self) {
        self.redraw.requested = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Return the viewer to its launch state (Ctrl+N): drop the loaded model and
    /// every file attached to it so the empty viewport is shown again, reset the
    /// dependent UI, and animate the camera back to its home framing. Bumping the
    /// scene revision drops the previously-uploaded GPU geometry on the next
    /// paint.
    pub(crate) fn reset_to_start_state(&mut self) {
        // A load still on the worker describes a model the user has just
        // dismissed; superseding it both drops its result when it lands (its
        // "Loading..." card ends there) and stops the worker at its next progress
        // report or stage boundary. Nothing waiting is wanted either.
        self.supersede_loads();
        self.loads.waiting.clear();

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
        self.replace_scene_model(empty, Vec::new());
        self.set_window_title(None);

        log::info!("reset to start state");
        self.redraw.requested = true;
    }

    /// Make `model`, opened from `files`, the model on screen — an empty
    /// [`ModelData`] and no files for the Ctrl+N start state — and re-point
    /// everything that described the outgoing one at it. Both entry points go
    /// through here, since the steps are only correct as a set: leaving one out
    /// strands the UI on indices, watches or history that describe a model no
    /// longer on screen. Returns the generation the new model is measured under.
    ///
    /// Call it with the *new* materials already in the renderer and in
    /// [`UiState::materials_snapshot`]: the undo rebaseline at the end snapshots
    /// what everything before it has set.
    ///
    /// [`UiState::materials_snapshot`]: review_ui::UiState::materials_snapshot
    fn replace_scene_model(&mut self, model: Arc<ModelData>, files: Vec<PathBuf>) -> u64 {
        self.reset_ui_for_new_model(&model);
        // The outgoing model's measuring describes a model about to leave the
        // screen; its results would be dropped on landing anyway.
        let generation = self.supersede_measuring();
        self.scene_model = model;
        self.scene_files = files;
        // The capture describing this model arrives on its own event a moment
        // later; the previous model's must not stand in for it.
        self.scene_extras = None;
        self.scene_revision = self.next_model_revision();
        // Any Opt result (and every per-object override) describes the previous
        // model, so drop both before the new one is drawn.
        self.reset_opt_for_new_model();
        self.reset_audit_for_new_model();
        // The undo history references the old model's indices / materials / pool;
        // drop it and rebaseline to this state. Last, because the resets above
        // bump revisions the snapshot compares (clearing the Opt overrides is a
        // stack edit): a baseline taken before them would read their bumps as an
        // edit on the next frame, and the first Ctrl+Z would put the outgoing
        // model's per-object overrides onto this one.
        self.reset_undo_history();
        generation
    }

    /// Re-point every piece of UI state that belongs to the *outgoing* model at
    /// `model`, the one about to be shown. Part of [`Self::replace_scene_model`],
    /// which is the only caller.
    fn reset_ui_for_new_model(&mut self, model: &ModelData) {
        // Indexes the outgoing model's triangles; the worker rebuilds it for the
        // incoming one a moment after it is drawn.
        self.scene_bvh = None;
        self.posed_pick = None;
        // Picking is a tool, not a document edit: a new file starts in View, so
        // opening one can never leave a stray click selecting something.
        self.ui.tool = review_ui::ViewportTool::View;
        self.announced_tool = review_ui::ViewportTool::View;
        self.ui.hover = None;
        // The Outliner selection, solo and hidden sets are node / material indices
        // into the old model; the skeleton state likewise. Through the one helper
        // `Esc` uses, so a texture inspected in the Textures tab is let go with
        // the rest — the pool it was in is cleared below. The audit profile is
        // the exception: it describes no model, so a load leaves it open.
        let profile_open = self.ui.aud.profile_open;
        self.ui.clear_selection();
        self.ui.aud.profile_open = profile_open;
        self.ui.solo = false;
        self.ui.hidden_meshes.clear();
        self.ui.reset_skeletal_state(model);
        // The clip selection, the playback clock and the attached animation files
        // all index the old session's clips, and so do their measured envelopes —
        // which the measuring worker refills a moment after the model is drawn.
        self.ui.reset_animation_state(model);
        self.reset_animation();
        // Every cached measurement is keyed on the outgoing model's node indices
        // or selection. Clearing the inputs above is not enough on its own: a new
        // model that reproduces a key — the same node index hidden or selected
        // again — would be served the previous model's box.
        self.ui.caches.reset();

        self.ui.stats = model.stats;
        self.ui.bounds = model.bounds;
        // Every UV picker resets to channel 0 so none points past the new model's
        // UV-set count; the view's dropdown labels come with it.
        self.ui.uv_checker.uv_channel = 0;
        self.ui.uv_seams.uv_channel = 0;
        self.ui.uv_sets = model.uv_set_labels();
        self.ui.uv_view_channel = 0;

        // The previous model's texture watches / decode cache no longer apply
        // (the fresh materials carry no slots).
        self.reset_texture_state();
    }

    pub(crate) fn should_open_on_double_click(&self) -> bool {
        // Only on the empty start state: a skeleton with no mesh has no vertices
        // either, and a double-click there is a click on the model.
        if !self.scene_model.is_empty() {
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

/// The image extensions the texture pool accepts (the picker filter + the
/// drag-drop routing). Anything else dropped on the window is treated as a model.
pub(crate) const TEXTURE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "tga", "tif", "tiff", "psd", "bmp", "gif",
];

/// A short human label for a texture path — its file name, or the full path when
/// it has no file-name component. Used in the notification toast captions.
pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// How long a load took, as the number the success notice puts its unit after:
/// `2.3`. Under a second a tenth is most of the figure, so those get two
/// decimals (`0.42`) - long enough to read as a real measurement rather than a
/// rounded `0.4`.
fn format_load_time(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds < 1.0 {
        format!("{seconds:.2}")
    } else {
        format!("{seconds:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request supersedes the parses in flight the moment it is made, but
    /// not the measuring of the model on screen: if every file of it fails, that
    /// model stays, and it must go on being measured — its pick index, clip
    /// envelopes, capture and audit all arrive through that worker.
    #[test]
    fn a_failed_open_leaves_the_model_on_screen_measured() {
        let mut app = App::default();
        let loads = app.model_load_generation();
        let measuring = app.measure_generation();
        app.open_files(
            vec![PathBuf::from("no-such-model-anywhere.fbx")],
            OpenIntent::NewSession,
        );
        assert!(
            app.model_load_generation() > loads,
            "the open superseded parses"
        );
        assert_eq!(
            app.measure_generation(),
            measuring,
            "the model on screen is still measured under its own generation"
        );
        assert!(
            app.loads.parsing.is_none(),
            "the failed request is finished"
        );
    }

    #[test]
    fn load_time_reads_as_seconds() {
        assert_eq!(format_load_time(Duration::from_millis(2340)), "2.3");
        assert_eq!(format_load_time(Duration::from_millis(420)), "0.42");
        assert_eq!(format_load_time(Duration::from_millis(1000)), "1.0");
        assert_eq!(format_load_time(Duration::from_secs(125)), "125.0");
    }
}
