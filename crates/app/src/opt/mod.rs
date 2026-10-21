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
    /// Replaced wholesale with each run and dropped with the result, so it can
    /// never describe a mesh that is no longer on screen. Read through
    /// [`Self::level_bvh`], which also knows when a preview stands in.
    level_bvhs: Vec<Arc<SceneBvh>>,
    /// Model revision of the finished level currently being drawn. Drawn from
    /// the app's shared revision counter so it can never collide with a source
    /// model's. A preview carries its own ([`OptPreviewMesh::revision`]), so showing
    /// one and then letting it go never leaves this number attached to it.
    level_revision: u64,
    /// Which level `level_revision` was issued for, so switching levels
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
    /// [`review_optimize::CancelToken`] over this counter and stops as soon as it
    /// reads a value other than its own (`review_optimize::cancel`). One piece of state, so a
    /// superseded run cannot be live and cancelled at the same time.
    generation: Arc<AtomicU64>,
    /// The generation of the run currently on the worker thread, if any.
    in_flight: Option<u64>,
    /// An edit landed while a run was in flight, so another is owed.
    dirty: bool,
    started_at: Option<Instant>,
    /// Whether the "still working" toast is currently up.
    activity: Option<ActivityId>,
    /// The newest part-finished mesh.
    ///
    /// Drawn in place of the finished level while a run is going, so a rebuild
    /// is watched settling rather than waited out. Cleared when that run lands,
    /// and never consulted for a *measurement*: the stats card keeps showing
    /// the last completed run, because a half-finished count is worse than a
    /// slightly old one.
    preview: Option<OptPreviewMesh>,
    /// Whether the newest run was started without the source-property capture,
    /// because it had not landed yet. Such a run has no polygons, extra skin
    /// layers or colour sets to carry, so once the capture lands it is run
    /// again ([`Self::capture_owed`]), and until then its result is exported
    /// without the capture rather than beside one it never saw.
    ran_without_capture: bool,
    /// Whether [`Self::processed`] came from such a run.
    processed_without_capture: bool,
    /// The capture has landed since the newest run started without it, so a
    /// run with it is owed.
    capture_owed: bool,
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
            level_revision: 0,
            revision_level: usize::MAX,
            covers_revision: u64::MAX,
            seen_revision: u64::MAX,
            generation: Arc::new(AtomicU64::new(0)),
            in_flight: None,
            dirty: false,
            started_at: None,
            activity: None,
            preview: None,
            ran_without_capture: false,
            processed_without_capture: false,
            capture_owed: false,
            last_progress: None,
            announced_warnings: Vec::new(),
            seen_hidden: Vec::new(),
        }
    }
}

/// A part-finished mesh a run published, and the revision it is drawn under.
#[derive(Debug)]
struct OptPreviewMesh {
    model: Arc<ModelData>,
    /// Issued for this mesh alone. The renderer keys its mesh buffers on the
    /// revision, so the finished level behind the preview keeps its own number
    /// and is uploaded again when the preview goes — whether the run finished,
    /// failed or was cancelled — rather than the preview staying on screen
    /// under a number that now names the finished level.
    revision: u64,
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
        self.shown_preview(level)
            .map(|preview| preview.model.as_ref())
    }

    /// The preview drawn for `level`, if any — the one rule [`Self::preview_of`],
    /// [`Self::level_mesh`] and [`Self::drawn_revision`] share, so the mesh drawn
    /// and the revision it is drawn under cannot disagree.
    fn shown_preview(&self, level: usize) -> Option<&OptPreviewMesh> {
        self.preview.as_ref().filter(|_| level == 0)
    }

    /// [`Self::preview_of`] as an owned handle, which the renderer's borrow of
    /// `self` can outlive.
    pub(crate) fn level_mesh(&self, level: usize) -> Option<Arc<ModelData>> {
        self.shown_preview(level)
            .map(|preview| Arc::clone(&preview.model))
    }

    /// The triangle index of the mesh drawn for `level`.
    ///
    /// `None` while a preview stands in for it: a part-finished mesh is never
    /// indexed, and the finished level's index describes a different mesh, so
    /// answering with it would test a pick or a label against triangles that are
    /// not on screen.
    pub(crate) fn level_bvh(&self, level: usize) -> Option<&Arc<SceneBvh>> {
        if self.preview_of(level).is_some() {
            return None;
        }
        self.level_bvhs.get(level)
    }

    /// The revision of the mesh drawn for `level`, or `None` when there is none.
    /// A preview is drawn under its own; the finished level under
    /// [`Self::level_revision`], which the caller has issued for `level`.
    fn drawn_revision(&self, level: usize) -> Option<u64> {
        if let Some(preview) = self.shown_preview(level) {
            return Some(preview.revision);
        }
        self.processed
            .as_ref()
            .and_then(|result| result.lod(level))
            .map(|_| self.level_revision)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether a run should be queued for `stack_revision` this frame: the UI
    /// just edited the stack, the source capture has landed since the newest
    /// run started without it, or there is no result and nothing has been asked
    /// to produce one (a run that errored leaves the revision covered, so a
    /// failure is reported once rather than retried forever).
    fn needs_run(&self, stack_revision: u64, changed: bool) -> bool {
        changed
            || self.capture_owed
            || (self.processed.is_none() && self.covers_revision != stack_revision)
    }

    /// Whether an export of [`Self::processed`] should be handed the source
    /// capture: only when the run that produced it had it. A chain processed
    /// before the capture landed carries none of the polygons the capture
    /// describes, and written beside it every mesh would come out as triangles
    /// with a note blaming the stack; written without it, the export says what
    /// actually happened.
    pub(crate) fn processed_with_capture(&self) -> bool {
        !self.processed_without_capture
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

        let stack_revision = self.ui.opt.stack_revision;
        if let Some(opt) = self.opt.as_mut() {
            // Bumping the generation invalidates any run still on the worker for
            // the previous model - and, since the worker's token reads this same
            // counter, stops it rather than letting it finish against a mesh
            // that has been replaced.
            opt.generation.fetch_add(1, Ordering::Relaxed);
            opt.preview = None;
            opt.processed = None;
            // The old model's level indices go with its result rather than stay
            // resident until the next run lands.
            opt.level_bvhs.clear();
            opt.covers_revision = u64::MAX;
            opt.revision_level = usize::MAX;
            opt.dirty = false;
            opt.ran_without_capture = false;
            opt.processed_without_capture = false;
            opt.capture_owed = false;
            opt.announced_warnings.clear();
            // The override clear above is not an edit to react to: the run below
            // (or the one entering the workspace starts) already covers it, and
            // seeing it as one next frame would cancel that run to start the
            // same one again.
            opt.seen_revision = stack_revision;
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

    /// A run started before the source capture landed — what a load in the
    /// workspace always starts — carries none of it, and its export would write
    /// every mesh as triangles. The capture landing makes a run owed; one that
    /// already had a capture owes nothing.
    #[test]
    fn a_run_without_the_capture_runs_again_once_it_lands() {
        let mut app = crate::App::default();
        let revision = app.ui.opt.stack_revision;
        app.opt = Some(OptSubsystem {
            covers_revision: revision,
            ran_without_capture: true,
            ..Default::default()
        });
        let needs_run = |app: &crate::App| {
            app.opt
                .as_ref()
                .is_some_and(|opt| opt.needs_run(revision, false))
        };
        assert!(!needs_run(&app), "the run in flight covers the revision");
        app.opt_capture_landed();
        assert!(needs_run(&app), "the capture landing owes a run");

        app.opt = Some(OptSubsystem {
            covers_revision: revision,
            ..Default::default()
        });
        app.opt_capture_landed();
        assert!(
            !needs_run(&app),
            "a run that had the capture is not repeated"
        );
    }

    /// A chain processed without the capture is exported without it, so the
    /// file says the properties had not loaded rather than blaming the stack
    /// for polygons it never saw.
    #[test]
    fn a_chain_processed_without_the_capture_is_exported_without_it() {
        let opt = OptSubsystem {
            processed_without_capture: true,
            ..Default::default()
        };
        assert!(!opt.processed_with_capture());
        assert!(OptSubsystem::default().processed_with_capture());
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
