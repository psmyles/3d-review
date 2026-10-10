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

/// What reading the saved profile found.
#[derive(Debug)]
enum Saved {
    /// No file: the first launch.
    Missing,
    /// A file the system would not hand over; already logged.
    Unreadable,
    Loaded(review_audit::AuditProfile),
    /// A file that is not a profile this build reads.
    Rejected(review_audit::AuditError),
}

fn read_saved(path: &Path) -> Saved {
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Saved::Missing,
        Err(error) => {
            log::warn!("audit profile unreadable {}: {error}", path.display());
            return Saved::Unreadable;
        }
    };
    match envelope::from_json(&json) {
        Ok(profile) => Saved::Loaded(profile),
        Err(error) => Saved::Rejected(error),
    }
}

/// Write `profile` to `path`, creating its directory.
fn write_saved(path: &Path, profile: &review_audit::AuditProfile) -> Result<(), String> {
    let json = envelope::to_json(profile).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("config dir unavailable {}: {error}", parent.display()))?;
    }
    review_optimize::write_bytes_replacing(path, &json)
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Restore the last-used profile into `app` at launch. A missing file is the
/// first launch and keeps the default; an unreadable one is reported once and
/// replaced by the default on the next save.
pub(crate) fn restore_saved_profile(app: &mut App) {
    let Some(path) = saved_profile_file() else {
        return;
    };
    match read_saved(&path) {
        Saved::Missing | Saved::Unreadable => {}
        Saved::Loaded(profile) => {
            app.ui.aud.set_profile(Arc::new(profile));
            app.audit.saved_revision = app.ui.aud.profile_revision;
        }
        Saved::Rejected(error) => {
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
        if let Err(error) = write_saved(&path, &self.ui.aud.profile) {
            log::warn!("audit profile not saved: {error}");
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use review_audit::{AuditProfile, Engine};

    use super::{Saved, read_saved, write_saved};

    /// A directory of this test's own, so parallel runs do not share a file.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "review-audit-profile-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The saved profile survives a write and a read — edits included — and the
    /// write creates the config directory on a first launch.
    #[test]
    fn the_saved_profile_round_trips_through_its_file() {
        let dir = scratch("round-trip");
        let path = dir.join("nested").join("audit-profile.json");
        assert!(matches!(read_saved(&path), Saved::Missing));

        let mut profile = AuditProfile::builtin(Engine::Unreal);
        profile.name = "Studio".into();
        write_saved(&path, &profile).expect("written");
        match read_saved(&path) {
            Saved::Loaded(read) => assert_eq!(read, profile),
            other => panic!("expected the profile back, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that is not a profile is rejected rather than half-applied.
    #[test]
    fn a_damaged_file_is_rejected() {
        let dir = scratch("damaged");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("audit-profile.json");
        std::fs::write(&path, "{ not json").expect("written");
        assert!(matches!(read_saved(&path), Saved::Rejected(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
