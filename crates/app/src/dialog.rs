//! Native file dialogs, opened on a worker thread and answered back through the
//! event loop (`docs/ARCHITECTURE.md`, Platform decisions D9).
//!
//! Every `rfd` call used to run *inline*, inside the winit callback that handled
//! the click or the key: the event loop stopped while the OS ran its own modal
//! loop, and picked up again with the answer in hand. Three of those sites were
//! reached from `render()` itself, one frame deep in the draw. On Windows that
//! merely freezes the window; on macOS it is a reliable abort, because nothing
//! called from a winit callback may pump a run loop of its own — AppKit ends the
//! process rather than re-entering.
//!
//! So a dialog now takes the shape every other slow thing in this crate already
//! has (`loading.rs`, `texture_manager.rs`, `opt.rs`): the request goes to a
//! worker thread, the worker blocks on the OS dialog, and the answer comes home
//! as a [`UserEvent::DialogDone`] applied on the main thread. The event loop
//! keeps running the whole time, so the viewport still orbits and animates while
//! the picker is up.
//!
//! Two consequences the inline version got for free and this one has to arrange:
//!
//! * **One dialog at a time.** A modal dialog made a second one unreachable —
//!   the window it would have been asked from was frozen. Nothing freezes now, so
//!   `Ctrl+O` twice in a row would open two pickers; [`App::ask`] drops a request
//!   that arrives while one is open.
//! * **The answer's payload is captured when the dialog opens, not when it
//!   closes.** An export or a preset save is *about* something — a LOD chain, an
//!   operation stack — and while the picker is up the user can still edit the
//!   stack or a background run can land a new chain. Writing whatever the state
//!   happened to be at the moment they clicked Save would be a race with no
//!   visible cause, so each request carries its own subject with it, out to the
//!   worker and back. What the user chose Export on is what gets exported.

use std::path::PathBuf;
use std::sync::Arc;

use review_model::{ModelData, SourceExtras};
use review_optimize::{ExportOptions, ProcessedResult, preset};

use crate::events::UserEvent;
use crate::keys;
use crate::loading::TEXTURE_EXTENSIONS;
use crate::{App, prof};

/// A dialog to open, carrying whatever its answer will act on.
///
/// Deliberately not `Clone`: the export variant holds a whole LOD chain, and an
/// accidental clone would deep-copy every mesh in it (invariant 1). The `Arc`s
/// are shares of what the app already holds, so building one of these is cheap
/// however large the model.
#[derive(Debug)]
pub(crate) enum Dialog {
    /// Pick one FBX to open (`Ctrl+O`, the toolbar, a double-click on the empty
    /// viewport).
    OpenModel,
    /// Pick any number of images to add to the scene texture pool.
    ImportTextures,
    /// Choose where to write the Opt workspace's LOD chain.
    ExportOpt {
        /// The chain as it stood when Export was clicked.
        result: Arc<ProcessedResult>,
        /// The source model the chain was processed from — the exporter reads its
        /// materials and node names.
        source: Arc<ModelData>,
        /// The source's property capture as it stood when Export was clicked
        /// (`None` if it had not landed yet, which the report notes).
        extras: Option<Arc<SourceExtras>>,
        options: ExportOptions,
        /// Default file name stem, which the per-LOD packaging also suffixes.
        stem: String,
        /// The review comments as they stood when Export was clicked, per source
        /// node, for the export to write onto those nodes.
        comments: Option<review_optimize::NodeStrings>,
    },
    /// Choose where to write an operation-stack preset. The JSON is serialized
    /// before the dialog opens, so it describes the stack the user was looking at.
    SavePreset { json: String },
    /// Pick an operation-stack preset to load.
    LoadPreset,
    /// Choose where to write the model with its review comments.
    SaveCommentsAs {
        /// The file name offered, the opened file's own.
        name: String,
    },
    /// The opened file changed on disk since its comments were read: write the
    /// comments over it anyway? `then` is what follows a save that goes ahead.
    ConfirmOverwrite {
        file: String,
        then: crate::comments_save::AfterSave,
    },
    /// The comments have unsaved changes and something is about to drop them:
    /// save, discard, or stay?
    UnsavedComments {
        file: String,
        /// What was about to happen, carried out once the question is answered.
        then: crate::comments_save::AfterSave,
    },
}

/// The answer to [`Dialog::UnsavedComments`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnsavedChoice {
    Save,
    Discard,
}

/// A dialog the user answered, with the request's payload carried back alongside
/// the path(s) they chose. A cancelled dialog produces no answer at all.
#[derive(Debug)]
pub(crate) enum DialogAnswer {
    OpenModel(PathBuf),
    ImportTextures(Vec<PathBuf>),
    ExportOpt {
        path: PathBuf,
        result: Arc<ProcessedResult>,
        source: Arc<ModelData>,
        extras: Option<Arc<SourceExtras>>,
        options: ExportOptions,
        comments: Option<review_optimize::NodeStrings>,
    },
    SavePreset {
        path: PathBuf,
        json: String,
    },
    LoadPreset(PathBuf),
    SaveCommentsAs(PathBuf),
    /// The user chose to write over a file that changed on disk.
    ConfirmOverwrite(crate::comments_save::AfterSave),
    UnsavedComments {
        choice: UnsavedChoice,
        then: crate::comments_save::AfterSave,
    },
}

impl Dialog {
    /// A short name for the worker thread, so a Tracy capture says which dialog is
    /// up rather than showing five identically-named threads.
    fn thread_name(&self) -> &'static str {
        match self {
            Dialog::OpenModel => "dialog-open-model",
            Dialog::ImportTextures => "dialog-import-textures",
            Dialog::ExportOpt { .. } => "dialog-export",
            Dialog::SavePreset { .. } => "dialog-save-preset",
            Dialog::LoadPreset => "dialog-load-preset",
            Dialog::SaveCommentsAs { .. } => "dialog-save-comments-as",
            Dialog::ConfirmOverwrite { .. } => "dialog-confirm-overwrite",
            Dialog::UnsavedComments { .. } => "dialog-unsaved-comments",
        }
    }

    /// Run the dialog. **Blocks** for as long as it is on screen, which is why
    /// this is only ever called from the worker [`App::ask`] spawns.
    fn ask(self) -> Option<DialogAnswer> {
        match self {
            Dialog::OpenModel => rfd::FileDialog::new()
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_FBX),
                    &["fbx"],
                )
                .set_title(review_localization::tr(keys::app_dialogs::OPEN_MODEL))
                .pick_file()
                .map(DialogAnswer::OpenModel),
            Dialog::ImportTextures => rfd::FileDialog::new()
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_IMAGE),
                    &TEXTURE_EXTENSIONS,
                )
                .set_title(review_localization::tr(keys::app_dialogs::IMPORT_TEXTURES))
                .pick_files()
                .map(DialogAnswer::ImportTextures),
            Dialog::ExportOpt {
                result,
                source,
                extras,
                options,
                stem,
                comments,
            } => rfd::FileDialog::new()
                .set_title(review_localization::tr(keys::app_dialogs::EXPORT_MESH))
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_FBX),
                    &["fbx"],
                )
                .set_file_name(format!("{stem}.fbx")) // localization: exempt a file name and its extension, not prose
                .save_file()
                .map(|path| DialogAnswer::ExportOpt {
                    path,
                    result,
                    source,
                    extras,
                    options,
                    comments,
                }),
            Dialog::SavePreset { json } => rfd::FileDialog::new()
                .set_title(review_localization::tr(keys::app_dialogs::SAVE_PRESET))
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_PRESET),
                    &[preset::PRESET_EXTENSION],
                )
                .set_file_name(review_localization::tr(keys::app_dialogs::PRESET_FILE_NAME))
                .save_file()
                .map(|path| DialogAnswer::SavePreset { path, json }),
            Dialog::LoadPreset => rfd::FileDialog::new()
                .set_title(review_localization::tr(keys::app_dialogs::LOAD_PRESET))
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_PRESET),
                    &[preset::PRESET_EXTENSION],
                )
                .pick_file()
                .map(DialogAnswer::LoadPreset),
            Dialog::SaveCommentsAs { name } => rfd::FileDialog::new()
                .set_title(review_localization::tr(keys::app_dialogs::SAVE_COMMENTS_AS))
                .add_filter(
                    review_localization::tr(keys::app_dialogs::FILTER_FBX),
                    &["fbx"],
                )
                .set_file_name(name)
                .save_file()
                .map(DialogAnswer::SaveCommentsAs),
            Dialog::ConfirmOverwrite { file, then } => {
                let answer = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title(review_localization::tr(
                        keys::app_dialogs::FILE_CHANGED_TITLE,
                    ))
                    .set_description(keys::app_dialogs::file_changed(file))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                (answer == rfd::MessageDialogResult::Yes)
                    .then_some(DialogAnswer::ConfirmOverwrite(then))
            }
            Dialog::UnsavedComments { file, then } => {
                let save = review_localization::tr(keys::app_dialogs::UNSAVED_SAVE).into_owned();
                let discard =
                    review_localization::tr(keys::app_dialogs::UNSAVED_DISCARD).into_owned();
                let cancel =
                    review_localization::tr(keys::app_dialogs::UNSAVED_CANCEL).into_owned();
                let answer = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title(review_localization::tr(keys::app_dialogs::UNSAVED_TITLE))
                    .set_description(keys::app_dialogs::unsaved(file))
                    .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                        save.clone(),
                        discard.clone(),
                        cancel,
                    ))
                    .show();
                // Backends that can't show custom labels answer with the plain
                // Yes / No they put up instead.
                let choice = match answer {
                    rfd::MessageDialogResult::Custom(label) if label == save => {
                        Some(UnsavedChoice::Save)
                    }
                    rfd::MessageDialogResult::Custom(label) if label == discard => {
                        Some(UnsavedChoice::Discard)
                    }
                    rfd::MessageDialogResult::Yes => Some(UnsavedChoice::Save),
                    rfd::MessageDialogResult::No => Some(UnsavedChoice::Discard),
                    _ => None,
                };
                choice.map(|choice| DialogAnswer::UnsavedComments { choice, then })
            }
        }
    }
}

impl App {
    /// Open a dialog on a worker thread. Its answer arrives later as
    /// [`UserEvent::DialogDone`] and is applied by [`Self::handle_dialog_done`].
    ///
    /// A request that arrives while another dialog is open is dropped: the OS
    /// modal used to make a second one unreachable, and two pickers fighting over
    /// the same click is not an improvement on that.
    pub(crate) fn ask(&mut self, dialog: Dialog) {
        if self.dialog_open {
            return;
        }
        // Without a proxy there is nowhere for the answer to land. `main` installs
        // one before the loop runs, so this is unreachable in a real launch — and
        // the fallback is *not* to run the dialog inline, which is the exact thing
        // this module exists to stop doing.
        let Some(proxy) = self.textures.proxy.clone() else {
            log::error!("no event-loop proxy; cannot open a file dialog");
            self.notifications.error(
                review_localization::tr(keys::app_notifications::DIALOG_FAILED).into_owned(),
            );
            return;
        };

        self.dialog_open = true;
        let name = dialog.thread_name();
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                prof::thread_name(name);
                let answer = dialog.ask();
                // A send failure only means the event loop has exited.
                let _ = proxy.send_event(UserEvent::DialogDone(answer.map(Box::new)));
            });
        if let Err(error) = spawned {
            // The OS would not give us a thread (it is out of them, or of memory).
            // Nothing opened, so nothing is waiting on an answer.
            self.dialog_open = false;
            log::error!("could not start the file dialog thread: {error}");
            self.notifications.error(
                review_localization::tr(keys::app_notifications::DIALOG_FAILED).into_owned(),
            );
        }
    }

    /// Apply a finished dialog on the main thread — the continuation of whichever
    /// request opened it, running exactly where the inline `rfd` call used to
    /// return.
    pub(crate) fn handle_dialog_done(&mut self, answer: Option<Box<DialogAnswer>>) {
        self.dialog_open = false;
        let Some(answer) = answer else {
            // Cancelled: nothing to apply, and nothing on screen changed.
            return;
        };
        match *answer {
            DialogAnswer::OpenModel(path) => self.open_model_from_path(&path),
            DialogAnswer::ImportTextures(files) => {
                for path in files {
                    self.import_texture_path(path);
                }
            }
            DialogAnswer::ExportOpt {
                path,
                result,
                source,
                extras,
                options,
                comments,
            } => self.spawn_opt_export(path, result, source, extras, options, comments),
            DialogAnswer::SavePreset { path, json } => self.write_opt_preset(&path, &json),
            DialogAnswer::LoadPreset(path) => self.read_opt_preset(&path),
            DialogAnswer::SaveCommentsAs(path) => {
                self.write_comments(path, crate::comments_save::AfterSave::Nothing, true)
            }
            DialogAnswer::ConfirmOverwrite(then) => self.save_comments_unchecked(then),
            DialogAnswer::UnsavedComments { choice, then } => match choice {
                UnsavedChoice::Save => self.save_comments(then),
                UnsavedChoice::Discard => {
                    // Treated as saved, so what comes next doesn't ask again.
                    self.ui.comments.mark_saved();
                    self.resume(then);
                }
            },
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
