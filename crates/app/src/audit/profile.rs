//! The audit profile on disk: the last-used profile in the config dir, and the
//! Save / Load profile dialogs.
//!
//! The last-used profile is the whole profile, not a pointer to a file — a
//! profile edited in the Inspector and never saved still comes back next launch.
//! It is written whenever the revision has settled (not on every frame of a
//! drag) and on exit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use review_audit::profile::envelope;

use crate::App;
use crate::dialog::Dialog;
use crate::keys;

/// The file's name inside `<config dir>/<app name>/`.
const PROFILE_FILE: &str = "audit-profile.json";

fn saved_profile_file() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(crate::APP_NAME);
    path.push(PROFILE_FILE);
    Some(path)
}

/// Restore the last-used profile into `app` at launch. A missing file is the
/// first launch and keeps the default; an unreadable one is reported once and
/// replaced by the default on the next save.
pub(crate) fn restore_saved_profile(app: &mut App) {
    let Some(path) = saved_profile_file() else {
        return;
    };
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            log::warn!("audit profile unreadable {}: {error}", path.display());
            return;
        }
    };
    match envelope::from_json(&json) {
        Ok(profile) => {
            app.ui.aud.set_profile(Arc::new(profile));
            app.audit.saved_revision = app.ui.aud.profile_revision;
        }
        Err(error) => {
            log::warn!("saved audit profile rejected: {error}");
            app.notifications
                .warning(keys::app_audit::saved_profile_reset(
                    crate::explain::explain_audit_error(&error),
                ));
        }
    }
}

impl App {
    /// Write the profile to the config dir once its revision has stopped
    /// moving — never mid-drag, which would rewrite the file every frame.
    pub(super) fn persist_audit_profile_when_settled(&mut self) {
        if self.gate.is_some()
            || self.drag_in_progress
            || self.audit.saved_revision == self.ui.aud.profile_revision
        {
            return;
        }
        self.persist_audit_profile();
    }

    /// Write the profile to the config dir now (on exit).
    pub(crate) fn persist_audit_profile(&mut self) {
        if self.gate.is_some() {
            return;
        }
        self.audit.saved_revision = self.ui.aud.profile_revision;
        let Some(path) = saved_profile_file() else {
            return;
        };
        let json = match envelope::to_json(&self.ui.aud.profile) {
            Ok(json) => json,
            Err(error) => {
                log::error!("audit profile not saved: {error}");
                return;
            }
        };
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            log::warn!("config dir unavailable {}: {error}", parent.display());
            return;
        }
        if let Err(error) = review_optimize::write_bytes_replacing(&path, &json) {
            log::warn!("audit profile not saved {}: {error}", path.display());
        }
    }

    /// Ask where to save the profile. Serialized before the picker opens, so it
    /// is the profile the user was looking at.
    pub(super) fn save_audit_profile(&mut self) {
        match envelope::to_json(&self.ui.aud.profile) {
            Ok(json) => self.ask(Dialog::SaveAuditProfile { json }),
            Err(error) => self
                .notifications
                .error(keys::app_audit::profile_unreadable(
                    crate::explain::explain_audit_error(&error),
                )),
        }
    }

    pub(super) fn load_audit_profile(&mut self) {
        self.ask(Dialog::LoadAuditProfile);
    }

    /// Write a serialized profile to the chosen path.
    pub(crate) fn write_audit_profile(&mut self, path: &Path, json: &str) {
        match review_optimize::write_bytes_replacing(path, json) {
            Ok(()) => self.notifications.success(keys::app_audit::profile_saved(
                crate::loading::file_label(path),
            )),
            Err(error) => {
                log::error!("audit profile save failed {}: {error}", path.display());
                self.notifications
                    .error(keys::app_notifications::couldnt_save(
                        crate::loading::file_label(path),
                    ));
            }
        }
    }

    /// Apply a chosen profile file.
    pub(crate) fn read_audit_profile(&mut self, path: &Path) {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(error) => {
                log::error!("audit profile read failed {}: {error}", path.display());
                self.notifications
                    .error(keys::app_notifications::couldnt_read(
                        crate::loading::file_label(path),
                    ));
                return;
            }
        };
        match envelope::from_json(&json) {
            Ok(profile) => {
                self.ui.aud.set_profile(Arc::new(profile));
                self.redraw.requested = true;
                self.notifications.success(keys::app_audit::profile_loaded(
                    crate::loading::file_label(path),
                ));
            }
            Err(error) => {
                self.notifications
                    .error(keys::app_audit::profile_unreadable(
                        crate::explain::explain_audit_error(&error),
                    ));
            }
        }
    }
}
