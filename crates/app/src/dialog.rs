//! Native file dialogs, opened on a worker thread and answered back through the
//! event loop (`mac-port-plan.md` D9).
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
    },
    /// Choose where to write an operation-stack preset. The JSON is serialized
    /// before the dialog opens, so it describes the stack the user was looking at.
    SavePreset { json: String },
    /// Pick an operation-stack preset to load.
    LoadPreset,
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
    },
    SavePreset {
        path: PathBuf,
        json: String,
    },
    LoadPreset(PathBuf),
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
        }
    }

    /// Run the dialog. **Blocks** for as long as it is on screen, which is why
    /// this is only ever called from the worker [`App::ask`] spawns.
    fn ask(self) -> Option<DialogAnswer> {
        match self {
            Dialog::OpenModel => rfd::FileDialog::new()
                .add_filter("FBX", &["fbx"])
                .set_title("Open Model")
                .pick_file()
                .map(DialogAnswer::OpenModel),
            Dialog::ImportTextures => rfd::FileDialog::new()
                .add_filter("Image", &TEXTURE_EXTENSIONS)
                .set_title("Import Textures")
                .pick_files()
                .map(DialogAnswer::ImportTextures),
            Dialog::ExportOpt {
                result,
                source,
                extras,
                options,
                stem,
            } => rfd::FileDialog::new()
                .set_title("Export optimized mesh")
                .add_filter("FBX", &["fbx"])
                .set_file_name(format!("{stem}.fbx"))
                .save_file()
                .map(|path| DialogAnswer::ExportOpt {
                    path,
                    result,
                    source,
                    extras,
                    options,
                }),
            Dialog::SavePreset { json } => rfd::FileDialog::new()
                .set_title("Save optimization preset")
                .add_filter("Optimization preset", &[preset::PRESET_EXTENSION])
                .set_file_name("optimization-preset.json")
                .save_file()
                .map(|path| DialogAnswer::SavePreset { path, json }),
            Dialog::LoadPreset => rfd::FileDialog::new()
                .set_title("Load optimization preset")
                .add_filter("Optimization preset", &[preset::PRESET_EXTENSION])
                .pick_file()
                .map(DialogAnswer::LoadPreset),
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
            prof::msg("no event-loop proxy; cannot open a file dialog");
            self.notifications
                .error("Couldn't open the file dialog".to_owned());
            return;
        };

        self.dialog_open = true;
        let name = dialog.thread_name();
        std::thread::spawn(move || {
            prof::thread_name(name);
            let answer = dialog.ask();
            // A send failure only means the event loop has exited.
            let _ = proxy.send_event(UserEvent::DialogDone(answer.map(Box::new)));
        });
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
            } => self.spawn_opt_export(path, result, source, extras, options),
            DialogAnswer::SavePreset { path, json } => self.write_opt_preset(&path, &json),
            DialogAnswer::LoadPreset(path) => self.read_opt_preset(&path),
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
