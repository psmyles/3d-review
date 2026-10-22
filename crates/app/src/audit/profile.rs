//! The audit profile on disk: the last-used profile in the config dir, and the
//! Save / Load profile dialogs.
//!
//! The last-used profile is the whole profile, not a pointer to a file — a
//! profile edited in the Inspector and never saved still comes back next launch.
//! It is written whenever the revision has settled (not on every frame of a
//! drag) and on exit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use review_audit::AuditProfile;
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

/// The profile name a file stands for: its name without the extension.
fn profile_name_from(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_owned())
        .filter(|name| !name.is_empty())
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
                    review_labels::audit::explain_audit_error(&error),
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

    /// Ask where to save the profile. It travels with the request, so the
    /// profile written is the one the user was looking at.
    pub(super) fn save_audit_profile(&mut self) {
        self.ask(Dialog::SaveAuditProfile {
            profile: Arc::clone(&self.ui.aud.profile),
        });
    }

    pub(super) fn load_audit_profile(&mut self) {
        self.ask(Dialog::LoadAuditProfile);
    }

    /// Write `profile` to the chosen path, named after the file — the save is
    /// where a profile is named, as an Opt preset is by its file alone. The
    /// profile on screen takes the name too unless it has been edited since the
    /// request: then it is no longer the profile that file holds.
    pub(crate) fn write_audit_profile(&mut self, path: &Path, profile: &AuditProfile) {
        let mut named = profile.clone();
        named.name = profile_name_from(path).unwrap_or_default();
        let json = match envelope::to_json(&named) {
            Ok(json) => json,
            Err(error) => {
                self.notifications
                    .error(keys::app_audit::profile_unreadable(
                        review_labels::audit::explain_audit_error(&error),
                    ));
                return;
            }
        };
        match review_optimize::write_bytes_replacing(path, &json) {
            Ok(()) => {
                let live = &self.ui.aud.profile;
                if live.base == named.base && live.rules == named.rules && live.name != named.name {
                    self.ui.aud.rename_profile(named.name);
                    self.persist_audit_profile();
                    self.redraw.requested = true;
                }
                self.notifications.success(keys::app_audit::profile_saved(
                    crate::loading::file_label(path),
                ));
            }
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
            Ok(mut profile) => {
                // A file saved before the name came from the file name may
                // carry none; its file name is what the user called it.
                if profile.name.trim().is_empty() {
                    profile.name = profile_name_from(path).unwrap_or_default();
                }
                self.ui.aud.set_profile(Arc::new(profile));
                self.redraw.requested = true;
                self.notifications.success(keys::app_audit::profile_loaded(
                    crate::loading::file_label(path),
                ));
            }
            Err(error) => {
                self.notifications
                    .error(keys::app_audit::profile_unreadable(
                        review_labels::audit::explain_audit_error(&error),
                    ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use review_audit::{AuditProfile, Engine};

    use super::{Saved, profile_name_from, read_saved, write_saved};

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

    /// A saved profile is named after its file, without the extension.
    #[test]
    fn a_profile_is_named_after_its_file() {
        let name = |path: &str| profile_name_from(&PathBuf::from(path));
        assert_eq!(
            name("/teams/Studio props.json").as_deref(),
            Some("Studio props")
        );
        assert_eq!(name("/teams/hero.v2.json").as_deref(), Some("hero.v2"));
        assert_eq!(name("/"), None);
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
