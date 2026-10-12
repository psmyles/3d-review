//! The Aud workspace's app-side machinery: running the audit on a background
//! thread and handing the report to the UI.
//!
//! ## When it runs
//!
//! On every load, once the model is on screen *and* its source-property capture
//! has settled (landed, or failed): several checks read the capture, and running
//! before it lands would mean running twice. That is after the mesh is drawn and
//! beside the import worker's remaining measurements, so startup and the time to
//! a drawn model are untouched. The worker's own parallelism leaves two cores to
//! the main thread and the import worker.
//!
//! ## Latest-request-wins
//!
//! The same shape as Opt (`opt/process.rs`): a profile edit bumps a generation,
//! which also cancels the run in flight through its [`CancelToken`]; only one
//! worker is ever running, an edit arriving mid-run marks the subsystem dirty,
//! and the result handler respawns once with the latest profile. A re-run hands
//! the previous report back as `reuse`, so editing one threshold re-measures one
//! rule.

mod highlight;
mod profile;
mod report;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use review_audit::{AuditError, AuditInput, AuditProfile, AuditReport, RunOptions};
use review_model::{CancelToken, ModelData, SourceExtras};
use review_ui::{ActivityId, AuditIntent};

pub(crate) use highlight::{AUDIT_PICK_TOLERANCE_POINTS, offender_near};
pub(crate) use profile::restore_saved_profile;

use crate::events::UserEvent;
use crate::keys;
use crate::{App, prof};

/// How long a run may take before the user is told it is still going.
const ACTIVITY_NOTICE_AFTER: Duration = Duration::from_millis(500);

/// What a run covers: the model and the profile it was asked for.
type Coverage = (u64, u64);

/// A finished run, posted back by the audit worker.
#[derive(Debug)]
pub(crate) struct AuditDone {
    generation: u64,
    coverage: Coverage,
    result: Result<AuditReport, AuditError>,
}

/// Everything the audit needs on the app side. Always present — it holds a
/// handful of counters until there is a model to check.
#[derive(Debug)]
pub(crate) struct AuditSubsystem {
    /// Monotonic run counter, shared with the worker so bumping it both marks a
    /// newer request and cancels the run in flight.
    generation: Arc<AtomicU64>,
    /// The generation of the run on the worker thread, if any.
    in_flight: Option<u64>,
    /// A newer request arrived while a run was in flight.
    dirty: bool,
    /// The model the capture has settled for — what makes a run worth starting.
    ready_for: Option<u64>,
    /// What the newest run was started for.
    covers: Option<Coverage>,
    /// What the report on screen describes, so a re-run can reuse it only when
    /// it describes the same model.
    report_for: Option<u64>,
    started_at: Option<Instant>,
    activity: Option<ActivityId>,
    /// The profile revision last written to the config dir.
    saved_revision: u64,
    /// What the viewport highlights for the focus.
    pub(crate) highlight: highlight::AuditHighlight,
}

impl Default for AuditSubsystem {
    fn default() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            in_flight: None,
            dirty: false,
            ready_for: None,
            covers: None,
            report_for: None,
            started_at: None,
            activity: None,
            saved_revision: 0,
            highlight: highlight::AuditHighlight::default(),
        }
    }
}

/// What a finished run means for the subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Finished {
    /// The run answers the newest request, so its result is shown.
    current: bool,
    /// An edit arrived while it ran: start one more with the latest profile.
    respawn: bool,
}

impl AuditSubsystem {
    /// Whether the model and profile on screen want a run they have not had:
    /// the capture has settled for this model, there is geometry to check, and
    /// no run has been started for exactly this pair.
    fn wants_run(&self, coverage: Coverage, has_geometry: bool) -> bool {
        self.ready_for == Some(coverage.0) && has_geometry && self.covers != Some(coverage)
    }

    /// Record a request for `coverage`, cancelling any run in flight. Whether a
    /// worker should start now; if one is running, the request waits as
    /// `dirty` for it to finish.
    fn request(&mut self, coverage: Coverage) -> bool {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.covers = Some(coverage);
        self.dirty = true;
        self.in_flight.is_none()
    }

    /// A worker is starting: the generation it carries.
    fn start(&mut self) -> u64 {
        let generation = self.generation.load(Ordering::Relaxed);
        self.in_flight = Some(generation);
        self.dirty = false;
        generation
    }

    /// A worker carrying `generation` has finished.
    fn finish(&mut self, generation: u64) -> Finished {
        if self.in_flight == Some(generation) {
            self.in_flight = None;
        }
        let current = generation == self.generation.load(Ordering::Relaxed);
        // Only an edit owes a run. A run made stale by a new load is not
        // replaced here: that model's run waits for its capture to settle.
        let respawn = self.in_flight.is_none() && self.dirty;
        Finished { current, respawn }
    }

    /// Forget the outgoing model, and stop its run.
    fn forget_model(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.ready_for = None;
        self.covers = None;
        self.report_for = None;
        self.dirty = false;
    }
}

impl App {
    /// The source-property capture of the model on screen has settled, one way
    /// or the other: the audit may run.
    pub(crate) fn audit_capture_settled(&mut self) {
        self.audit.ready_for = Some(self.scene_revision);
    }

    /// Drop everything that describes the outgoing model and stop its run.
    pub(crate) fn reset_audit_for_new_model(&mut self) {
        self.audit.forget_model();
        if let Some(activity) = self.audit.activity.take() {
            self.notifications.end_activity(activity);
        }
        self.ui.aud.reset_for_new_model();
    }

    /// Reconcile the audit with the model and profile on screen. Every frame, in
    /// every workspace: the toolbar shows the count from anywhere.
    pub(crate) fn sync_audit(&mut self) {
        let has_geometry =
            !(self.scene_model.indices.is_empty() && self.scene_model.nodes.is_empty());
        let wanted: Coverage = (self.scene_revision, self.ui.aud.profile_revision);
        if self.audit.wants_run(wanted, has_geometry) && self.audit.request(wanted) {
            self.spawn_audit();
        }

        if let Some(started) = self.audit.started_at
            && self.audit.in_flight.is_some()
            && self.audit.activity.is_none()
            && started.elapsed() >= ACTIVITY_NOTICE_AFTER
        {
            self.audit.activity =
                Some(self.notifications.begin_activity(
                    review_localization::tr(keys::app_audit::AUDITING).into_owned(),
                ));
        }

        let running = self.audit.in_flight.is_some();
        if self.ui.aud.running != running {
            self.ui.aud.running = running;
            self.redraw.requested = true;
        }

        self.persist_audit_profile_when_settled();
    }

    fn spawn_audit(&mut self) {
        let Some(proxy) = self.textures.proxy.clone() else {
            log::error!("no event-loop proxy; cannot audit off-thread");
            self.audit.dirty = false;
            return;
        };
        let model: Arc<ModelData> = Arc::clone(&self.scene_model);
        let extras: Option<Arc<SourceExtras>> = self.scene_extras.clone();
        let profile: Arc<AuditProfile> = Arc::clone(&self.ui.aud.profile);
        let coverage: Coverage = (self.scene_revision, self.ui.aud.profile_revision);
        // Reuse only a report of this same model.
        let reuse = (self.audit.report_for == Some(self.scene_revision))
            .then(|| self.ui.aud.report.clone())
            .flatten();
        let generation = self.audit.start();
        let cancel = CancelToken::new(Arc::clone(&self.audit.generation), generation);
        self.audit.started_at = Some(Instant::now());
        let threads = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(2).max(1));

        let spawned = std::thread::Builder::new()
            .name("model-audit".into())
            .spawn(move || {
                prof::thread_name("model-audit");
                let _z = prof::zone!("Audit");
                let result = review_audit::run(
                    AuditInput {
                        model: &model,
                        extras: extras.as_deref(),
                    },
                    &profile,
                    reuse.as_deref(),
                    RunOptions {
                        cancel: Some(&cancel),
                        threads,
                    },
                );
                // A send failure only means the event loop has exited.
                let _ = proxy.send_event(UserEvent::AuditDone(Box::new(AuditDone {
                    generation,
                    coverage,
                    result,
                })));
            });
        if let Err(error) = spawned {
            log::error!("could not start the audit thread: {error}");
            self.audit.in_flight = None;
        }
    }

    /// Apply a finished run on the main thread.
    pub(crate) fn handle_audit_done(&mut self, done: AuditDone) {
        let finished = self.audit.finish(done.generation);
        if finished.current {
            match done.result {
                Ok(report) => {
                    log::info!(
                        "audited in {:.0} ms ({} B): {} errors, {} warnings, {} info",
                        report.elapsed_ms,
                        report.heap_bytes(),
                        report.summary.failed[2],
                        report.summary.failed[1],
                        report.summary.failed[0],
                    );
                    self.audit.report_for = Some(done.coverage.0);
                    self.ui.aud.report = Some(Arc::new(report));
                }
                // Superseded mid-run; the newer request is already scheduled.
                Err(AuditError::Cancelled) => {}
                Err(error) => {
                    log::error!("audit failed: {error}");
                    self.notifications.error(keys::app_audit::failed(
                        crate::explain::explain_audit_error(&error),
                    ));
                }
            }
        }
        if self.audit.in_flight.is_none() {
            if let Some(activity) = self.audit.activity.take() {
                self.notifications.end_activity(activity);
            }
            self.audit.started_at = None;
        }
        if finished.respawn {
            self.spawn_audit();
        }
        self.request_redraw();
    }

    /// Whether a run is still going — the gate waits for the audit to settle
    /// before it samples memory.
    pub(crate) fn audit_idle(&self) -> bool {
        self.audit.in_flight.is_none() && !self.audit.dirty
    }

    /// Carry out an Aud action the chrome raised.
    pub(crate) fn apply_audit_intent(&mut self, intent: AuditIntent) {
        match intent {
            AuditIntent::SaveProfile => self.save_audit_profile(),
            AuditIntent::LoadProfile => self.load_audit_profile(),
            AuditIntent::SaveReport => self.save_audit_report(),
            AuditIntent::SummaryCopied => self
                .notifications
                .success(review_localization::tr(keys::app_audit::SUMMARY_COPIED).into_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditSubsystem, Finished};

    /// Nothing runs before the capture has settled for this model, or on a
    /// model with nothing in it; once started, the same pair never runs twice.
    #[test]
    fn a_run_waits_for_the_capture_and_is_never_repeated() {
        let mut audit = AuditSubsystem::default();
        assert!(
            !audit.wants_run((1, 0), true),
            "the capture has not settled"
        );
        audit.ready_for = Some(1);
        assert!(!audit.wants_run((1, 0), false), "an empty model");
        assert!(audit.wants_run((1, 0), true));
        assert!(audit.request((1, 0)), "nothing in flight: start now");
        audit.start();
        assert!(!audit.wants_run((1, 0), true), "already covered");
        assert!(
            audit.wants_run((1, 1), true),
            "a profile edit is a new pair"
        );
    }

    /// Edits during a run coalesce: the run is cancelled, its result dropped,
    /// and exactly one more run starts, with the latest profile.
    #[test]
    fn edits_during_a_run_coalesce_into_one_more() {
        let mut audit = AuditSubsystem {
            ready_for: Some(1),
            ..Default::default()
        };
        assert!(audit.request((1, 0)));
        let first = audit.start();
        let token = review_model::CancelToken::new(audit.generation.clone(), first);

        // Two edits land while it runs.
        assert!(!audit.request((1, 1)), "a run is in flight: wait");
        assert!(!audit.request((1, 2)));
        assert!(token.is_cancelled(), "the edit cancels the run in flight");

        assert_eq!(
            audit.finish(first),
            Finished {
                current: false,
                respawn: true
            },
            "the stale result is dropped and one run is owed",
        );
        let second = audit.start();
        assert_eq!(audit.covers, Some((1, 2)), "with the latest profile");
        assert_eq!(
            audit.finish(second),
            Finished {
                current: true,
                respawn: false
            },
        );
    }

    /// A new model cancels the old one's run, and its result neither shows nor
    /// starts a run: the new model's run waits for its own capture.
    #[test]
    fn a_new_model_drops_the_old_run_without_replacing_it() {
        let mut audit = AuditSubsystem {
            ready_for: Some(1),
            ..Default::default()
        };
        assert!(audit.request((1, 0)));
        let old = audit.start();
        audit.forget_model();
        assert_eq!(
            audit.finish(old),
            Finished {
                current: false,
                respawn: false
            },
        );
        assert!(
            !audit.wants_run((2, 0), true),
            "the new capture has not settled"
        );
    }
}
