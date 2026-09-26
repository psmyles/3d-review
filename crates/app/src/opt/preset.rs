//! Saving and loading the operation stack as a preset file.

use std::path::Path;
use std::sync::Arc;

use review_optimize::{RebindReport, preset};

use super::*;

/// What to tell the user about a loaded preset's per-object overrides, one line
/// per thing that happened. A preset that landed cleanly produces nothing.
///
/// The two halves are separate warnings because they mean different things: a
/// dropped override is a setting the user has lost, while one matched by
/// position is a setting that *may* have landed on the wrong object — only an
/// old preset can produce it, and only the user can tell whether it is right.
fn rebind_notes(report: RebindReport) -> Vec<String> {
    // Both notes read differently for one override than for several. That is a
    // plural rule, so it belongs in the catalog's own `{ $count -> }` selector
    // rather than in hand-picked `if count == 1` fragments here — English needs
    // two forms, and other languages need more or fewer.
    let mut notes = Vec::new();
    if report.dropped > 0 {
        notes.push(keys::app_notifications::overrides_dropped(
            report.dropped as f64,
        ));
    }
    if report.by_position > 0 {
        notes.push(keys::app_notifications::overrides_by_position(
            report.by_position as f64,
        ));
    }
    notes
}

impl App {
    /// Ask where to save the operation stack as a preset.
    ///
    /// The stack is serialized before the picker opens, for the same reason the
    /// export captures its chain: the chrome keeps running behind the dialog, so
    /// the stack the user was looking at when they clicked Save is the one that
    /// gets written, not whatever it has become by the time they choose a name.
    pub(super) fn save_opt_preset(&mut self) {
        // Per-object overrides address node *indices*, which mean nothing in
        // another file. Record the name of each one's object on the way out so
        // the preset can be rebound wherever it is loaded; the live stack keeps
        // addressing by index.
        let mut stack = (*self.ui.opt.stack).clone();
        stack.stamp_node_names(&self.scene_model.nodes);
        let json = match preset::to_json(&stack) {
            Ok(json) => json,
            Err(error) => {
                self.notifications
                    .error(keys::app_notifications::preset_build_failed(
                        crate::explain::explain_opt_error(&error),
                    ));
                return;
            }
        };
        self.ask(Dialog::SavePreset { json });
    }

    /// Write a serialized preset to the chosen path.
    ///
    /// Replaced rather than overwritten (`write_bytes_replacing`): saving over an
    /// existing preset truncates it before the first byte lands, so a failure
    /// halfway through would cost the user the setup they were replacing as well
    /// as the one they were saving.
    pub(crate) fn write_opt_preset(&mut self, path: &Path, json: &str) {
        match review_optimize::write_bytes_replacing(path, json) {
            Ok(()) => self
                .notifications
                .success(keys::app_notifications::preset_saved(
                    crate::loading::file_label(path),
                )),
            Err(error) => {
                log::error!("preset save failed {}: {error}", path.display());
                self.notifications
                    .error(keys::app_notifications::couldnt_save(
                        crate::loading::file_label(path),
                    ));
            }
        }
    }

    /// Ask for a preset to load.
    pub(super) fn load_opt_preset(&mut self) {
        self.ask(Dialog::LoadPreset);
    }

    /// Apply a chosen preset file to the operation stack.
    pub(crate) fn read_opt_preset(&mut self, path: &Path) {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(error) => {
                log::error!("preset read failed {}: {error}", path.display());
                self.notifications
                    .error(keys::app_notifications::couldnt_read(
                        crate::loading::file_label(path),
                    ));
                return;
            }
        };
        let mut stack = match preset::from_json(&json) {
            Ok(stack) => stack,
            Err(error) => {
                self.notifications
                    .error(keys::app_notifications::preset_load_failed(
                        crate::explain::explain_opt_error(&error),
                    ));
                return;
            }
        };

        // A preset authored against another asset carries that asset's node
        // indices, which say nothing about *which* object was meant here. Each
        // override names its object, so match on that and drop what this model
        // has no unambiguous answer for.
        let report = stack.rebind_to_model(&self.scene_model.nodes);

        self.ui.opt.set_stack(Arc::new(stack));
        self.schedule_reprocess();
        self.redraw.requested = true;

        self.notifications
            .success(keys::app_notifications::preset_loaded(
                crate::loading::file_label(path),
            ));
        for line in rebind_notes(report) {
            self.notifications.warning(line);
        }
    }
}
