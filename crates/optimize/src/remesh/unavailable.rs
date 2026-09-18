//! What remeshing does without a vendored retopologizer.
//!
//! `third_party/instant-meshes` (and the `third_party/eigen` it needs) are
//! optional exactly as `third_party/meshoptimizer` is: delete either tree and
//! the workspace still builds, with the Remesh operation reporting
//! [`OptError::RemeshUnavailable`] as a per-operation warning. The rest of the
//! stack runs, so the level still appears — this is the same failure policy every
//! other operation follows, not a whole-run failure.
//!
//! In its own file rather than a `cfg`-else beside the call, so the two halves of
//! the signature cannot drift (the pattern `meshopt/unavailable.rs` set).

#![cfg(not(has_instant_meshes))]

use crate::OptError;

use super::{RemeshInput, RemeshOptions, RemeshOutput};

pub(super) fn run(
    _input: &RemeshInput<'_>,
    _options: &RemeshOptions,
) -> Result<RemeshOutput, OptError> {
    Err(OptError::RemeshUnavailable)
}
