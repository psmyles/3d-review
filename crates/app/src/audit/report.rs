//! Save report: the audit report as JSON, written where the user chooses.
//!
//! The document is built before the picker opens, from the report, model and
//! profile on screen at the click — the same rule every other save follows
//! (`dialog.rs`): a run that lands while the picker is up must not change what
//! gets written.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use review_audit::report::{ReportSource, to_json};

use crate::App;
use crate::dialog::Dialog;
use crate::keys;

impl App {
    pub(super) fn save_audit_report(&mut self) {
        let Some(report) = self.ui.aud.report.clone() else {
            return;
        };
        let extras = self.scene_extras.as_deref();
        let file = extras
            .map(|extras| extras.scene.filename.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.scene_model.name.clone());
        let source = ReportSource {
            file,
            generator: format!("{} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION")),
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        };
        match to_json(
            &report,
            &self.scene_model,
            extras,
            &self.ui.aud.profile,
            &source,
        ) {
            Ok(json) => self.ask(Dialog::SaveAuditReport {
                json,
                stem: self.scene_model.name.clone(),
            }),
            Err(error) => self.notifications.error(keys::app_audit::report_failed(
                crate::explain::explain_audit_error(&error),
            )),
        }
    }

    pub(crate) fn write_audit_report(&mut self, path: &Path, json: &str) {
        match review_optimize::write_bytes_replacing(path, json) {
            Ok(()) => self.notifications.success(keys::app_audit::report_saved(
                crate::loading::file_label(path),
            )),
            Err(error) => {
                log::error!("audit report save failed {}: {error}", path.display());
                self.notifications
                    .error(keys::app_notifications::couldnt_save(
                        crate::loading::file_label(path),
                    ));
            }
        }
    }
}
