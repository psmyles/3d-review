//! `render`: headless pictures of models through the viewer's own renderer.
//!
//! The GPU work is the render engine's (`crates/snapshot`), a binary shipped
//! beside this one (`review3d-render`; `review-snapshot` in a development
//! build). It is a separate process so that every other command stays free of
//! a GPU: linking the renderer here would make `optimize` on a headless build
//! machine need a Vulkan loader it never uses. One call still starts the GPU
//! once for every model it is given.
//!
//! The arguments after `render` go to the engine as they are; its exit status
//! is this command's. `review3d-cli render --help` is the engine's own help.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use crate::console;
use crate::keys::cli as k;

/// The engine's names, as shipped and as a development build makes it.
const ENGINES: [&str; 2] = ["review3d-render", "review-snapshot"];

/// The render engine beside this binary, if there is one.
fn engine() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    ENGINES.iter().find_map(|name| {
        let path = folder.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        path.is_file().then_some(path)
    })
}

/// Run `render` with the arguments after it.
pub(crate) fn run(arguments: &[OsString]) -> ExitCode {
    let Some(engine) = engine() else {
        console::warn(&review_localization::tr(k::RENDER_NO_ENGINE));
        return ExitCode::from(2);
    };
    match Command::new(&engine).args(arguments).status() {
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(error) => {
            console::warn(&k::render_engine_failed(
                error.to_string(),
                engine.display().to_string(),
            ));
            ExitCode::FAILURE
        }
    }
}
