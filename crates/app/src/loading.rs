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
//! ready. The chrome comes up immediately over the empty viewport and a loading
//! card reports what the import is doing.
//!
//! **The worker publishes the model as soon as it is drawable, then keeps
//! measuring.** `review_import::load_model_with_progress` stops at the geometry,
//! materials, scene graph, rest-pose bounds and tangents — everything the first
//! frame needs — and the two measurements it leaves out (each clip's motion
//! envelope, and the per-draw-group stats table) are made afterwards, off the
//! same thread, against the `Arc` it just sent. Each lands as its own
//! [`UserEvent::ModelMeasured`] and folds into the UI where it belongs, so the
//! stats card fills in rather than the viewport waiting for it: on a
//! 2.8M-triangle scene that is the mesh on screen in ~1.8 s instead of ~5 s.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use review_import::{
    CancelToken, ImportError, ImportProgress, ImportStage, StagedImport,
    load_model_staged_cancellable, measure_clip_bounds,
};
use review_model::{Bounds, MeshGroupStats, ModelData, SceneBvh, SourceExtras};
use review_render::Renderer;
use review_ui::{ActivityId, Selection};

use crate::dialog::Dialog;
use crate::events::UserEvent;
use crate::keys;
use crate::{APP_NAME, App, prof};

/// The shortest gap between two progress reports reaching the event loop. The
/// loading card rewrites itself in place, so this is only about not waking the
/// event loop more often than a person can read: ufbx reports every 512 KB, which
/// on a 120 MB file is a couple of hundred callbacks in under two seconds.
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// A second primary click counts as a double-click only within this interval…
const DOUBLE_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(450);
/// …and only if the pointer stayed within this many physical pixels of the first.
const DOUBLE_CLICK_MAX_DISTANCE_PX: f32 = 6.0;

/// A drawable model from a background import, posted from the worker thread back
/// to the event loop.
#[derive(Debug)]
pub(crate) struct ModelLoaded {
    /// The load generation this import was started for. A result whose
    /// generation is no longer the current one — a newer open, or a Ctrl+N —
    /// is stale and dropped rather than shown.
    generation: u64,
    path: PathBuf,
    /// When the request was made, so the success notice can say how long the
    /// user waited for the model to appear. Started on the main thread at the
    /// top of [`App::open_model_from_path`] — the worker spawn, the parse and
    /// the hop back are all part of the wait.
    started: Instant,
    /// Already an `Arc`: the worker keeps a handle so it can go on measuring the
    /// model it has just published, and `app` would have made one anyway.
    result: Result<Arc<ModelData>, ImportError>,
    /// The loading card this import opened, which a failed import closes here.
    activity: ActivityId,
}

/// The source-property capture, marshaled on the import worker right after
/// the model was published. `Ok(None)` when this build cannot capture; `Err`
/// when the capture did not describe the model (nothing is kept, and the
/// export says so).
#[derive(Debug)]
pub(crate) struct SourceExtrasReady {
    /// Matched against the current load generation exactly as [`ModelLoaded`] is.
    generation: u64,
    extras: Result<Option<Arc<SourceExtras>>, ImportError>,
}

/// One of the measurements an import defers until after the model is drawn.
/// Applied to the UI as it lands, so the stats card fills in rather than the
/// viewport waiting for it.
#[derive(Debug)]
pub(crate) enum ModelMeasurement {
    /// Each clip's motion envelope, parallel to `ModelData::animations`.
    ClipBounds(Vec<Option<Bounds>>),
    /// The per-(node, material) table behind the `GPU Verts` row and the stats
    /// card's scoped columns.
    MeshGroups(Vec<MeshGroupStats>),
    /// The triangle BVH the viewport pick and the dimension labels query.
    SceneBvh(Arc<SceneBvh>),
    /// The worker stopped because its load was superseded. Carries nothing to
    /// apply; it exists so the last-message flag still reaches the event loop and
    /// closes the loading card that this import's request opened.
    Cancelled,
}

/// A measurement arriving from the import worker.
#[derive(Debug)]
pub(crate) struct ModelMeasured {
    /// Matched against the current load generation exactly as [`ModelLoaded`] is.
    generation: u64,
    measurement: ModelMeasurement,
    /// The import's last message: the loading card comes down here. Exactly one
    /// message per spawn carries it — this on the success path, `ModelLoaded` on
    /// the error path — so every `begin_activity` is balanced once.
    last: bool,
    /// The loading card this import opened.
    activity: ActivityId,
}

/// An import reporting where it has got to, posted from the worker thread.
#[derive(Debug)]
pub(crate) struct ModelLoadProgress {
    /// Matched against the current load generation exactly as [`ModelLoaded`] is,
    /// so a superseded import can't retitle the toast of the one that replaced it.
    generation: u64,
    /// The finished stage line, built on the worker: `Reading… 47%`.
    message: String,
    /// How far through that stage, when it can be known — the card's bar.
    fraction: Option<f32>,
    /// The loading card this import opened: its line is the one rewritten, even
    /// while another job's card is the one showing.
    activity: ActivityId,
}

/// The stage line under the loading card's title. The file is already named by
/// the title, so this is just what the import is doing, plus a percentage for the
/// one stage that has a real denominator ([`ImportStage`] says which).
fn progress_message(progress: ImportProgress) -> String {
    let stage = review_localization::tr(stage_name(progress.stage)).into_owned();
    match progress.fraction() {
        Some(fraction) => keys::app_notifications::stage_line_percent(
            f64::from((fraction * 100.0).round() as u32),
            stage,
        ),
        None => keys::app_notifications::stage_line(stage),
    }
}

/// What the loading card calls an import stage.
///
/// `import` keeps its own `label()` — it goes down the profiling channel, where
/// a translated string would make two captures harder to compare — so the map
/// lives here, on the side that draws text (invariant 12).
fn stage_name(stage: ImportStage) -> review_localization::Key {
    match stage {
        ImportStage::Reading => keys::app_notifications::STAGE_READING,
        ImportStage::Building => keys::app_notifications::STAGE_BUILDING,
        ImportStage::Extras => keys::app_notifications::STAGE_EXTRAS,
        ImportStage::Measuring => keys::app_notifications::STAGE_MEASURING,
        ImportStage::Finishing => keys::app_notifications::STAGE_FINISHING,
    }
}

/// Times an import's stages for the log, from the progress reports the import
/// already makes. Lives on the import worker, which is the only thread that
/// reports; `Cell` rather than a lock for the same reason the throttle is one.
#[derive(Default)]
struct StageClock {
    current: std::cell::Cell<Option<(ImportStage, Instant)>>,
}

impl StageClock {
    /// The import has reached `stage`. Ends (and logs) the stage before it; a
    /// repeat report within one stage changes nothing.
    fn enter(&self, stage: ImportStage) {
        match self.current.get() {
            Some((current, _)) if current == stage => {}
            _ => {
                self.finish();
                self.current.set(Some((stage, Instant::now())));
            }
        }
    }

    /// End the stage in progress, if any, and log how long it took.
    fn finish(&self) {
        if let Some((stage, since)) = self.current.take() {
            log::debug!(
                "import stage '{}' took {:.0} ms",
                stage.label(),
                since.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

/// A file's size for the log, or `?` when it cannot be read.
fn file_size_label(path: &Path) -> String {
    std::fs::metadata(path).map_or_else(
        |_| "?".to_owned(),
        |metadata| format!("{:.1} MB", metadata.len() as f64 / (1024.0 * 1024.0)),
    )
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
    pub(crate) fn open_model_from_dialog(&mut self) {
        self.ask(Dialog::OpenModel);
    }

    /// The newest model-load request's generation.
    pub(crate) fn model_load_generation(&self) -> u64 {
        self.model_load_generation.load(Ordering::Relaxed)
    }

    /// Supersede every load in flight, and return the new request's generation.
    ///
    /// One store does both jobs: a result carrying an older generation is dropped
    /// when it lands, and a worker reading the same counter through its
    /// [`CancelToken`] sees that it has been superseded and stops.
    fn supersede_loads(&mut self) -> u64 {
        let next = self.model_load_generation().saturating_add(1);
        self.model_load_generation.store(next, Ordering::Relaxed);
        next
    }

    pub(crate) fn open_model_from_path(&mut self, path: &Path) {
        // The clock the success notice reports runs from here — what the user
        // waited, not what the parse cost.
        let started = Instant::now();
        // Each request supersedes the last: only a result carrying the current
        // generation is applied, so a slow parse can never overwrite a newer one
        // — and the worker running it stops rather than finishing for nothing.
        let generation = self.supersede_loads();
        let cancel = CancelToken::new(Arc::clone(&self.model_load_generation), generation);
        log::info!("loading {} ({})", path.display(), file_size_label(path));

        // Parse on a worker thread so a large FBX can't hold the event loop —
        // at startup the window would otherwise stay blank until the model was
        // ready. The chrome keeps running; the result lands back on the main
        // thread via [`UserEvent::ModelLoaded`].
        if let Some(proxy) = self.textures.proxy.clone() {
            let activity = self
                .notifications
                .begin_activity(keys::app_notifications::loading(file_label(path)));
            self.redraw.requested = true;
            let path = path.to_path_buf();
            let failed_label = file_label(&path);
            let spawned = std::thread::Builder::new()
                .name("model-import".into())
                .spawn(move || {
                    prof::thread_name("model-import");
                    // Report where the parse has got to, so a 100 MB file shows
                    // movement instead of a silent "Loading…". Two filters keep the
                    // toast calm and the event loop idle: the rendered line must have
                    // actually changed (ufbx calls back every 512 KB, which is many
                    // reports per whole percent) and a beat must have passed since the
                    // last one. `Cell`, not an atomic — the sink is only ever called
                    // from this thread, from inside the import.
                    let last_sent = std::cell::Cell::new(None::<Instant>);
                    let progress_proxy = proxy.clone();
                    // Every report reaches the clock, before the throttle drops any:
                    // a stage boundary is exactly the report that must not be missed.
                    let clock = StageClock::default();
                    clock.enter(ImportStage::Reading);
                    // `path` travels with the model; the last line still names it.
                    let name = file_label(&path);
                    let report = |progress: ImportProgress| {
                        clock.enter(progress.stage);
                        if last_sent
                            .get()
                            .is_some_and(|sent| sent.elapsed() < PROGRESS_MIN_INTERVAL)
                        {
                            return;
                        }
                        last_sent.set(Some(Instant::now()));
                        let _ = progress_proxy.send_event(UserEvent::ModelLoadProgress(
                            ModelLoadProgress {
                                generation,
                                message: progress_message(progress),
                                fraction: progress.fraction(),
                                activity,
                            },
                        ));
                    };
                    let staged = {
                        let _z = prof::zone!("Import Model");
                        load_model_staged_cancellable(&path, &report, Some(&cancel))
                    };
                    let (result, pending) = match staged {
                        Ok(StagedImport { model, extras }) => (Ok(Arc::new(model)), Some(extras)),
                        Err(error) => (Err(error), None),
                    };
                    let measure = result.as_ref().ok().map(Arc::clone);
                    // A send failure only means the event loop has exited.
                    let _ = proxy.send_event(UserEvent::ModelLoaded(Box::new(ModelLoaded {
                        generation,
                        path,
                        started,
                        result,
                        activity,
                    })));

                    // The model is on screen from here; what is left is the source
                    // properties and the measurements. Each piece is sent the moment
                    // it lands, so the stats card and the clip framing fill in one at
                    // a time rather than together.
                    let Some(model) = measure else {
                        clock.finish();
                        return;
                    };
                    let send = |measurement, last| {
                        let _ =
                            proxy.send_event(UserEvent::ModelMeasured(Box::new(ModelMeasured {
                                generation,
                                measurement,
                                last,
                                activity,
                            })));
                    };
                    // Checked before each stage below; a worker that stops early
                    // still sends the last-message flag, since that is what closes
                    // the loading card.
                    let superseded = || {
                        if !cancel.is_cancelled() {
                            return false;
                        }
                        clock.finish();
                        log::debug!("model load superseded while measuring");
                        send(ModelMeasurement::Cancelled, true);
                        true
                    };
                    // Each of the three stages below is checked first. They are the
                    // majority of a large load — on a 2.8M-triangle scene the mesh
                    // groups alone are ~2.6 s — and none of it is worth doing once
                    // the user has opened something else. The last-message flag is
                    // what ends the loading card, so a worker that stops early still
                    // has to send it: `handle_model_measured` balances the activity
                    // before it looks at the generation.
                    if superseded() {
                        return;
                    }
                    if let Some(pending) = pending {
                        let _z = prof::zone!("Marshal Extras");
                        report(ImportProgress::stage(ImportStage::Extras));
                        let extras = pending.marshal(&model).map(|extras| extras.map(Arc::new));
                        let _ = proxy.send_event(UserEvent::SourceExtrasReady(Box::new(
                            SourceExtrasReady { generation, extras },
                        )));
                    }
                    if superseded() {
                        return;
                    }
                    {
                        // Before the clip envelopes, because this is what the pointer
                        // needs: a user reaching for a part of the model the moment
                        // it appears should be able to click it.
                        let _z = prof::zone!("Build Scene BVH");
                        report(ImportProgress::stage(ImportStage::Measuring));
                        send(
                            ModelMeasurement::SceneBvh(Arc::new(SceneBvh::build(&model))),
                            false,
                        );
                    }
                    if superseded() {
                        return;
                    }
                    {
                        let _z = prof::zone!("Measure Clip Bounds");
                        report(ImportProgress::stage(ImportStage::Measuring));
                        send(
                            ModelMeasurement::ClipBounds(measure_clip_bounds(&model)),
                            false,
                        );
                    }
                    if superseded() {
                        return;
                    }
                    {
                        let _z = prof::zone!("Measure Mesh Groups");
                        report(ImportProgress::stage(ImportStage::Finishing));
                        send(ModelMeasurement::MeshGroups(model.mesh_group_stats()), true);
                    }
                    clock.finish();
                    // The worker's own finish. The main thread's "loaded" line can
                    // come after it, since that is when the model reached the screen.
                    log::debug!(
                        "import worker finished {name} {:.2} s after the request",
                        started.elapsed().as_secs_f64()
                    );
                });
            if let Err(error) = spawned {
                // No worker, so no message will ever close the loading card.
                log::error!("could not start the import thread: {error}");
                self.notifications.end_activity(activity);
                self.notifications
                    .error(keys::app_notifications::couldnt_load(
                        failed_label,
                        error.to_string(),
                    ));
            }
        } else {
            // No proxy to post back through (never the case once `main` has
            // built the event loop) — load in place, measurements and all, so the
            // file still opens fully.
            match load_model_staged_cancellable(path, &|_| {}, None) {
                Ok(StagedImport { model, extras }) => {
                    let model = Arc::new(model);
                    let bvh = Arc::new(SceneBvh::build(&model));
                    let clip_bounds = measure_clip_bounds(&model);
                    let groups = model.mesh_group_stats();
                    let extras = extras.marshal(&model).map(|extras| extras.map(Arc::new));
                    self.apply_loaded_model(path, Ok(Arc::clone(&model)), started.elapsed());
                    self.apply_source_extras(extras);
                    self.ui.clip_bounds = clip_bounds;
                    self.ui.set_mesh_group_stats(groups);
                    self.scene_bvh = Some(bvh);
                }
                Err(error) => self.apply_loaded_model(path, Err(error), started.elapsed()),
            }
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Rewrite the loading card's stage line from a worker's progress report. A
    /// report from a superseded load is dropped, exactly as its result would be.
    pub(crate) fn handle_model_load_progress(&mut self, message: ModelLoadProgress) {
        if message.generation != self.model_load_generation() {
            return;
        }
        self.notifications
            .update_activity(message.activity, message.message, message.fraction);
        self.request_redraw();
    }

    /// Fold one deferred measurement into the UI. The model it describes is
    /// already on screen; a measurement whose generation has been superseded is
    /// dropped, but its `last` flag still balances the activity it began.
    pub(crate) fn handle_model_measured(&mut self, message: ModelMeasured) {
        if message.last {
            self.notifications.end_activity(message.activity);
        }
        if message.generation != self.model_load_generation() {
            return;
        }
        match message.measurement {
            // Framing and the bounding box describe the selected clip's whole
            // motion; until this lands they describe the whole model instead.
            ModelMeasurement::ClipBounds(bounds) => self.ui.clip_bounds = bounds,
            // Fills in the `GPU Verts` row and the scoped stats columns, and
            // spares the main thread building the same table itself.
            ModelMeasurement::MeshGroups(groups) => self.ui.set_mesh_group_stats(groups),
            // Until this lands the viewport cannot be picked and the dimension
            // labels do not occlude; both simply start working when it does.
            ModelMeasurement::SceneBvh(bvh) => self.scene_bvh = Some(bvh),
            // Only ever sent by a worker that has already been superseded, so
            // the generation check above has returned by now.
            ModelMeasurement::Cancelled => {}
        }
        self.request_redraw();
    }

    /// Keep the source-property capture of the model on screen. A capture whose
    /// generation has been superseded describes a model no longer shown and is
    /// dropped.
    pub(crate) fn handle_source_extras_ready(&mut self, message: SourceExtrasReady) {
        if message.generation != self.model_load_generation() {
            return;
        }
        self.apply_source_extras(message.extras);
    }

    fn apply_source_extras(&mut self, extras: Result<Option<Arc<SourceExtras>>, ImportError>) {
        match extras {
            Ok(extras) => self.scene_extras = extras,
            Err(error) => {
                // The model stays; only the re-export fidelity is lost, and the
                // export report will say so.
                self.scene_extras = None;
                log::warn!("source properties dropped: {error}");
                // A warning, not an error: the model loaded and is on screen.
                self.notifications
                    .warning(keys::app_notifications::couldnt_read_properties(
                        error.to_string(),
                    ));
            }
        }
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

    /// Show a drawable model from a background import. The loading card stays up:
    /// the worker is still measuring, and its final message is what ends the
    /// activity (a failed import has no measuring stage, so it ends here instead —
    /// exactly one of the two paths balances each `begin_activity`).
    pub(crate) fn handle_model_loaded(&mut self, message: ModelLoaded) {
        if message.result.is_err() {
            self.notifications.end_activity(message.activity);
        }

        // A cancelled import is not a failed one: the parse stopped because a
        // newer load superseded it, which is the same case as the generation
        // mismatch below and says nothing the user needs to hear.
        if matches!(message.result, Err(ImportError::Cancelled)) {
            log::debug!("model load cancelled mid-parse: {}", message.path.display());
        } else if message.generation == self.model_load_generation() {
            self.apply_loaded_model(&message.path, message.result, message.started.elapsed());
        } else {
            log::debug!("model load superseded, dropped: {}", message.path.display());
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Show a completed import — the loaded model, or its error toast — and
    /// re-point every piece of dependent state (framing, stats, materials, undo)
    /// at it. The one place both the worker path and the no-proxy fallback land.
    fn apply_loaded_model(
        &mut self,
        path: &Path,
        result: Result<Arc<ModelData>, ImportError>,
        elapsed: Duration,
    ) {
        // Loading into the empty viewport (first load, or after Ctrl+N) shows
        // the model already framed — a fly-in from the home view would only
        // delay it. Replacing an already-loaded model keeps the animated
        // re-frame so the view change reads as a transition.
        let animate_framing = !self.scene_model.vertices.is_empty();

        match result {
            Ok(model) => {
                let stats = model.stats;
                log::info!(
                    "loaded {} in {:.2} s: {} polygons, {} triangles, {} vertices, \
                     {} materials, {} draws, {} UV sets, {} nodes, {} bones, {} clips, \
                     1 unit = {} m",
                    path.display(),
                    elapsed.as_secs_f64(),
                    stats.polygon_count,
                    stats.triangle_count,
                    stats.vertex_count,
                    stats.material_count,
                    stats.draw_count,
                    stats.uv_set_count,
                    model.nodes.len(),
                    stats.bone_count,
                    stats.clip_count,
                    stats.source_unit_meters,
                );
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
                // The capture describing this model arrives on its own event a
                // moment later; the previous model's must not stand in for it.
                self.scene_extras = None;
                self.scene_revision = self.next_model_revision();
                // Any Opt result (and every per-object override) describes the
                // previous model, so drop both before the new one is drawn.
                self.reset_opt_for_new_model();
                let label = file_label(path);
                self.set_window_title(Some(&label));
                self.notifications.success(keys::app_notifications::loaded(
                    label.clone(),
                    format_load_time(elapsed),
                ));
                self.remember_recent_file(path);
                // A gate run starts measuring from here — the first present with
                // the model actually on screen (`gate.rs`); no-op otherwise.
                self.gate_model_ready(path);
            }
            Err(error) => {
                // Surface the cause, not just the file name — without `--tracy`
                // the prof channel below is the user's only *hidden* diagnostic.
                self.notifications
                    .error(keys::app_notifications::couldnt_load(
                        file_label(path),
                        error.to_string(),
                    ));
                log::error!("model load failed: {} ({error})", path.display());
                self.forget_missing_recent_file(path);
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
        // dismissed; superseding it both drops its result when it lands (its
        // "Loading…" toast ends there, where its refcount is balanced) and stops
        // the worker at its next progress report or stage boundary.
        self.supersede_loads();

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
        self.scene_extras = None;
        self.scene_revision = self.next_model_revision();
        self.reset_opt_for_new_model();
        self.set_window_title(None);

        log::info!("reset to start state");
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
        // into the old model; the skeleton state likewise.
        self.ui.selection = Selection::None;
        self.ui.solo = false;
        self.ui.hidden_meshes.clear();
        self.ui.reset_skeletal_state(model);
        // The clip selection and playback clock index the old model's clips, and
        // so do their measured envelopes — which the import worker refills for the
        // incoming model a moment after it is drawn.
        self.ui.reset_animation_state(model);
        self.ui.clip_bounds = vec![None; model.animations.len()];
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

    #[test]
    fn load_time_reads_as_seconds() {
        assert_eq!(format_load_time(Duration::from_millis(2340)), "2.3");
        assert_eq!(format_load_time(Duration::from_millis(420)), "0.42");
        assert_eq!(format_load_time(Duration::from_millis(1000)), "1.0");
        assert_eq!(format_load_time(Duration::from_secs(125)), "125.0");
    }
}
