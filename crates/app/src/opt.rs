//! The Opt workspace's app-side machinery: running the operation stack on a
//! background thread and applying results back on the main thread.
//!
//! ## Why a worker at all
//!
//! Simplifying a real game asset takes long enough to drop frames, and the whole
//! point of the workspace is scrubbing a slider and watching the mesh respond.
//! So a run happens off-thread and posts its result back through the event-loop
//! proxy, exactly as a texture decode does ([`crate::texture_manager`]).
//!
//! ## Latest-request-wins
//!
//! Dragging a slider produces an edit per frame. Each bumps a generation counter,
//! but only one worker is ever in flight: edits arriving mid-run just mark the
//! subsystem dirty, and the handler respawns once with the *latest* stack when
//! the run lands. The in-flight coalescing is the debounce — no timer needed —
//! and a result whose generation is no longer current is dropped rather than
//! shown, so a slow run can never overwrite a newer one.
//!
//! ## Laziness
//!
//! Startup speed is a product goal, so nothing here is built until the user
//! actually enters the Opt workspace: `App::opt` is `None` until then. Entering
//! it does schedule one run even with an empty stack — that run produces no mesh,
//! only the source's measured figures, which the workspace shows as the baseline
//! every later change is quoted against.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use review_model::{ModelData, SceneBvh};
use review_optimize::{
    ExportOptions, ExportReport, OpKind, OptError, OptStack, ProcessInput, ProcessedResult,
    RebindReport, export_fbx, preset, process,
};
use review_ui::{NoticeKind, OptIntent, OptLevelView, OptResultView};

use crate::dialog::Dialog;
use crate::events::UserEvent;
use crate::keys;
use crate::{App, prof};

/// How long a run may take before the user is told it is still going. Below this
/// the result usually lands within a frame or two and a toast would only flicker.
const ACTIVITY_NOTICE_AFTER: Duration = Duration::from_millis(300);

/// What to tell the user about a loaded preset's per-object overrides, one line
/// per thing that happened. A preset that landed cleanly produces nothing.
///
/// The two halves are separate warnings because they mean different things: a
/// dropped override is a setting the user has lost, while one matched by
/// position is a setting that *may* have landed on the wrong object — only an
/// old preset can produce it, and only the user can tell whether it is right.
fn rebind_notes(report: RebindReport) -> Vec<String> {
    // Both notes read differently for one override than for several. That is a
    // plural rule, so it belongs in the catalog's own `{ $count -> }` selector
    // rather than in hand-picked `if count == 1` fragments here — English needs
    // two forms, and other languages need more or fewer.
    let mut notes = Vec::new();
    if report.dropped > 0 {
        notes.push(keys::app_notifications::overrides_dropped(
            report.dropped as f64,
        ));
    }
    if report.by_position > 0 {
        notes.push(keys::app_notifications::overrides_by_position(
            report.by_position as f64,
        ));
    }
    notes
}

/// A finished run, posted from the worker thread back to the event loop.
#[derive(Debug)]
pub(crate) struct OptProcessed {
    /// The generation this run was started for. A result whose generation is no
    /// longer the current one is stale and dropped.
    pub(crate) generation: u64,
    pub(crate) result: Result<ProcessedResult, OptError>,
    /// One triangle BVH per produced level, parallel to `result`'s `lods` — what
    /// a pick in the split's processed half queries, and what occludes that
    /// half's dimension labels.
    ///
    /// Built on the same worker as the run itself: a processed level is a whole
    /// new mesh, so its index is as expensive to build as the source's and just
    /// as unwelcome on the main thread (invariant 6). Empty when the run failed
    /// or produced no mesh.
    pub(crate) level_bvhs: Vec<Arc<SceneBvh>>,
}

/// Everything the Opt workspace needs on the app side. Created on first entry
/// into the workspace, so a session that never opens it pays nothing.
#[derive(Debug)]
pub(crate) struct OptSubsystem {
    /// The most recent completed run, shared by `Arc` so the render path can
    /// borrow a level's mesh without copying it.
    pub(crate) processed: Option<Arc<ProcessedResult>>,
    /// One triangle BVH per level of [`Self::processed`], in the same order.
    /// Replaced wholesale with each run, so it can never describe a mesh that is
    /// no longer on screen.
    pub(crate) level_bvhs: Vec<Arc<SceneBvh>>,
    /// Model revision for the level currently being drawn. Drawn from the app's
    /// shared revision counter so it can never collide with a source model's.
    pub(crate) processed_revision: u64,
    /// Which level `processed_revision` was issued for, so switching levels
    /// re-issues one (the renderer caches its mesh buffers by revision).
    revision_level: usize,
    /// The stack revision the newest run *covers* — set when that run is
    /// started, not when it lands. It has to be the request that marks a
    /// revision as handled: were it only set on completion, the retry below
    /// would fire again on every frame a run was in flight, and each retry bumps
    /// the generation, so the run would be stale by the time it landed and the
    /// workspace would reprocess forever without ever showing a result.
    covers_revision: u64,
    /// The stack revision most recently seen from the UI.
    seen_revision: u64,
    /// Monotonic run counter; the newest generation is the only one accepted.
    generation: u64,
    /// The generation of the run currently on the worker thread, if any.
    in_flight: Option<u64>,
    /// An edit landed while a run was in flight, so another is owed.
    dirty: bool,
    started_at: Option<Instant>,
    /// Whether the "still working" toast is currently up.
    activity_shown: bool,
    /// The warnings the last accepted run reported, so a condition that persists
    /// across runs is announced once rather than once per run.
    announced_warnings: Vec<String>,
    /// The Outliner-hidden node set most recently seen (sorted). Visibility is
    /// part of the AO bake's input — a hidden mesh neither occludes nor bakes —
    /// so toggling an eye while a bake is enabled reruns the stack.
    seen_hidden: Vec<u32>,
}

impl Default for OptSubsystem {
    fn default() -> Self {
        Self {
            processed: None,
            level_bvhs: Vec::new(),
            processed_revision: 0,
            revision_level: usize::MAX,
            covers_revision: u64::MAX,
            seen_revision: u64::MAX,
            generation: 0,
            in_flight: None,
            dirty: false,
            started_at: None,
            activity_shown: false,
            announced_warnings: Vec::new(),
            seen_hidden: Vec::new(),
        }
    }
}

impl OptSubsystem {
    /// The mesh for `level` of the latest result, if there is one.
    pub(crate) fn level_model(&self, level: usize) -> Option<&ModelData> {
        self.processed
            .as_ref()
            .and_then(|result| result.lod(level))
            .map(|lod| &lod.model)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether a run should be queued for `stack_revision` this frame: either the
    /// UI just edited the stack, or there is no result and nothing has been asked
    /// to produce one (a run that errored leaves the revision covered, so a
    /// failure is reported once rather than retried forever).
    fn needs_run(&self, stack_revision: u64, changed: bool) -> bool {
        changed || (self.processed.is_none() && self.covers_revision != stack_revision)
    }
}

impl App {
    /// Bring the Opt workspace up to date: create the subsystem on first use,
    /// notice stack edits, and manage the "still working" toast.
    ///
    /// Called once per frame while the Opt workspace is active. Doing nothing at
    /// all in the other workspaces is what keeps the feature free for a session
    /// that never opens it.
    pub(crate) fn sync_opt(&mut self) {
        let stack_revision = self.ui.opt.stack_revision;
        // Outliner visibility is part of the AO bake's input (a hidden mesh
        // neither occludes nor bakes), so toggling an eye reruns the stack —
        // but only when an enabled bake is actually reading it; no other
        // operation consults visibility.
        let hidden = self.ui.hidden_mesh_nodes();
        let bake_enabled = self
            .ui
            .opt
            .stack
            .ops
            .iter()
            .any(|op| op.enabled && matches!(op.kind, OpKind::BakeAo(_)));
        let opt = self.opt.get_or_insert_with(OptSubsystem::default);

        // First visit: adopt the current stack revision without treating it as an
        // edit, so merely opening the workspace with an empty stack schedules
        // nothing.
        let changed = opt.seen_revision != stack_revision;
        opt.seen_revision = stack_revision;

        let hidden_changed = opt.seen_hidden != hidden;
        if hidden_changed {
            opt.seen_hidden = hidden;
        }

        if opt.needs_run(stack_revision, changed || (hidden_changed && bake_enabled)) {
            self.schedule_reprocess();
        }

        // Raise the "still working" notice only once a run has actually been slow,
        // and only once per run.
        let opt = self.opt.as_mut().expect("just created");
        if let Some(started) = opt.started_at
            && opt.in_flight.is_some()
            && !opt.activity_shown
            && started.elapsed() >= ACTIVITY_NOTICE_AFTER
        {
            opt.activity_shown = true;
            self.notifications.begin_activity(
                review_localization::tr(keys::app_notifications::OPTIMIZING).into_owned(),
            );
        }

        let running = self.opt.as_ref().is_some_and(OptSubsystem::is_running);
        if self.ui.opt.processing != running {
            self.ui.opt.processing = running;
            self.redraw.requested = true;
        }

        // Turning sync back on should snap the second view to the first, not wait
        // for the next drag. Doing it while synced also keeps the two cameras
        // equal through the animated moves (framing, home, the WASD steps, the
        // gizmo), which only ever drive the main one.
        if self.opt_cameras_synced()
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.sync_opt_camera();
        }
    }

    /// Queue a run for the current stack, or mark one owed if the worker is busy.
    pub(crate) fn schedule_reprocess(&mut self) {
        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        opt.generation = opt.generation.saturating_add(1);
        opt.dirty = true;
        if opt.in_flight.is_none() {
            self.spawn_process();
        }
    }

    /// Start a worker for the current stack.
    fn spawn_process(&mut self) {
        let stack = Arc::clone(&self.ui.opt.stack);
        let model = Arc::clone(&self.scene_model);
        let extras = self.scene_extras.clone();
        let stack_revision = self.ui.opt.stack_revision;

        // A stack with nothing enabled still runs: it produces no mesh, but it
        // measures the source, which is what the workspace shows as the baseline
        // before anything has been asked of it. An empty *model* has nothing to
        // measure either way.
        if model.indices.is_empty() {
            let Some(opt) = self.opt.as_mut() else {
                return;
            };
            opt.dirty = false;
            opt.processed = None;
            opt.level_bvhs.clear();
            opt.covers_revision = stack_revision;
            self.ui.opt.result = None;
            self.ui.opt.active_lod = 0;
            // Free the processed mesh's GPU buffers rather than leave them
            // resident for a mesh nothing will draw (invariant 3).
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.release_processed_mesh();
            }
            self.redraw.requested = true;
            return;
        }

        let Some(proxy) = self.textures.proxy.clone() else {
            prof::msg("no event-loop proxy; cannot optimize off-thread");
            self.notifications.error(
                review_localization::tr(keys::app_notifications::OPT_START_FAILED).into_owned(),
            );
            // Mark the revision covered even though nothing ran: the failure has
            // been reported once, and leaving it uncovered would re-report it on
            // every frame.
            if let Some(opt) = self.opt.as_mut() {
                opt.dirty = false;
                opt.covers_revision = stack_revision;
            }
            return;
        };

        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        let generation = opt.generation;
        opt.in_flight = Some(generation);
        opt.dirty = false;
        opt.covers_revision = stack_revision;
        opt.started_at = Some(Instant::now());

        // The renderer's own vertex size, so the reported overfetch describes the
        // buffer the GPU actually reads rather than the optimizer's intermediate
        // layout (invariant 5: a measured figure, of the real thing).
        let render_vertex_size = review_render::scene_vertex_size();
        // Snapshot of the Outliner-hidden nodes, read only by the AO bake.
        let hidden = self.ui.hidden_mesh_nodes();

        std::thread::spawn(move || {
            prof::thread_name("mesh-optimize");
            let result = {
                let _z = prof::zone!("Optimize Mesh");
                process(ProcessInput {
                    model: &model,
                    stack: &stack,
                    render_vertex_size,
                    hidden_nodes: &hidden,
                    extras: extras.as_deref(),
                })
            };
            // Indexed here rather than on the main thread, for the same reason
            // the source mesh's is built on the import worker.
            let level_bvhs = match &result {
                Ok(processed) => {
                    let _z = prof::zone!("Build Level BVHs");
                    processed
                        .lods
                        .iter()
                        .map(|lod| Arc::new(SceneBvh::build(&lod.model)))
                        .collect()
                }
                Err(_) => Vec::new(),
            };
            // A send failure only means the event loop has exited.
            let _ = proxy.send_event(UserEvent::OptProcessed(Box::new(OptProcessed {
                generation,
                result,
                level_bvhs,
            })));
        });
    }

    /// Apply a finished run on the main thread.
    pub(crate) fn handle_opt_processed(&mut self, message: OptProcessed) {
        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        opt.in_flight = None;
        opt.started_at = None;
        if opt.activity_shown {
            opt.activity_shown = false;
            self.notifications.end_activity();
        }

        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        // A result for a superseded stack is dropped: the newer run is already
        // queued and showing this one would flash a mesh the user has moved past.
        let current = message.generation == opt.generation;
        if current {
            match message.result {
                Ok(result) => self.accept_opt_result(result, message.level_bvhs),
                Err(error) => {
                    prof::msg(&format!("mesh optimization failed: {error}"));
                    self.notifications
                        .error(keys::app_notifications::optimization_failed(
                            crate::explain::explain_opt_error(&error),
                        ));
                }
            }
        }

        // Edits that arrived mid-run are honoured now, with the latest stack.
        let owed = self.opt.as_ref().is_some_and(|opt| opt.dirty);
        if owed {
            self.spawn_process();
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Ask where to write the current LOD chain.
    ///
    /// The chain, the source model and the export options are captured *here*, not
    /// when the picker closes: the workspace stays live behind the dialog, so a
    /// background run could land a different chain, or the user edit the stack,
    /// between the click and the chosen path. What they clicked Export on is what
    /// travels out to the dialog worker and comes back to be written.
    fn export_opt_result(&mut self) {
        let Some(result) = self.opt.as_ref().and_then(|opt| opt.processed.clone()) else {
            self.notifications.error(
                review_localization::tr(keys::app_notifications::NOTHING_TO_EXPORT).into_owned(),
            );
            return;
        };

        // A sensible default file name: the model's own, which is also the stem
        // the per-LOD packaging appends its suffixes to.
        let stem = if self.scene_model.name.is_empty() {
            "optimized".to_owned()
        } else {
            self.scene_model.name.clone()
        };
        self.ask(Dialog::ExportOpt {
            result,
            source: Arc::clone(&self.scene_model),
            extras: self.scene_extras.clone(),
            options: self.ui.opt.stack.export,
            stem,
        });
    }

    /// Write a chosen LOD chain out to `path`.
    ///
    /// The write runs on a worker like processing does: a large chain in ASCII is
    /// slow enough to drop frames, and freezing the window mid-export is exactly
    /// the failure the workspace is built to avoid.
    pub(crate) fn spawn_opt_export(
        &mut self,
        path: PathBuf,
        result: Arc<ProcessedResult>,
        source: Arc<ModelData>,
        extras: Option<Arc<review_model::SourceExtras>>,
        options: ExportOptions,
    ) {
        let Some(proxy) = self.textures.proxy.clone() else {
            prof::msg("no event-loop proxy; cannot export off-thread");
            self.notifications.error(
                review_localization::tr(keys::app_notifications::EXPORT_START_FAILED).into_owned(),
            );
            return;
        };

        self.notifications.begin_activity(
            review_localization::tr(keys::app_notifications::EXPORTING).into_owned(),
        );

        std::thread::spawn(move || {
            prof::thread_name("mesh-export");
            let outcome = {
                let _z = prof::zone!("Export FBX");
                export_fbx(&result.lods, &source, extras.as_deref(), &path, &options)
            };
            let _ = proxy.send_event(UserEvent::OptExported(Box::new(outcome)));
        });
    }

    /// Report a finished export on the main thread.
    pub(crate) fn handle_opt_exported(&mut self, outcome: Result<ExportReport, OptError>) {
        self.notifications.end_activity();
        match outcome {
            Ok(report) => {
                let files = report.files.len();
                let name = report
                    .files
                    .first()
                    .map(|path| crate::loading::file_label(path))
                    .unwrap_or_else(|| {
                        review_localization::tr(keys::app_notifications::EXPORTED_FALLBACK)
                            .into_owned()
                    });
                let title = if files > 1 {
                    keys::app_notifications::exported_many(
                        files as f64,
                        report.triangle_count as f64,
                    )
                } else {
                    keys::app_notifications::exported_one(name, report.triangle_count as f64)
                };
                // The notes describe the genuine losses (a level written as
                // triangles because a simplify rebuilt it, an animation target
                // that has no element in the file, an export before the source
                // capture landed), which the user should learn now rather than
                // when the file reaches an engine. They ride the success notice
                // as its body rather than one notice each: a chain of several
                // meshes has a note per mesh, and that used to bury the export's
                // own result under a column of near-identical boxes.
                self.notifications
                    .report(NoticeKind::Success, title, report.notes);
            }
            // A partial replacement is the one failure the user has to act on:
            // some of their assets on disk are now from this run and some are
            // not, so name them rather than leaving it to be discovered in an
            // engine.
            Err(OptError::ExportIncomplete {
                replaced,
                failed,
                reason,
            }) => {
                prof::msg(&format!(
                    "FBX export incomplete at {}: {reason}",
                    failed.display()
                ));
                let mut lines = vec![keys::app_notifications::export_incomplete_file(
                    crate::loading::file_label(&failed),
                    reason.clone(),
                )];
                if replaced.is_empty() {
                    lines.push(
                        review_localization::tr(keys::app_notifications::EXPORT_NOTHING_CHANGED)
                            .into_owned(),
                    );
                } else {
                    lines.push(
                        review_localization::tr(keys::app_notifications::EXPORT_REPLACED)
                            .into_owned(),
                    );
                    lines.extend(replaced.iter().map(|path| crate::loading::file_label(path)));
                }
                self.notifications.report(
                    NoticeKind::Error,
                    review_localization::tr(keys::app_notifications::EXPORT_INCOMPLETE)
                        .into_owned(),
                    lines,
                );
            }
            Err(error) => {
                prof::msg(&format!("FBX export failed: {error}"));
                self.notifications
                    .error(keys::app_notifications::export_failed(
                        crate::explain::explain_opt_error(&error),
                    ));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Store a successful run and mirror its measured figures into the UI.
    fn accept_opt_result(&mut self, result: ProcessedResult, level_bvhs: Vec<Arc<SceneBvh>>) {
        let warnings = result.warnings.clone();
        let levels: Vec<OptLevelView> = result
            .lods
            .iter()
            .map(|lod| OptLevelView {
                stats: lod.model.stats,
                metrics: lod.metrics,
            })
            .collect();
        let view = OptResultView {
            elapsed_ms: result.elapsed.as_secs_f32() * 1000.0,
            warnings: result.warnings.clone(),
            source: result.source,
            source_metrics: result.source_metrics,
            levels,
        };

        // A shorter chain than last time would leave the selection past the end.
        let level_count = view.levels.len();
        self.ui.opt.active_lod = self.ui.opt.active_lod.min(level_count.saturating_sub(1));
        // Until the user picks a level themselves, show the first *simplified*
        // one. Level 0 is the mesh the LOD operation simplifies from, so leaving
        // the viewport there shows something identical to the source and reads as
        // the operation having done nothing.
        if !self.ui.opt.lod_pinned {
            self.ui.opt.active_lod = usize::from(level_count > 1);
        }
        self.ui.opt.result = Some(view);

        // A run with nothing enabled produced figures but no mesh; the viewport
        // is showing the source alone, so the processed slot's buffers go
        // (invariant 3) — this is what makes disabling the last operation snap
        // straight back to the source mesh.
        if result.lods.is_empty()
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.release_processed_mesh();
        }

        let revision = self.next_model_revision();
        let stack_revision = self.ui.opt.stack_revision;
        let level = self.ui.opt.active_lod;
        if let Some(opt) = self.opt.as_mut() {
            opt.processed = Some(Arc::new(result));
            opt.level_bvhs = level_bvhs;
            opt.processed_revision = revision;
            opt.revision_level = level;
            opt.covers_revision = stack_revision;
        }

        // Warnings describe a *condition* of the mesh and the stack (skinning
        // dropped, a level that simplified away, a simplifier that can't collapse
        // this mesh), so they persist across runs while it holds — and dragging a
        // slider is a run per frame. Only what is newly true is announced;
        // otherwise one stuck condition buries the viewport in identical toasts.
        let fresh: Vec<String> = self
            .opt
            .as_ref()
            .map(|opt| {
                warnings
                    .iter()
                    .filter(|warning| !opt.announced_warnings.contains(*warning))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if let Some(opt) = self.opt.as_mut() {
            opt.announced_warnings = warnings;
        }
        if !fresh.is_empty() {
            // One notice for the run, however many things it has to say.
            self.notifications.report(
                NoticeKind::Warning,
                if fresh.len() == 1 {
                    review_localization::tr(keys::app_notifications::OPTIMIZATION_WARNING)
                        .into_owned()
                } else {
                    keys::app_notifications::optimization_warnings(fresh.len() as f64)
                },
                fresh,
            );
        }

        self.redraw.requested = true;
    }

    /// The processed mesh to draw, and its revision — `None` when there is no
    /// result yet, so the workspace falls back to the source mesh.
    ///
    /// Switching levels re-issues the revision: the renderer caches its mesh
    /// buffers by revision alone, so reusing one across two different meshes
    /// would leave the previous level's geometry on screen.
    pub(crate) fn opt_processed_revision(&mut self) -> Option<u64> {
        let level = self.ui.opt.active_lod;
        let stale = self
            .opt
            .as_ref()
            .is_some_and(|opt| opt.revision_level != level);
        if stale {
            let revision = self.next_model_revision();
            if let Some(opt) = self.opt.as_mut() {
                opt.processed_revision = revision;
                opt.revision_level = level;
            }
        }
        self.opt.as_ref().and_then(|opt| {
            opt.level_model(level)
                .is_some()
                .then_some(opt.processed_revision)
        })
    }

    /// Carry out an Opt action from the UI. These are the operations that reach
    /// outside the app — file dialogs and disk writes — which is why they travel
    /// as intents rather than being applied by the chrome itself (invariant 2).
    pub(crate) fn apply_opt_intent(&mut self, intent: OptIntent) {
        match intent {
            OptIntent::SavePreset => self.save_opt_preset(),
            OptIntent::LoadPreset => self.load_opt_preset(),
            OptIntent::Export => self.export_opt_result(),
        }
    }

    /// Ask where to save the operation stack as a preset.
    ///
    /// The stack is serialized before the picker opens, for the same reason the
    /// export captures its chain: the chrome keeps running behind the dialog, so
    /// the stack the user was looking at when they clicked Save is the one that
    /// gets written, not whatever it has become by the time they choose a name.
    fn save_opt_preset(&mut self) {
        // Per-object overrides address node *indices*, which mean nothing in
        // another file. Record the name of each one's object on the way out so
        // the preset can be rebound wherever it is loaded; the live stack keeps
        // addressing by index.
        let mut stack = (*self.ui.opt.stack).clone();
        stack.stamp_node_names(&self.scene_model.nodes);
        let json = match preset::to_json(&stack) {
            Ok(json) => json,
            Err(error) => {
                self.notifications
                    .error(keys::app_notifications::preset_build_failed(
                        crate::explain::explain_opt_error(&error),
                    ));
                return;
            }
        };
        self.ask(Dialog::SavePreset { json });
    }

    /// Write a serialized preset to the chosen path.
    ///
    /// Replaced rather than overwritten (`write_bytes_replacing`): saving over an
    /// existing preset truncates it before the first byte lands, so a failure
    /// halfway through would cost the user the setup they were replacing as well
    /// as the one they were saving.
    pub(crate) fn write_opt_preset(&mut self, path: &Path, json: &str) {
        match review_optimize::write_bytes_replacing(path, json) {
            Ok(()) => self
                .notifications
                .success(keys::app_notifications::preset_saved(
                    crate::loading::file_label(path),
                )),
            Err(error) => {
                prof::msg(&format!("preset save failed {}: {error}", path.display()));
                self.notifications
                    .error(keys::app_notifications::couldnt_save(
                        crate::loading::file_label(path),
                    ));
            }
        }
    }

    /// Ask for a preset to load.
    fn load_opt_preset(&mut self) {
        self.ask(Dialog::LoadPreset);
    }

    /// Apply a chosen preset file to the operation stack.
    pub(crate) fn read_opt_preset(&mut self, path: &Path) {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(error) => {
                prof::msg(&format!("preset read failed {}: {error}", path.display()));
                self.notifications
                    .error(keys::app_notifications::couldnt_read(
                        crate::loading::file_label(path),
                    ));
                return;
            }
        };
        let mut stack = match preset::from_json(&json) {
            Ok(stack) => stack,
            Err(error) => {
                self.notifications
                    .error(keys::app_notifications::preset_load_failed(
                        crate::explain::explain_opt_error(&error),
                    ));
                return;
            }
        };

        // A preset authored against another asset carries that asset's node
        // indices, which say nothing about *which* object was meant here. Each
        // override names its object, so match on that and drop what this model
        // has no unambiguous answer for.
        let report = stack.rebind_to_model(&self.scene_model.nodes);

        self.ui.opt.set_stack(Arc::new(stack));
        self.schedule_reprocess();
        self.redraw.requested = true;

        self.notifications
            .success(keys::app_notifications::preset_loaded(
                crate::loading::file_label(path),
            ));
        for line in rebind_notes(report) {
            self.notifications.warning(line);
        }
    }

    /// Drop every Opt result and queue a fresh run — called when the model is
    /// replaced, since the stored meshes and node overrides describe the old one.
    pub(crate) fn reset_opt_for_new_model(&mut self) {
        // Node overrides address the previous model's node indices. Keeping the
        // in-range ones would silently apply to whatever now sits at those
        // positions — a same-sized scene of unrelated objects passes every
        // bounds check there is. The operations describe the setup and stay.
        self.ui.opt.edit_stack(OptStack::clear_overrides);
        self.ui.opt.result = None;
        self.ui.opt.active_lod = 0;
        // A new model is a fresh comparison: the level the user pinned was a
        // choice about the old one's chain, which may not even have this length.
        self.ui.opt.lod_pinned = false;
        self.ui.opt.processing = false;

        if let Some(opt) = self.opt.as_mut() {
            // Bumping the generation invalidates any run still on the worker for
            // the previous model.
            opt.generation = opt.generation.saturating_add(1);
            opt.processed = None;
            opt.covers_revision = u64::MAX;
            opt.revision_level = usize::MAX;
            opt.dirty = false;
            opt.announced_warnings.clear();
            // The hidden set is cleared on model load (its node indices no
            // longer apply), so forget the old one rather than read a change
            // out of it.
            opt.seen_hidden.clear();
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.release_processed_mesh();
        }
        if self.ui.mode == review_ui::WorkspaceMode::Opt {
            self.schedule_reprocess();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OptSubsystem;

    /// A run that is merely *in flight* must leave the revision covered.
    ///
    /// Every `needs_run` that answers yes bumps the generation, and a result is
    /// only accepted while its generation is still current — so a rule that kept
    /// answering yes while the worker was busy would invalidate the very run it
    /// was waiting for, every frame, forever. That shipped once: the workspace
    /// pegged a core reprocessing and never drew a processed mesh.
    #[test]
    fn a_run_in_flight_is_not_rescheduled_every_frame() {
        let mut opt = OptSubsystem::default();
        // First frame in the workspace: the stack revision is new.
        assert!(opt.needs_run(7, true));
        // `spawn_process` marks the revision as covered as it starts the worker.
        opt.covers_revision = 7;
        assert!(!opt.needs_run(7, false), "the queued run already covers it");
    }

    /// A failed run leaves no result, but the revision stays covered: the error
    /// was reported, and retrying it every frame would only repeat the toast.
    #[test]
    fn a_failed_run_is_not_retried_forever() {
        let opt = OptSubsystem {
            covers_revision: 3,
            ..Default::default()
        };
        assert!(!opt.needs_run(3, false));
        // A fresh edit is a different matter — that always runs.
        assert!(opt.needs_run(4, true));
    }
}
