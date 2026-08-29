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
//! actually enters the Opt workspace: `App::opt` is `None` until then, and no
//! run is scheduled for a stack with nothing enabled in it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use review_model::ModelData;
use review_optimize::{
    ExportReport, OptError, ProcessInput, ProcessedResult, export_fbx, preset, process,
};
use review_ui::{OptIntent, OptLevelView, OptResultView};

use crate::{App, UserEvent, prof};

/// How long a run may take before the user is told it is still going. Below this
/// the result usually lands within a frame or two and a toast would only flicker.
const ACTIVITY_NOTICE_AFTER: Duration = Duration::from_millis(300);

/// A finished run, posted from the worker thread back to the event loop.
#[derive(Debug)]
pub(crate) struct OptProcessed {
    /// The generation this run was started for. A result whose generation is no
    /// longer the current one is stale and dropped.
    pub(crate) generation: u64,
    pub(crate) result: Result<ProcessedResult, OptError>,
}

/// Everything the Opt workspace needs on the app side. Created on first entry
/// into the workspace, so a session that never opens it pays nothing.
#[derive(Debug)]
pub(crate) struct OptSubsystem {
    /// The most recent completed run, shared by `Arc` so the render path can
    /// borrow a level's mesh without copying it.
    pub(crate) processed: Option<Arc<ProcessedResult>>,
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
}

impl Default for OptSubsystem {
    fn default() -> Self {
        Self {
            processed: None,
            processed_revision: 0,
            revision_level: usize::MAX,
            covers_revision: u64::MAX,
            seen_revision: u64::MAX,
            generation: 0,
            in_flight: None,
            dirty: false,
            started_at: None,
            activity_shown: false,
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
        let opt = self.opt.get_or_insert_with(OptSubsystem::default);

        // First visit: adopt the current stack revision without treating it as an
        // edit, so merely opening the workspace with an empty stack schedules
        // nothing.
        let changed = opt.seen_revision != stack_revision;
        opt.seen_revision = stack_revision;

        if opt.needs_run(stack_revision, changed) {
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
            self.notifications.begin_activity("Optimizing mesh…");
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
        let stack_revision = self.ui.opt.stack_revision;

        // Nothing to do: an empty (or entirely disabled) stack would process to a
        // copy of the source, which is exactly what the workspace already shows.
        // Clearing any previous result here is what makes disabling the last
        // operation snap straight back to the source mesh.
        let has_work = stack.ops.iter().any(|op| op.enabled);
        if !has_work || model.indices.is_empty() {
            let Some(opt) = self.opt.as_mut() else {
                return;
            };
            opt.dirty = false;
            opt.processed = None;
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
            self.notifications
                .error("Couldn't start mesh optimization".to_owned());
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

        std::thread::spawn(move || {
            prof::thread_name("mesh-optimize");
            let result = {
                let _z = prof::zone!("Optimize Mesh");
                process(ProcessInput {
                    model: &model,
                    stack: &stack,
                    render_vertex_size,
                })
            };
            // A send failure only means the event loop has exited.
            let _ = proxy.send_event(UserEvent::OptProcessed(Box::new(OptProcessed {
                generation,
                result,
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
                Ok(result) => self.accept_opt_result(result),
                Err(error) => {
                    prof::msg(&format!("mesh optimization failed: {error}"));
                    self.notifications
                        .error(format!("Optimization failed: {error}"));
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

    /// Write the current LOD chain to disk.
    ///
    /// The write runs on a worker like processing does: a large chain in ASCII is
    /// slow enough to drop frames, and freezing the window mid-export is exactly
    /// the failure the workspace is built to avoid.
    fn export_opt_result(&mut self) {
        let Some(result) = self.opt.as_ref().and_then(|opt| opt.processed.clone()) else {
            self.notifications
                .error("Nothing to export yet — add an operation to the stack.".to_owned());
            return;
        };

        // A sensible default file name: the model's own, which is also the stem
        // the per-LOD packaging appends its suffixes to.
        let stem = if self.scene_model.name.is_empty() {
            "optimized".to_owned()
        } else {
            self.scene_model.name.clone()
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export optimized mesh")
            .add_filter("FBX", &["fbx"])
            .set_file_name(format!("{stem}.fbx"))
            .save_file()
        else {
            return;
        };

        let Some(proxy) = self.textures.proxy.clone() else {
            prof::msg("no event-loop proxy; cannot export off-thread");
            self.notifications
                .error("Couldn't start the export".to_owned());
            return;
        };

        let source = Arc::clone(&self.scene_model);
        let options = self.ui.opt.stack.export;
        self.notifications.begin_activity("Exporting FBX…");

        std::thread::spawn(move || {
            prof::thread_name("mesh-export");
            let outcome = {
                let _z = prof::zone!("Export FBX");
                export_fbx(&result.lods, &source, &path, &options)
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
                    .map(|path| crate::file_label(path))
                    .unwrap_or_else(|| "the mesh".to_owned());
                self.notifications.success(if files > 1 {
                    format!(
                        "Exported {files} files ({} triangles)",
                        report.triangle_count
                    )
                } else {
                    format!("Exported {name} ({} triangles)", report.triangle_count)
                });
                // The notes describe what the format could not carry (untextured
                // materials, dropped skinning), which the user should learn now
                // rather than when the file reaches an engine.
                for note in report.notes {
                    self.notifications.info(note);
                }
            }
            Err(error) => {
                prof::msg(&format!("FBX export failed: {error}"));
                self.notifications.error(format!("Export failed: {error}"));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Store a successful run and mirror its measured figures into the UI.
    fn accept_opt_result(&mut self, result: ProcessedResult) {
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
            levels,
        };

        // A shorter chain than last time would leave the selection past the end.
        let level_count = view.levels.len();
        self.ui.opt.active_lod = self.ui.opt.active_lod.min(level_count.saturating_sub(1));
        self.ui.opt.result = Some(view);

        let revision = self.next_model_revision();
        let stack_revision = self.ui.opt.stack_revision;
        let level = self.ui.opt.active_lod;
        if let Some(opt) = self.opt.as_mut() {
            opt.processed = Some(Arc::new(result));
            opt.processed_revision = revision;
            opt.revision_level = level;
            opt.covers_revision = stack_revision;
        }

        // Warnings describe the run as a whole (skinning dropped, a level that
        // simplified away), so they surface once per run rather than per frame.
        let warnings = self
            .ui
            .opt
            .result
            .as_ref()
            .map(|view| view.warnings.clone())
            .unwrap_or_default();
        for warning in warnings {
            self.notifications.info(warning);
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

    fn save_opt_preset(&mut self) {
        let json = match preset::to_json(&self.ui.opt.stack) {
            Ok(json) => json,
            Err(error) => {
                self.notifications
                    .error(format!("Couldn't build the preset: {error}"));
                return;
            }
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save optimization preset")
            .add_filter("Optimization preset", &[preset::PRESET_EXTENSION])
            .set_file_name("optimization-preset.json")
            .save_file()
        else {
            return;
        };

        match std::fs::write(&path, json) {
            Ok(()) => self
                .notifications
                .success(format!("Saved {}", crate::file_label(&path))),
            Err(error) => {
                prof::msg(&format!("preset save failed {}: {error}", path.display()));
                self.notifications
                    .error(format!("Couldn't save {}", crate::file_label(&path)));
            }
        }
    }

    fn load_opt_preset(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load optimization preset")
            .add_filter("Optimization preset", &[preset::PRESET_EXTENSION])
            .pick_file()
        else {
            return;
        };

        let json = match std::fs::read_to_string(&path) {
            Ok(json) => json,
            Err(error) => {
                prof::msg(&format!("preset read failed {}: {error}", path.display()));
                self.notifications
                    .error(format!("Couldn't read {}", crate::file_label(&path)));
                return;
            }
        };
        let mut stack = match preset::from_json(&json) {
            Ok(stack) => stack,
            Err(error) => {
                self.notifications
                    .error(format!("Couldn't load that preset: {error}"));
                return;
            }
        };

        // A preset authored against another asset carries that asset's node
        // indices; keeping them would apply its exclusions to whatever now
        // occupies those positions.
        let dropped = stack.overrides.len();
        stack.clamp_to_model(self.scene_model.nodes.len());
        let dropped = dropped - stack.overrides.len();

        self.ui.opt.set_stack(Arc::new(stack));
        self.schedule_reprocess();
        self.redraw.requested = true;

        self.notifications
            .success(format!("Loaded {}", crate::file_label(&path)));
        if dropped > 0 {
            self.notifications.info(format!(
                "{dropped} per-object override{} referenced objects this model \
                 doesn't have, and {} dropped.",
                if dropped == 1 { "" } else { "s" },
                if dropped == 1 { "was" } else { "were" },
            ));
        }
    }

    /// Drop every Opt result and queue a fresh run — called when the model is
    /// replaced, since the stored meshes and node overrides describe the old one.
    pub(crate) fn reset_opt_for_new_model(&mut self) {
        let node_count = self.scene_model.nodes.len();
        // Node overrides address the previous model's node indices; keeping them
        // would silently apply to whatever now sits at those positions.
        self.ui
            .opt
            .edit_stack(|stack| stack.clamp_to_model(node_count));
        self.ui.opt.result = None;
        self.ui.opt.active_lod = 0;
        self.ui.opt.processing = false;

        if let Some(opt) = self.opt.as_mut() {
            // Bumping the generation invalidates any run still on the worker for
            // the previous model.
            opt.generation = opt.generation.saturating_add(1);
            opt.processed = None;
            opt.covers_revision = u64::MAX;
            opt.revision_level = usize::MAX;
            opt.dirty = false;
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
