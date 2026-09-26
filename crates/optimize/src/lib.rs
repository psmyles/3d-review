//! Mesh optimization for the Opt workspace: a safe Rust surface over vendored
//! [meshoptimizer](https://github.com/zeux/meshoptimizer), plus the operation
//! stack the UI edits and the pipeline that turns one into a LOD chain.
//!
//! ## Layering
//!
//! * [`stack`] — the user's setup as plain, serializable data. No mesh state.
//! * [`process`] — interprets a stack against a `ModelData`, producing one
//!   `ModelData` per LOD level plus measured metrics.
//! * [`submesh`] / [`ops`] — the pieces `process` is built from.
//! * [`meshopt`] / `ffi` — the checked wrappers and the raw C declarations.
//! * [`preset`] — stack ↔ JSON.
//!
//! This crate depends only on `review-model` (and `glam` through it), matching
//! `review-import`: it hands back plain `ModelData` and never touches GPU or UI
//! types, so `render` stays the only thing that knows about buffers.
//!
//! ## `unsafe`
//!
//! Invariant 9 confines `unsafe`/FFI to a named set of sites; this crate is one
//! of them (see docs/ARCHITECTURE.md). All of it lives in `ffi` (declarations) and
//! [`meshopt`] (checked wrappers that validate every buffer and index before the
//! call and re-validate the reported sizes after). Nothing above that layer —
//! [`ops`], [`process`], [`stack`], [`preset`] — contains any.
//!
//! ## Optional vendoring
//!
//! Like `review-import` and its ufbx tree, the vendored sources are optional:
//! without `third_party/meshoptimizer` the crate still compiles and every
//! operation reports [`OptError::Unavailable`].

// Everything outside the FFI modules is ordinary safe Rust (invariant 9). The
// crate refuses `unsafe`, and those modules opt in individually with a
// module-level `allow` saying why.
#![deny(unsafe_code)]

mod export_ffi;
mod ffi;
mod prof;

#[cfg(has_ufbxw_probe)]
#[doc(hidden)]
pub mod probe;

mod ao;
mod parallel;
mod shading;

pub mod cancel;

pub mod export;
pub mod meshopt;
pub mod notice;
pub mod ops;
pub mod preset;
pub mod process;
pub mod remesh;
pub mod replace_file;
pub mod shrinkwrap;
pub mod stack;
pub mod submesh;

pub use cancel::CancelToken;
pub use export::{ExportReport, export_fbx};
pub use notice::{ExportNote, OptWarning};
pub use process::{
    AnalysisMetrics, MeshCounts, OptPreview, OptPreviewSink, OptProgress, OptProgressSink,
    OptStage, ProcessInput, ProcessedLod, ProcessedResult, process, process_cancellable,
    process_progressive, process_with_progress,
};
pub use replace_file::{Staged, write_bytes_replacing, write_replacing};
pub use stack::{
    AoQuality, AoTarget, AttributeWeights, BakeAoParams, ExportOptions, FbxFormat, HierarchyMode,
    LodLevel, LodPackaging, LodParams, NodeOverride, NormalMode, NormalParams, OpInstance, OpKind,
    OptStack, RebindReport, ReduceParams, RemeshDensity, RemeshParams, RemeshTopology,
    ShrinkwrapMethod, ShrinkwrapParams, SimplifyAlgorithm, SimplifyFlags, SimplifySettings,
    VoxelTarget, WeldParams,
};

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum OptError {
    #[error(
        "mesh optimization is unavailable: this build has no vendored meshoptimizer \
         (third_party/meshoptimizer)"
    )]
    Unavailable,

    #[error("the remesher could not rebuild this object: {0}")]
    Remesh(String),

    #[error("the mesh has no triangles to process")]
    EmptyMesh,

    /// Not a failure: the stack was edited while this run was going, so it
    /// stopped rather than finish a result nobody would see. `app` drops it
    /// silently - the run that superseded it is already queued.
    #[error("the run was cancelled")]
    Cancelled,

    #[error("index buffer has {0} indices, which is not a whole number of triangles")]
    IndexCount(usize),

    #[error("index {index} addresses no vertex in a buffer of {vertex_count}")]
    IndexRange { index: u32, vertex_count: usize },

    #[error("vertex stream has {len} values, expected {expected}")]
    StreamLength { len: usize, expected: usize },

    #[error("vertex stream contains a non-finite value")]
    NonFiniteStream,

    /// A numeric setting outside what the library accepts. meshoptimizer
    /// `assert`s on these, and its asserts abort the process, so they are
    /// refused before the call instead.
    #[error("{name} of {value} is out of range")]
    InvalidParameter { name: &'static str, value: f32 },

    #[error("buffer size overflowed while sizing a destination")]
    SizeOverflow,

    #[error("meshoptimizer reported {produced} elements for a destination of {capacity}")]
    BadResult { produced: usize, capacity: usize },

    #[error("preset could not be read or written: {0}")]
    Preset(String),

    #[error("preset was written by a newer build (version {found}, this build reads {supported})")]
    PresetVersion { found: u32, supported: u32 },

    #[error("FBX export failed: {0}")]
    Export(String),

    /// A multi-file export that wrote everything but could not put all of it in
    /// place. Carries what *was* replaced, because the user needs to know which
    /// of their assets on disk are now from this run and which are not — an
    /// error that only says "export failed" leaves them to guess.
    #[error("the export replaced {} file(s) but could not replace {}: {reason}", replaced.len(), failed.display())]
    ExportIncomplete {
        replaced: Vec<std::path::PathBuf>,
        failed: std::path::PathBuf,
        reason: String,
    },
}

/// Filesystem failures reach the user as export failures: every one of them in
/// this crate happens while writing a file out.
impl From<std::io::Error> for OptError {
    fn from(error: std::io::Error) -> Self {
        OptError::Export(error.to_string())
    }
}

/// A de-duplicating warning collector.
///
/// Processing walks every submesh, so a problem with one operation typically
/// recurs once per object. Reporting "Weld Vertices: …" fifty times would bury
/// the rest; the user needs to know *that* it happened, once.
#[derive(Debug, Default)]
pub struct Warnings {
    warnings: Vec<OptWarning>,
}

impl Warnings {
    pub fn push(&mut self, warning: OptWarning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    pub fn into_vec(self) -> Vec<OptWarning> {
        self.warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_report_a_repeated_problem_once() {
        let mut warnings = Warnings::default();
        warnings.push(OptWarning::LodEmpty { level: 1 });
        warnings.push(OptWarning::LodEmpty { level: 1 });
        warnings.push(OptWarning::LodEmpty { level: 2 });

        assert_eq!(warnings.into_vec().len(), 2);
    }

    #[test]
    fn availability_matches_the_vendored_tree() {
        assert_eq!(meshopt::available(), cfg!(has_meshopt));
        // The rebuild is ordinary Rust with nothing vendored behind it, so it
        // is always there - which is itself worth pinning, since it used to
        // depend on two C++ trees being present.
        assert!(remesh::available());
    }
}
