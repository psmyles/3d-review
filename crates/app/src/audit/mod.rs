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

mod profile;
mod report;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use review_audit::{AuditError, AuditInput, AuditProfile, AuditReport, RunOptions};
use review_model::{CancelToken, ModelData, SourceExtras};
use review_ui::{ActivityId, AuditIntent};

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
        }
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
        self.audit.generation.fetch_add(1, Ordering::Relaxed);
        self.audit.ready_for = None;
        self.audit.covers = None;
        self.audit.report_for = None;
        self.audit.dirty = false;
        if let Some(activity) = self.audit.activity.take() {
            self.notifications.end_activity(activity);
        }
        self.ui.aud.reset_for_new_model();
    }

    /// Reconcile the audit with the model and profile on screen. Every frame, in
    /// every workspace: the toolbar shows the count from anywhere.
    pub(crate) fn sync_audit(&mut self) {
        let model_ready = self.audit.ready_for == Some(self.scene_revision)
            && !(self.scene_model.indices.is_empty() && self.scene_model.nodes.is_empty());
        let wanted: Coverage = (self.scene_revision, self.ui.aud.profile_revision);
        if model_ready && self.audit.covers != Some(wanted) {
            self.schedule_audit();
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

    /// Queue a run for the current model and profile, or mark one owed.
    fn schedule_audit(&mut self) {
        self.audit.generation.fetch_add(1, Ordering::Relaxed);
        self.audit.covers = Some((self.scene_revision, self.ui.aud.profile_revision));
        self.audit.dirty = true;
        if self.audit.in_flight.is_none() {
            self.spawn_audit();
        }
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
        let generation = self.audit.generation.load(Ordering::Relaxed);
        let cancel = CancelToken::new(Arc::clone(&self.audit.generation), generation);
        self.audit.in_flight = Some(generation);
        self.audit.dirty = false;
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
        if self.audit.in_flight == Some(done.generation) {
            self.audit.in_flight = None;
        }
        let current = done.generation == self.audit.generation.load(Ordering::Relaxed);
        if current {
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
            // Only an edit owes a run. A run made stale by a new load is not
            // replaced here: that model's run waits for its capture to settle.
            if self.audit.dirty {
                self.spawn_audit();
            }
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
