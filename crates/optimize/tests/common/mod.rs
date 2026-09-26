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
//!
//! Set `REVIEW_REQUIRE_FIXTURES=1` to turn every skip into a failure. A skip that
//! passes is right for a fresh checkout and wrong for the run that decides
//! whether the tree is good: without it a green suite means "nothing was found to
//! run" just as readily as "everything passed", and the two are indistinguishable
//! from the outside. `scripts/check` sets it.

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

/// Refuse to skip, or say why this run is skipping.
///
/// Returns `None` so a caller can `let Some(..) = .. else { return }`; panics
/// instead when `REVIEW_REQUIRE_FIXTURES` is set, naming the same reason.
pub fn skip(reason: &str) -> Option<std::convert::Infallible> {
    if std::env::var_os("REVIEW_REQUIRE_FIXTURES").is_some_and(|value| value != "0") {
        panic!("REVIEW_REQUIRE_FIXTURES is set, so this run may not skip: {reason}");
    }
    eprintln!("skipping: {reason}");
    None
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
        skip(&format!("{name} is not present"))?;
        return None;
    }
    match review_import::load_model(&path) {
        Ok(model) => Some(model),
        Err(review_import::ImportError::UfbxUnavailable) => {
            skip(&format!("{name}: FBX import is unavailable in this build"))?;
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
        skip(&format!("{name} is not present"))?;
        return None;
    }
    load_full(&path)
}

/// Load a file with its capture, skipping when the build cannot produce one.
pub fn load_full(path: &Path) -> Option<(ModelData, SourceExtras)> {
    match review_import::load_model_full(path) {
        Ok((model, Some(extras))) => Some((model, extras)),
        Ok((_, None)) => {
            skip(&format!("{}: no capture in this build", path.display()))?;
            None
        }
        Err(review_import::ImportError::UfbxUnavailable) => {
            skip(&format!("{}: FBX import is unavailable", path.display()))?;
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

/// Compare `digest` against whatever another test binary recorded under `name`,
/// recording it instead when this is the first one to get there.
///
/// The two suites that use this run the *same* stack over the same fixture at
/// different thread counts, so the mesh must be identical; which of them runs
/// first is not defined, and does not need to be. When only one of them runs —
/// a filtered `cargo test`, or a build with no retopologizer — nothing is
/// compared, which is the honest outcome rather than a failure.
///
/// A recording older than this executable was is **ignored**, not compared
/// against. The file lives in the target directory and outlives any number of
/// rebuilds, so without that rule the first legitimate change to the remesher
/// fails this test against a mesh that no longer exists — a failure that says
/// "the result depends on scheduling" when it does not, and that a reader can
/// only clear by guessing at `cargo clean`.
pub fn compare_digest_across_binaries(name: &str, digest: u64) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shared_digests");
    std::fs::create_dir_all(&dir).expect("create the shared digest directory");
    let path = dir.join(name);
    let fresh = |path: &Path| -> bool {
        let built = std::env::current_exe()
            .and_then(|exe| exe.metadata())
            .and_then(|meta| meta.modified());
        let recorded = path.metadata().and_then(|meta| meta.modified());
        match (built, recorded) {
            (Ok(built), Ok(recorded)) => recorded >= built,
            // No clock to compare with: treat it as fresh and compare, which
            // fails loudly rather than passing silently.
            _ => true,
        }
    };
    match std::fs::read_to_string(&path) {
        Ok(recorded) if fresh(&path) => {
            let recorded: u64 = recorded.trim().parse().expect("a recorded digest");
            assert_eq!(
                recorded, digest,
                "{name}: this run produced a different mesh from the one the sibling \
                 suite recorded, so the result depends on how the work was scheduled"
            );
        }
        _ => std::fs::write(&path, digest.to_string()).expect("record the digest"),
    }
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
