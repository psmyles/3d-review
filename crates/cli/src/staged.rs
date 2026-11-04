//! `--staged`: the models staged for the next commit, checked as they are
//! staged (AC15).
//!
//! Everything git is `asset-rules`' (`asset_rules::git`), which shells out to
//! the `git` on PATH - so the hook, Fork and CI read the index the way git
//! itself does, `GIT_INDEX_FILE` included. A staged model whose working file
//! has changed since it was staged is read from a temporary copy of what is
//! staged; one that has not is read in place.

use std::path::Path;

use asset_rules::git::{Repo, StagedFile};

use crate::inputs::is_fbx;
use crate::keys::cli as k;

/// The repository the current folder is in, and its staged models.
pub(crate) struct Staged {
    pub(crate) repo: Repo,
    /// Added or changed in the index, in path order. Only `.fbx` files: every
    /// other staged file is another tool's or nobody's.
    pub(crate) models: Vec<StagedFile>,
}

/// The staged models of the repository the current folder is in, or why they
/// could not be listed.
pub(crate) fn gather() -> Result<Staged, String> {
    let failed = |error: std::io::Error| k::staged_failed(error.to_string());
    let repo = Repo::discover(Path::new(".")).map_err(failed)?;
    // Copies a killed run never removed. Nothing depends on this succeeding.
    let _ = repo.prune_copies();
    let models = repo
        .staged()
        .map_err(failed)?
        .into_iter()
        .filter(|file| is_fbx(Path::new(&file.path)))
        .collect();
    Ok(Staged { repo, models })
}
