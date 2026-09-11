//! Helpers shared by this crate's integration suites.
//!
//! Every suite loads a fixture, runs a stack over it and (for the export ones)
//! reads the written file back, and each had grown its own copy of that
//! scaffolding. The copies mattered because of the **skip policy**: a checkout
//! without `assets/test_models`, without vendored ufbx or without a capture must
//! print a reason and pass rather than fail, and deciding what counts as
//! skippable is one rule — it was four.
//!
//! Each suite uses a subset of what is here, so unused items are expected: a
//! `tests/common` module is compiled into every test binary that declares it.

#![allow(dead_code)]

pub mod extras;

use std::path::{Path, PathBuf};

use review_model::{ModelData, SourceExtras};
use review_optimize::{OptStack, ProcessInput, ProcessedResult, process};

/// Stand-in for the renderer's `SceneVertex` size (position, normal, uv,
/// tangent, color). Only the overfetch figure depends on it.
pub const VERTEX_SIZE: usize = 64;

/// A per-test temporary directory under the target dir, removed and recreated so
/// a rerun never sees the previous run's files.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp directory");
    dir
}

/// Where a named fixture lives, whether or not it is present.
pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name)
}

/// A fixture, or `None` when this checkout cannot run the test.
///
/// A checkout without the vendored ufbx sources can't import *anything*; that is
/// the same "this checkout can't run these" situation as a missing fixture. Any
/// other error means the file is there and did not load, which the suite must
/// report rather than skip past.
pub fn fixture(name: &str) -> Option<ModelData> {
    let path = fixture_path(name);
    if !path.exists() {
        eprintln!("skipping {name}: the fixture is not present");
        return None;
    }
    match review_import::load_model(&path) {
        Ok(model) => Some(model),
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping {name}: FBX import is unavailable in this build");
            None
        }
        Err(error) => panic!("{name} is present but failed to load: {error}"),
    }
}

/// A fixture with its source-property capture, or `None` when this checkout
/// cannot run the test.
pub fn fixture_full(name: &str) -> Option<(ModelData, SourceExtras)> {
    let path = fixture_path(name);
    if !path.exists() {
        eprintln!("skipping {name}: the fixture is not present");
        return None;
    }
    load_full(&path)
}

/// Load a file with its capture, skipping when the build cannot produce one.
pub fn load_full(path: &Path) -> Option<(ModelData, SourceExtras)> {
    match review_import::load_model_full(path) {
        Ok((model, Some(extras))) => Some((model, extras)),
        Ok((_, None)) => {
            eprintln!("skipping {}: no capture in this build", path.display());
            None
        }
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping {}: FBX import is unavailable", path.display());
            None
        }
        Err(error) => panic!("{} is present but failed to load: {error}", path.display()),
    }
}

/// Re-import a written file with its capture. Unlike [`load_full`] this is for a
/// file *we just wrote*, so failing to read it back is a test failure.
pub fn reload_full(path: &Path) -> (ModelData, SourceExtras) {
    load_full(path).unwrap_or_else(|| panic!("could not read back {}", path.display()))
}

/// Re-import a written file, failing with the reader's own message.
pub fn reimport(path: &Path) -> ModelData {
    review_import::load_model(path)
        .unwrap_or_else(|error| panic!("could not read back {}: {error}", path.display()))
}

/// Run a stack over a model.
pub fn run(model: &ModelData, stack: &OptStack) -> ProcessedResult {
    process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: None,
    })
    .expect("processing succeeds")
}

/// Run a stack over a model that carries a source-property capture.
pub fn run_with_extras(
    model: &ModelData,
    extras: &SourceExtras,
    stack: &OptStack,
) -> ProcessedResult {
    process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: Some(extras),
    })
    .expect("processing succeeds")
}

/// The first line containing `needle`, or a panic naming it.
pub fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}"))
}

/// The first line containing every needle. A property template in the
/// Definitions block precedes the element that overrides it, so a value check
/// has to name the value as well as the property.
pub fn line_with_all<'a>(text: &'a str, needles: &[&str]) -> &'a str {
    text.lines()
        .find(|line| needles.iter().all(|needle| line.contains(needle)))
        .unwrap_or_else(|| panic!("no line contains all of {needles:?}"))
}

/// The `P: "<name>", ...,<value>` property value, as written.
pub fn scene_property(text: &str, name: &str) -> String {
    let needle = format!("P: \"{name}\"");
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&needle))
        .unwrap_or_else(|| panic!("the file declares {name}"));
    line.rsplit(',')
        .next()
        .expect("a property carries a value")
        .to_owned()
}

/// The count out of an ASCII FBX array header, e.g. `12888 {` -> 12888.
pub fn array_size(rest: &str) -> usize {
    rest.split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
        .expect("an array header carries its length")
}
