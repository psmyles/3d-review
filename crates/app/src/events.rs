//! The one cross-module message bus: everything a worker thread, a watcher or
//! the shell posts back into the event loop.
//!
//! `winit` delivers these to `App::user_event`, which is the only place they are
//! handled — a background job never touches `App` directly.

use std::path::PathBuf;

use crate::texture_manager::TextureDecode;
use crate::{dialog, loading, opt, update};

/// Custom event posted from a background thread to the winit event loop, so work
/// done off the main thread is applied back on it (the redraw loop + all renderer
/// state stay in `app` — invariant 6).
///
/// Deliberately not `Clone`: a variant carries a whole LOD chain of meshes, and
/// an accidental clone would deep-copy every one of them (invariant 1).
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// A watched directory reported a change to this path; if it's a bound texture,
    /// re-decode + re-upload it (posted by the file-watcher thread).
    TextureChanged(PathBuf),
    /// A background texture decode finished (posted by the decode thread). The
    /// result is uploaded + the slot/binding updated here on the main thread.
    TextureDecoded(TextureDecode),
    /// A background Opt processing run finished (posted by the optimize thread).
    /// Boxed because a `ProcessedResult` carries a mesh per LOD level, which
    /// would otherwise make every variant of this enum that large.
    OptProcessed(Box<opt::OptProcessed>),
    /// A background Opt run reached a new step, or moved within one (posted by
    /// the optimize thread, already throttled there - see `opt.rs`). Rewrites
    /// the optimizing card's stage line in place.
    OptProgressed(opt::OptProgressed),
    /// A background Opt run produced a mesh part way through (posted by the
    /// optimize thread, already throttled there). Drawn in place of the
    /// finished level until that run lands. Boxed: it carries a whole model.
    OptPreviewed(Box<opt::OptPreviewed>),
    /// A background FBX export finished (posted by the export thread).
    OptExported(Box<Result<review_optimize::ExportReport, review_optimize::OptError>>),
    /// A background model import produced a drawable model (posted by the import
    /// thread). Boxed because it carries the whole parsed model.
    ModelLoaded(Box<loading::ModelLoaded>),
    /// A background model import reached a new stage, or moved within one
    /// (posted by the import thread, already throttled there — see `loading.rs`).
    /// Rewrites the loading card's stage line in place.
    ModelLoadProgress(loading::ModelLoadProgress),
    /// One of the measurements the import deferred until after the model was on
    /// screen has landed (posted by the same thread, which keeps measuring once
    /// it has published the mesh). Boxed because the draw-group table is one
    /// entry per (node, material) pair.
    ModelMeasured(Box<loading::ModelMeasured>),
    /// The source-property capture of the model on screen has been marshaled
    /// (posted by the import thread right after the mesh). Carried for the
    /// exporter; nothing in the viewport reads it.
    SourceExtrasReady(Box<loading::SourceExtrasReady>),
    /// A native file dialog closed (posted by the thread that opened it —
    /// `mac-port-plan.md` D9). `None` when the user cancelled. Boxed because the
    /// export variant carries a whole LOD chain's worth of `Arc`s.
    DialogDone(Option<Box<dialog::DialogAnswer>>),
    /// Help > Check for Updates has its answer (posted by the thread that asked
    /// GitHub - see `update.rs`).
    UpdateChecked(update::UpdateCheck),
    /// The OS asked for a file to be opened (`mac-port-plan.md` D14): a Finder
    /// double-click, an `open(1)`, or a drop on the Dock icon. macOS only —
    /// Windows delivers the same intent as `argv[1]`, which `main` reads directly.
    OpenPath(PathBuf),
    /// A macOS menu item the viewer performs itself was chosen (D15). Routed
    /// through the loop rather than acted on in muda's callback so it lands on the
    /// main thread, in order with every other event, instead of racing the state
    /// it is about to change.
    MenuCommand(review_shell_macos::MenuCommand),
    /// A line was logged while the Log window is open (posted by whichever
    /// thread logged it, at most once until the window next reads the log).
    /// Asks for a frame, which is where the window is handed the line.
    LogUpdated,
}
