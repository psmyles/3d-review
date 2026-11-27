//! Exporting the processed chain: the dialog, the worker that writes it, and
//! the report once it lands.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use review_model::ModelData;
use review_optimize::{
    CancelToken, ExportOptions, ExportReport, OptError, ProcessedResult, export_cancellable,
};
use review_ui::{ActivityId, NoticeKind};

use super::*;

/// A finished export, posted from its worker: what it wrote (or why it did not),
/// and the card it opened.
#[derive(Debug)]
pub(crate) struct OptExported {
    outcome: Result<ExportReport, OptError>,
    activity: ActivityId,
}

impl App {
    /// Ask where to write the current LOD chain.
    ///
    /// The chain, the source model and the export options are captured *here*, not
    /// when the picker closes: the workspace stays live behind the dialog, so a
    /// background run could land a different chain, or the user edit the stack,
    /// between the click and the chosen path. What they clicked Export on is what
    /// travels out to the dialog worker and comes back to be written.
    pub(super) fn export_opt_result(&mut self) {
        let Some(result) = self.opt.as_ref().and_then(|opt| opt.processed.clone()) else {
            self.notifications.error(
                review_localization::tr(keys::app_notifications::NOTHING_TO_EXPORT).into_owned(),
            );
            return;
        };
        // The capture the chain was processed with, and none if it was processed
        // before the capture landed (the rerun that fixes that may still be on
        // its way): the chain and the capture have to describe the same run.
        let with_capture = self
            .opt
            .as_ref()
            .is_some_and(OptSubsystem::processed_with_capture);
        let extras = self.scene_extras.clone().filter(|_| with_capture);

        // A sensible default file name: the model's own, which is also the stem
        // the per-LOD packaging appends its suffixes to.
        let stem = if self.scene_model.name.is_empty() {
            review_localization::tr(keys::app_dialogs::EXPORT_FILE_NAME).into_owned()
        } else {
            self.scene_model.name.clone()
        };
        self.ask(Dialog::ExportOpt {
            result,
            source: Arc::clone(&self.scene_model),
            extras,
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
            log::error!("no event-loop proxy; cannot export off-thread");
            self.notifications.error(
                review_localization::tr(keys::app_notifications::EXPORT_START_FAILED).into_owned(),
            );
            return;
        };

        let activity = self.notifications.begin_activity(
            review_localization::tr(keys::app_notifications::EXPORTING).into_owned(),
        );
        // A counter of the export's own, so its card's Cancel stops this export
        // and no other: the token is live until the counter leaves 0.
        let stop = Arc::new(AtomicU64::new(0));
        let cancel = CancelToken::new(Arc::clone(&stop), 0);
        if let Some(opt) = self.opt.as_mut() {
            opt.exports.push((activity, stop));
        }

        log::info!("exporting to {}", path.display());
        let spawned = std::thread::Builder::new()
            .name("mesh-export".into())
            .spawn(move || {
                prof::thread_name("mesh-export");
                let started = Instant::now();
                let outcome = {
                    let _z = prof::zone!("Export");
                    export_cancellable(
                        &result.lods,
                        &source,
                        extras.as_deref(),
                        &path,
                        &options,
                        Some(&cancel),
                    )
                };
                // The failures are logged where they are reported, on the main thread.
                if let Ok(report) = &outcome {
                    let files: Vec<String> = report
                        .files
                        .iter()
                        .map(|file| file.display().to_string())
                        .collect();
                    log::info!(
                        "exported {} meshes, {} triangles, in {:.2} s: {}",
                        report.mesh_count,
                        report.triangle_count,
                        started.elapsed().as_secs_f64(),
                        files.join(", ")
                    );
                }
                let _ = proxy.send_event(UserEvent::OptExported(Box::new(OptExported {
                    outcome,
                    activity,
                })));
            });
        if let Err(error) = spawned {
            log::error!("could not start the export thread: {error}");
            self.forget_export(activity);
            self.notifications.end_activity(activity);
            self.notifications.error(
                review_localization::tr(keys::app_notifications::EXPORT_START_FAILED).into_owned(),
            );
        }
    }

    /// An export card's Cancel: ask that export's worker to stop. Whether
    /// `activity` was an export's card.
    ///
    /// The worker gives up before it replaces any file, and says so when it
    /// lands; one that had already put every file in place lands as a success,
    /// which is then the truth and is reported as one.
    pub(crate) fn cancel_opt_export(&mut self, activity: ActivityId) -> bool {
        let Some((_, stop)) = self
            .opt
            .as_ref()
            .and_then(|opt| opt.exports.iter().find(|(card, _)| *card == activity))
        else {
            return false;
        };
        stop.fetch_add(1, Ordering::Relaxed);
        log::info!("export cancel requested");
        true
    }

    /// Stop tracking the export that opened `activity`.
    fn forget_export(&mut self, activity: ActivityId) {
        if let Some(opt) = self.opt.as_mut() {
            opt.exports.retain(|(card, _)| *card != activity);
        }
    }

    /// Report a finished export on the main thread.
    pub(crate) fn handle_opt_exported(&mut self, exported: OptExported) {
        self.forget_export(exported.activity);
        self.notifications.end_activity(exported.activity);
        match exported.outcome {
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
                for note in &report.notes {
                    log::info!("export: {note}");
                }
                let notes = report
                    .notes
                    .iter()
                    .map(review_labels::opt::explain_export_note)
                    .collect();
                self.notifications.report(NoticeKind::Success, title, notes);
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
                log::error!("FBX export incomplete at {}: {reason}", failed.display());
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
            // Its card's Cancel, honoured before anything was replaced.
            Err(OptError::Cancelled) => {
                log::info!("export cancelled; nothing written");
                self.notifications.info(
                    review_localization::tr(keys::app_notifications::EXPORT_CANCELLED).into_owned(),
                );
            }
            Err(error) => {
                log::error!("FBX export failed: {error}");
                self.notifications
                    .error(keys::app_notifications::export_failed(
                        review_labels::opt::explain_opt_error(&error),
                    ));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
