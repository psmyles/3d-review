//! What a load measures after it is on screen, on a worker thread of its own.
//!
//! **The model is published as soon as it is drawable, then measured.** A
//! request's parse stops at the geometry, materials, scene graph, rest-pose
//! bounds and tangents — everything the first frame needs — and what it leaves
//! out is made here, against the `Arc` already on screen, each result posted as
//! its own [`ModelMeasured`] and folded into the UI where it belongs: the source
//! capture, the pick BVH, each clip's motion envelope, and the per-draw-group
//! stats table. On a 2.8M-triangle scene that is the mesh on screen in ~1.8 s
//! instead of ~5 s. Clips attached from other files are measured here too, on
//! the model they play on.

use std::sync::Arc;
use std::time::Instant;

use review_import::{
    CancelToken, ImportError, ImportProgress, ImportStage, PendingExtras, measure_clip_bounds,
    measure_clip_bounds_of,
};
use review_model::{AttachedAnimation, Bounds, MeshGroupStats, ModelData, SceneBvh, SourceExtras};
use review_ui::ActivityId;

use super::parse::{ModelLoadProgress, StageClock, progress_message};
use crate::events::UserEvent;
use crate::{App, keys, prof};

/// What one load leaves to be measured once it is on screen.
pub(super) struct MeasureJob {
    /// The measure generation of the model on screen (`App::measure_generation`)
    /// — not the load generation, which a request bumps before it is known to
    /// succeed.
    pub(super) generation: u64,
    /// The loading card the request opened: the job's last message closes it.
    pub(super) activity: ActivityId,
    pub(super) model: Arc<ModelData>,
    /// `Some` when [`Self::model`] has just been put on screen: its source
    /// capture, pick BVH, own clip envelopes and stats table are measured too.
    pub(super) new_model: Option<PendingExtras>,
    /// Attached files whose clip envelopes to measure, by their file index.
    pub(super) attached: Vec<(usize, Arc<AttachedAnimation>)>,
    /// For the log line that says when the worker was done.
    pub(super) started: Instant,
}

/// The source-property capture, marshaled on the worker right after the model
/// was published. `Ok(None)` when this build cannot capture; `Err` when the
/// capture did not describe the model (nothing is kept, and the export says so).
#[derive(Debug)]
pub(crate) struct SourceExtrasReady {
    /// Matched against the current measure generation exactly as
    /// [`ModelMeasured`] is.
    generation: u64,
    extras: Result<Option<Arc<SourceExtras>>, ImportError>,
}

/// One of the measurements a load defers until after it is drawn.
#[derive(Debug)]
pub(crate) enum ModelMeasurement {
    /// The model's own clips' motion envelopes, parallel to
    /// `ModelData::animations`.
    ClipBounds(Vec<Option<Bounds>>),
    /// An attached file's clip envelopes, on the model they play on.
    AttachedClipBounds {
        file: usize,
        bounds: Vec<Option<Bounds>>,
    },
    /// The per-(node, material) table behind the `GPU Verts` row and the stats
    /// card's scoped columns.
    MeshGroups(Vec<MeshGroupStats>),
    /// The triangle BVH the viewport pick and the dimension labels query.
    SceneBvh(Arc<SceneBvh>),
    /// The job is done: the loading card comes down. Exactly one message per
    /// job ends it, this or [`Self::Cancelled`].
    Finished,
    /// The job stopped because its load was superseded. Carries nothing to apply;
    /// it exists so the loading card still comes down.
    Cancelled,
}

/// A measurement arriving from a measuring worker.
#[derive(Debug)]
pub(crate) struct ModelMeasured {
    /// Matched against the current measure generation, so a measurement of a
    /// model no longer on screen is dropped.
    generation: u64,
    measurement: ModelMeasurement,
    /// The loading card this job's request opened.
    activity: ActivityId,
}

/// Where a job sends what it measures: through the event loop from a worker,
/// or straight into a list when there is no event loop to post to.
type Sink<'a> = &'a dyn Fn(UserEvent);

/// Measure everything `job` asks for, sending each result as it lands.
fn run(job: MeasureJob, cancel: &CancelToken, send: Sink<'_>) {
    let MeasureJob {
        generation,
        activity,
        model,
        new_model,
        attached,
        started,
    } = job;
    let measure_groups = new_model.is_some();
    let clock = StageClock::new(model.name.clone());
    let report = |stage: ImportStage| {
        clock.enter(stage);
        send(UserEvent::ModelLoadProgress(ModelLoadProgress {
            generation,
            measuring: true,
            message: progress_message(ImportProgress::stage(stage)),
            fraction: None,
            activity,
        }));
    };
    let measured = |measurement| {
        send(UserEvent::ModelMeasured(Box::new(ModelMeasured {
            generation,
            measurement,
            activity,
        })));
    };
    // Checked before each stage below. Together they are the majority of a large
    // load — on a 2.8M-triangle scene the mesh groups alone are ~2.6 s — and none
    // of it is worth doing once the user has opened something else. The worker
    // still has to say it stopped, since that is what closes the loading card.
    let superseded = || {
        if !cancel.is_cancelled() {
            return false;
        }
        clock.finish();
        log::debug!("measuring {} superseded", model.name);
        measured(ModelMeasurement::Cancelled);
        true
    };

    if let Some(pending) = new_model {
        if superseded() {
            return;
        }
        {
            let _z = prof::zone!("Marshal Extras");
            report(ImportStage::Extras);
            let extras = pending.marshal(&model).map(|extras| extras.map(Arc::new));
            send(UserEvent::SourceExtrasReady(Box::new(SourceExtrasReady {
                generation,
                extras,
            })));
        }
        if superseded() {
            return;
        }
        {
            // Before the clip envelopes, because this is what the pointer needs: a
            // user reaching for a part of the model the moment it appears should
            // be able to click it.
            let _z = prof::zone!("Build Scene BVH");
            report(ImportStage::Measuring);
            measured(ModelMeasurement::SceneBvh(Arc::new(SceneBvh::build(
                &model,
            ))));
        }
        if superseded() {
            return;
        }
        {
            let _z = prof::zone!("Measure Clip Bounds");
            measured(ModelMeasurement::ClipBounds(measure_clip_bounds(&model)));
        }
    }
    for (file, animation) in &attached {
        if superseded() {
            return;
        }
        let _z = prof::zone!("Measure Attached Clip Bounds");
        report(ImportStage::Measuring);
        measured(ModelMeasurement::AttachedClipBounds {
            file: *file,
            bounds: measure_clip_bounds_of(&model, &animation.clips),
        });
    }
    // Last: the slowest of them, and the one only a number on the stats card
    // waits for.
    if measure_groups {
        if superseded() {
            return;
        }
        let _z = prof::zone!("Measure Mesh Groups");
        report(ImportStage::Finishing);
        measured(ModelMeasurement::MeshGroups(model.mesh_group_stats()));
    }
    clock.finish();
    log::debug!(
        "measuring {} finished {:.2} s after the request",
        model.name,
        started.elapsed().as_secs_f64()
    );
    measured(ModelMeasurement::Finished);
}

impl App {
    /// Measure `job` on a worker thread — or in place, when there is no event
    /// loop to post back to.
    pub(super) fn spawn_measure(&mut self, job: MeasureJob) {
        let cancel = CancelToken::new(Arc::clone(&self.measure_generation), job.generation);
        let Some(proxy) = self.textures.proxy.clone() else {
            let landed = std::cell::RefCell::new(Vec::new());
            run(job, &cancel, &|event| landed.borrow_mut().push(event));
            for event in landed.into_inner() {
                match event {
                    UserEvent::ModelMeasured(message) => self.handle_model_measured(*message),
                    UserEvent::SourceExtrasReady(message) => {
                        self.handle_source_extras_ready(*message);
                    }
                    _ => {}
                }
            }
            return;
        };
        let activity = job.activity;
        let spawned = std::thread::Builder::new()
            .name("model-measure".into())
            .spawn(move || {
                prof::thread_name("model-measure");
                // A send failure only means the event loop has exited.
                run(job, &cancel, &|event| {
                    let _ = proxy.send_event(event);
                });
            });
        if let Err(error) = spawned {
            // No worker, so nothing will close the card; the model stays on screen
            // unmeasured, which every consumer already reads as "not yet".
            log::error!("could not start the measuring thread: {error}");
            self.notifications.end_activity(activity);
        }
    }

    /// Fold one deferred measurement into the UI. The model it describes is
    /// already on screen; a measurement whose generation has been superseded is
    /// dropped, but the message that ends a job still brings its card down.
    pub(crate) fn handle_model_measured(&mut self, message: ModelMeasured) {
        if matches!(
            message.measurement,
            ModelMeasurement::Finished | ModelMeasurement::Cancelled
        ) {
            self.notifications.end_activity(message.activity);
        }
        if message.generation != self.measure_generation() {
            return;
        }
        match message.measurement {
            // Framing and the bounding box describe the selected clip's whole
            // motion; until this lands they describe the whole model instead.
            ModelMeasurement::ClipBounds(bounds) => self.ui.set_clip_bounds(0, bounds),
            ModelMeasurement::AttachedClipBounds { file, bounds } => {
                self.ui.set_clip_bounds(file, bounds);
            }
            // Fills in the `GPU Verts` row and the scoped stats columns, and
            // spares the main thread building the same table itself.
            ModelMeasurement::MeshGroups(groups) => self.ui.set_mesh_group_stats(groups),
            // Until this lands the viewport cannot be picked and the dimension
            // labels do not occlude; both simply start working when it does.
            ModelMeasurement::SceneBvh(bvh) => self.scene_bvh = Some(bvh),
            ModelMeasurement::Finished | ModelMeasurement::Cancelled => {}
        }
        self.request_redraw();
    }

    /// Keep the source-property capture of the model on screen. A capture whose
    /// generation has been superseded describes a model no longer shown and is
    /// dropped.
    pub(crate) fn handle_source_extras_ready(&mut self, message: SourceExtrasReady) {
        if message.generation != self.measure_generation() {
            return;
        }
        // Landed or failed, the capture has settled: the audit can run now
        // rather than run twice.
        self.audit_capture_settled();
        match message.extras {
            Ok(extras) => {
                let landed = extras.is_some();
                self.scene_extras = extras;
                // An Opt run started before this — the one a load starts in
                // the workspace at once — had none of it to carry; the audit
                // waits for the capture, but Opt does not, so it runs again.
                if landed {
                    self.opt_capture_landed();
                    // The capture is what names the model's own textures.
                    self.load_imported_textures();
                }
            }
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
}
