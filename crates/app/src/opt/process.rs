//! The processing loop: scheduling a run for each edit to the stack,
//! the worker that runs it, and folding its progress, previews and result back
//! into the UI.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use review_model::{ModelData, SceneBvh};
use review_optimize::{
    CancelToken, OpKind, OptError, OptPreview, OptProgress, OptStack, OptStage, OptWarning,
    ProcessInput, ProcessedResult, process_progressive,
};

use review_ui::{NoticeKind, OptLevelView, OptResultView};

use super::*;

/// Least time between two previews reaching the event loop.
///
/// A preview is a whole mesh: it costs an assemble on the worker and a buffer
/// upload on the main thread, and beyond a few a second the eye cannot tell.
const PREVIEW_MIN_INTERVAL: Duration = Duration::from_millis(250);

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

/// A mesh from part way through a run, posted from the worker thread.
///
/// Boxed at the event, because it carries a whole `ModelData`.
#[derive(Debug)]
pub(crate) struct OptPreviewed {
    /// Matched against the current generation exactly as [`OptProcessed`] is.
    pub(crate) generation: u64,
    pub(crate) model: ModelData,
}

/// A run reporting where it has got to, posted from the worker thread.
#[derive(Debug)]
pub(crate) struct OptProgressed {
    /// Matched against the current generation exactly as [`OptProcessed`] is, so
    /// a superseded run can't retitle the notice of the one that replaced it.
    pub(crate) generation: u64,
    /// The finished stage line, built on the worker: `Remesh: leaf_012 (3 of 13)`.
    pub(crate) message: String,
    /// How far through that step, when it can be known - the card's bar.
    pub(crate) fraction: Option<f32>,
}

/// The stage line under the optimizing card's title.
///
/// Rendered on the worker, like the import's, so the main thread only ever
/// swaps a finished string into the card. What a stage is *called* comes from
/// the chrome's catalog (`review_ui::op_kind_name` for an operation), so the
/// notice and the stack row can't drift apart.
fn progress_message(progress: OptProgress<'_>) -> String {
    let stage = match progress.op {
        Some(kind) => review_ui::op_kind_name(kind),
        None => review_localization::tr(stage_name(progress.stage)).into_owned(),
    };
    match (progress.object, progress.total) {
        (Some(object), total) if total > 0 => keys::app_notifications::opt_stage_object(
            f64::from(progress.done + 1),
            object.to_owned(),
            stage,
            f64::from(total),
        ),
        (None, total) if total > 0 => keys::app_notifications::opt_stage_count(
            f64::from(progress.done + 1),
            stage,
            f64::from(total),
        ),
        _ => keys::app_notifications::opt_stage(stage),
    }
}

/// What the optimizing card calls a stage that isn't one of the user's
/// operations. `optimize` keeps its own English `label()` for the profiling
/// channel, so the map lives here, on the side that draws text (invariant 12).
fn stage_name(stage: OptStage) -> review_localization::Key {
    match stage {
        OptStage::Preparing => keys::app_notifications::OPT_STAGE_PREPARING,
        OptStage::Measuring => keys::app_notifications::OPT_STAGE_MEASURING,
        // An operation is named by `op_kind_name` above; this is only reached by
        // a report that somehow carried no operation with it.
        OptStage::Operation => keys::app_notifications::OPTIMIZING,
        OptStage::Assembling => keys::app_notifications::OPT_STAGE_ASSEMBLING,
    }
}

/// One line describing a finished run: how long it took, the operations that
/// ran, and each level's counts against the source's - the same figures, and
/// the same baseline, the processed stats card shows.
fn run_summary(stack: &OptStack, result: &ProcessedResult) -> String {
    let operations: Vec<&str> = stack
        .ops
        .iter()
        .filter(|op| op.enabled)
        .map(|op| op.kind.label())
        .collect();
    let source = result.source;
    let mut line = format!(
        "optimized in {:.0} ms [{}]",
        result.elapsed.as_secs_f64() * 1000.0,
        operations.join(", ")
    );
    if !stack.overrides.is_empty() {
        line.push_str(&format!(
            " with {} per-object overrides",
            stack.overrides.len()
        ));
    }
    line.push_str(&format!(
        ": source {} tris / {} verts",
        source.triangles, source.vertices
    ));
    for lod in &result.lods {
        let stats = lod.model.stats;
        line.push_str(&format!(
            "; LOD{} {} tris ({}) / {} verts ({})",
            lod.level,
            stats.triangle_count,
            percent_change(source.triangles, stats.triangle_count),
            stats.vertex_count,
            percent_change(source.vertices, stats.vertex_count),
        ));
    }
    line
}

/// `-50.0%`, or `n/a` against a zero baseline.
fn percent_change(before: usize, after: usize) -> String {
    if before == 0 {
        return "n/a".to_owned();
    }
    let change = (after as f64 - before as f64) / before as f64 * 100.0;
    format!("{change:+.1}%")
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
            && opt.activity.is_none()
            && started.elapsed() >= ACTIVITY_NOTICE_AFTER
        {
            let activity = self.notifications.begin_activity(
                review_localization::tr(keys::app_notifications::OPTIMIZING).into_owned(),
            );
            opt.activity = Some(activity);
            // The step the run is already on, rather than a blank line until the
            // next report - which on a per-object operation is a long way off.
            if let Some((message, fraction)) = opt.last_progress.clone() {
                self.notifications
                    .update_activity(activity, message, fraction);
            }
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
        // The bump both marks the newer request and cancels the older run: an
        // edit arriving mid-run used to wait out a whole rebuild nobody would
        // see before the run replacing it could start, which on a Remesh is
        // tens of seconds per slider nudge.
        opt.generation.fetch_add(1, Ordering::Relaxed);
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
            log::error!("no event-loop proxy; cannot optimize off-thread");
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
        let generation = opt.generation.load(Ordering::Relaxed);
        let cancel = CancelToken::new(Arc::clone(&opt.generation), generation);
        opt.in_flight = Some(generation);
        opt.dirty = false;
        opt.covers_revision = stack_revision;
        opt.started_at = Some(Instant::now());
        opt.last_progress = None;

        // The renderer's own vertex size, so the reported overfetch describes the
        // buffer the GPU actually reads rather than the optimizer's intermediate
        // layout (invariant 5: a measured figure, of the real thing).
        let render_vertex_size = review_render::scene_vertex_size();
        // Snapshot of the Outliner-hidden nodes, read only by the AO bake.
        let hidden = self.ui.hidden_mesh_nodes();

        let progress_proxy = proxy.clone();
        std::thread::spawn(move || {
            prof::thread_name("mesh-optimize");
            // Filtered on the line having *changed*, and deliberately not on a
            // minimum interval the way the import's is. The import reports from
            // inside the parse, many times per whole percent, so dropping one
            // costs nothing - another is 512 KB away. A run reports at step and
            // object boundaries, and the next one can be a whole field solve
            // away: a timer that dropped the report saying which object we are
            // on would leave the card reading `Preparing the mesh...` for the
            // minutes that object took. The volume is bounded by the stack times
            // the scene, which the loop can take all of. `RefCell`, not a lock -
            // the sink is only ever called from this thread, from inside the run.
            let last_line = std::cell::RefCell::new(String::new());
            let report = |progress: OptProgress<'_>| {
                let message = progress_message(progress);
                if *last_line.borrow() == message {
                    return;
                }
                last_line.replace(message.clone());
                let _ = progress_proxy.send_event(UserEvent::OptProgressed(OptProgressed {
                    generation,
                    message,
                    fraction: progress.fraction(),
                }));
            };
            // Self-throttled: a preview costs an assemble of the whole level,
            // and a scene of small objects can produce them faster than the
            // event loop can draw them. Skipping one only delays what is shown,
            // never what is produced.
            let last_preview = std::cell::Cell::new(None::<Instant>);
            let show = |preview: OptPreview| {
                if last_preview
                    .get()
                    .is_some_and(|sent| sent.elapsed() < PREVIEW_MIN_INTERVAL)
                {
                    return;
                }
                last_preview.set(Some(Instant::now()));
                let _ =
                    progress_proxy.send_event(UserEvent::OptPreviewed(Box::new(OptPreviewed {
                        generation,
                        model: preview.model,
                    })));
            };
            let result = {
                let _z = prof::zone!("Optimize Mesh");
                process_progressive(
                    ProcessInput {
                        model: &model,
                        stack: &stack,
                        render_vertex_size,
                        hidden_nodes: &hidden,
                        extras: extras.as_deref(),
                    },
                    &report,
                    Some(&show),
                    Some(&cancel),
                )
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
        opt.last_progress = None;
        // Whatever this run was showing is superseded by what it produced —
        // including when it produced an error, where the source alone is the
        // honest thing to draw.
        opt.preview = None;
        // An edit that arrived mid-run is already marked, and it is what decides
        // whether the card comes down: a drag now *cancels* each run rather than
        // queueing behind it, so ending the activity here would blink the card
        // off and on for every frame of the drag. It stays up, and the run about
        // to be spawned inherits it.
        let owed = opt.dirty;
        if !owed && let Some(activity) = opt.activity.take() {
            self.notifications.end_activity(activity);
        }

        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        // A result for a superseded stack is dropped: the newer run is already
        // queued and showing this one would flash a mesh the user has moved past.
        let current = message.generation == opt.generation.load(Ordering::Relaxed);
        if current {
            match message.result {
                Ok(result) => self.accept_opt_result(result, message.level_bvhs),
                // A cancelled run is not a failed one - it stopped because the
                // user moved on, which is the feature working. It cannot reach
                // here with a current generation (cancelling *is* superseding),
                // so this arm only guards against saying "optimization failed"
                // if that ever stops being true.
                Err(OptError::Cancelled) => log::debug!("mesh optimization cancelled"),
                Err(error) => {
                    log::error!("mesh optimization failed: {error}");
                    self.notifications
                        .error(keys::app_notifications::optimization_failed(
                            crate::explain::explain_opt_error(&error),
                        ));
                }
            }
        }

        // Edits that arrived mid-run are honoured now, with the latest stack.
        if owed {
            self.spawn_process();
        }
        // The card was held up for a run that `spawn_process` then declined to
        // start (an empty model, or no event-loop proxy). Nothing will end it,
        // so end it here: every `begin_activity` has to be balanced exactly once.
        let idle = self.opt.as_ref().is_some_and(|opt| opt.in_flight.is_none());
        if idle
            && let Some(opt) = self.opt.as_mut()
            && let Some(activity) = opt.activity.take()
        {
            self.notifications.end_activity(activity);
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Rewrite the optimizing card's stage line from a worker's progress report.
    /// A report from a superseded run is dropped, exactly as its result would be.
    pub(crate) fn handle_opt_progressed(&mut self, message: OptProgressed) {
        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        if message.generation != opt.generation.load(Ordering::Relaxed) {
            return;
        }
        opt.last_progress = Some((message.message.clone(), message.fraction));
        // Only rewrites this run's own card, and only once it is up: below
        // `ACTIVITY_NOTICE_AFTER` there is none, and the line above is what the
        // card is raised with.
        if let Some(activity) = opt.activity {
            self.notifications
                .update_activity(activity, message.message, message.fraction);
        }
        self.request_redraw();
    }

    /// Show a mesh from part way through a run.
    ///
    /// Touches none of the run's bookkeeping — not `in_flight`, not `dirty`,
    /// not the activity card — so the respawn logic and the "still working"
    /// notice carry on exactly as they would have. A preview from a superseded
    /// run is dropped, as its result would be.
    pub(crate) fn handle_opt_previewed(&mut self, message: OptPreviewed) {
        let revision = self.next_model_revision();
        let Some(opt) = self.opt.as_mut() else {
            return;
        };
        if message.generation != opt.generation.load(Ordering::Relaxed) {
            return;
        }
        opt.preview = Some((message.generation, Arc::new(message.model)));
        // The renderer caches its mesh buffers by revision, so a fresh one is
        // what makes the new mesh reach the GPU.
        opt.processed_revision = revision;
        self.request_redraw();
    }

    /// Store a successful run and mirror its measured figures into the UI.
    fn accept_opt_result(&mut self, result: ProcessedResult, level_bvhs: Vec<Arc<SceneBvh>>) {
        // Only a run that lands is logged: during a slider drag every run but
        // the last is cancelled, and a line per cancellation would bury the one
        // that describes what is on screen. Its generation is current, so the
        // stack in the UI is the stack it ran.
        log::info!("{}", run_summary(&self.ui.opt.stack, &result));
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
            warnings: result
                .warnings
                .iter()
                .map(crate::explain::explain_opt_warning)
                .collect(),
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
        let fresh: Vec<OptWarning> = self
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
            for warning in &fresh {
                // The English diagnostic, for the log; the card shows the catalog's.
                log::warn!("optimization: {warning}");
            }
            let fresh: Vec<String> = fresh
                .iter()
                .map(crate::explain::explain_opt_warning)
                .collect();
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
}
