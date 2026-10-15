//! Saving and loading the operation stack as a preset file.

use std::path::Path;
use std::sync::Arc;

use review_optimize::preset;

use super::*;

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
                        review_labels::opt::explain_opt_error(&error),
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
                        review_labels::opt::explain_opt_error(&error),
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
        for line in review_labels::opt::rebind_notes(report) {
            self.notifications.warning(line);
        }
    }
}
