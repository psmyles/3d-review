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

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use review_model::{ModelData, SceneBvh};
use review_optimize::{OptStack, OptWarning, ProcessedResult};
use review_ui::{ActivityId, OptIntent};

// The processing loop, the export, and preset I/O, each in its own file; the
// subsystem they all work on is defined here.
mod export;
mod preset;
mod process;

pub(crate) use export::OptExported;
pub(crate) use process::{OptPreviewed, OptProcessed, OptProgressed};

use crate::dialog::Dialog;
use crate::events::UserEvent;
use crate::keys;
use crate::{App, prof};

/// How long a run may take before the user is told it is still going. Below this
/// the result usually lands within a frame or two and a toast would only flicker.
const ACTIVITY_NOTICE_AFTER: Duration = Duration::from_millis(300);

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
    ///
    /// Shared with the worker as an atomic rather than kept here alone, because
    /// bumping it is also what *cancels* the run in flight: the worker holds a
    /// [`CancelToken`] over this counter and stops as soon as it reads a value
    /// other than its own (`review_optimize::cancel`). One piece of state, so a
    /// superseded run cannot be live and cancelled at the same time.
    generation: Arc<AtomicU64>,
    /// The generation of the run currently on the worker thread, if any.
    in_flight: Option<u64>,
    /// An edit landed while a run was in flight, so another is owed.
    dirty: bool,
    started_at: Option<Instant>,
    /// Whether the "still working" toast is currently up.
    activity: Option<ActivityId>,
    /// The newest part-finished mesh, and the generation that produced it.
    ///
    /// Drawn in place of the finished level while a run is going, so a rebuild
    /// is watched settling rather than waited out. Cleared when that run lands,
    /// and never consulted for a *measurement*: the stats card keeps showing
    /// the last completed run, because a half-finished count is worse than a
    /// slightly old one.
    pub(crate) preview: Option<(u64, Arc<ModelData>)>,
    /// The newest progress line, kept whether or not the toast is up yet.
    ///
    /// Reports start arriving the moment the run does, but the toast is
    /// deliberately withheld for [`ACTIVITY_NOTICE_AFTER`] - and an operation
    /// that rebuilds one object at a time can be minutes between reports. So the
    /// line is remembered and written into the card as it is raised; otherwise
    /// the card that finally appears is the one with a blank stage line under it
    /// for the whole of the step the user is actually waiting on.
    last_progress: Option<(String, Option<f32>)>,
    /// The warnings the last accepted run reported, so a condition that persists
    /// across runs is announced once rather than once per run.
    announced_warnings: Vec<OptWarning>,
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
            generation: Arc::new(AtomicU64::new(0)),
            in_flight: None,
            dirty: false,
            started_at: None,
            activity: None,
            preview: None,
            last_progress: None,
            announced_warnings: Vec::new(),
            seen_hidden: Vec::new(),
        }
    }
}

impl OptSubsystem {
    /// The mesh for `level` of the latest result, if there is one.
    pub(crate) fn level_model(&self, level: usize) -> Option<&ModelData> {
        if let Some(model) = self.preview_of(level) {
            return Some(model);
        }
        self.processed
            .as_ref()
            .and_then(|result| result.lod(level))
            .map(|lod| &lod.model)
    }

    /// The part-finished mesh for `level`, if a run is showing one.
    ///
    /// Level 0 only: a preview is the mesh the stack has built so far, which is
    /// what level 0 becomes. The LOD fan-out has not happened yet, so standing
    /// in for a simplified level would show the wrong density entirely.
    fn preview_of(&self, level: usize) -> Option<&ModelData> {
        match &self.preview {
            Some((_, model)) if level == 0 => Some(model),
            _ => None,
        }
    }

    /// [`Self::preview_of`] as an owned handle, which the renderer's borrow of
    /// `self` can outlive.
    pub(crate) fn level_mesh(&self, level: usize) -> Option<Arc<ModelData>> {
        match &self.preview {
            Some((_, model)) if level == 0 => Some(Arc::clone(model)),
            _ => None,
        }
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
            // the previous model - and, since the worker's token reads this same
            // counter, stops it rather than letting it finish against a mesh
            // that has been replaced.
            opt.generation.fetch_add(1, Ordering::Relaxed);
            opt.preview = None;
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
