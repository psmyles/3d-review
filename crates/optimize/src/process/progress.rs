//! Where a run has got to, reported as it goes.
//!
//! A stack run is one call that can take a minute — Remesh and Shrinkwrap
//! rebuild a whole surface per object, seconds apiece on a dense one — and
//! until this existed the only thing on screen for all of it was
//! "Optimizing mesh...". That says nothing about whether the run is close, and
//! nothing about whether it is moving at all, which is the question a user
//! actually has when they are looking at it.
//!
//! Shaped exactly like `review_import`'s `ImportProgress`, for the same reasons
//! and with the same rules: the sink is called on the worker thread, so it must
//! be cheap and must not block (`app` throttles and forwards to the event loop),
//! and [`OptStage::label`] stays English because it goes down the profiling
//! channel — the side that draws text maps it to a catalog key (invariant 12).

use crate::cancel::CancelToken;
use crate::stack::OpKind;
use crate::submesh::Submesh;

/// The phase a run is in.
///
/// Ordered as they happen. Only [`Self::Operation`] repeats, once per enabled
/// operation — and, for the operations that rebuild a surface object by object,
/// it reports which object as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptStage {
    /// Splitting the model into per-(node, material) pieces and merging the
    /// corner-split vertices back together (`process::index_mesh`). No
    /// denominator.
    Preparing,
    /// The baseline counts and cache / overdraw / fetch figures every level's
    /// change is quoted against. No denominator.
    Measuring,
    /// One of the user's operations, named by [`OptProgress::op`]. Carries a
    /// `done`/`total` over objects for the operations that work one object at a
    /// time (Remesh, Shrinkwrap); the rest are a single step.
    Operation,
    /// Rebuilding one level's `ModelData` and measuring it; `done`/`total`
    /// count levels.
    Assembling,
}

impl OptStage {
    /// The English verb for this stage, for logs and profiling captures.
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing the mesh",
            Self::Measuring => "Measuring the source",
            Self::Operation => "Running",
            Self::Assembling => "Rebuilding",
        }
    }
}

/// One progress report from a run.
///
/// Borrowed rather than owned so reporting costs nothing when nobody is
/// listening: `process` hands out references into the stack and the model it is
/// already holding, and only a sink that renders a line allocates.
#[derive(Debug, Clone, Copy)]
pub struct OptProgress<'a> {
    pub stage: OptStage,
    /// The operation being applied, for [`OptStage::Operation`].
    pub op: Option<&'a OpKind>,
    /// The object it is on right now, for an operation that rebuilds one
    /// surface at a time. A node name, so it is the user's own word for the
    /// thing they are waiting on.
    pub object: Option<&'a str>,
    /// How far through this stage, in whatever this stage counts. `total` is 0
    /// when the stage has no denominator, in which case only the stage and its
    /// operation are meaningful.
    pub done: u32,
    pub total: u32,
}

impl<'a> OptProgress<'a> {
    /// A stage with no denominator.
    pub fn stage(stage: OptStage) -> Self {
        Self {
            stage,
            op: None,
            object: None,
            done: 0,
            total: 0,
        }
    }

    /// An operation starting, with no per-object detail.
    pub fn operation(op: &'a OpKind) -> Self {
        Self {
            stage: OptStage::Operation,
            op: Some(op),
            object: None,
            done: 0,
            total: 0,
        }
    }

    /// An operation part way through its objects.
    pub fn object(op: &'a OpKind, object: &'a str, done: u32, total: u32) -> Self {
        Self {
            stage: OptStage::Operation,
            op: Some(op),
            object: Some(object),
            done,
            total,
        }
    }

    /// An operation part way through units that have no name of their own —
    /// the LOD fan-out's levels.
    pub fn operation_step(op: &'a OpKind, done: u32, total: u32) -> Self {
        Self {
            stage: OptStage::Operation,
            op: Some(op),
            object: None,
            done,
            total,
        }
    }

    /// A counted stage with no operation behind it.
    pub fn counted(stage: OptStage, done: u32, total: u32) -> Self {
        Self {
            stage,
            op: None,
            object: None,
            done,
            total,
        }
    }

    /// How far through this stage the run is, `None` when it can't be known.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
}

/// What [`crate::process::process_with_progress`] reports through. Called on the
/// processing thread, so it must be cheap and must not block.
pub type OptProgressSink<'a> = &'a dyn Fn(OptProgress<'_>);

/// What a part-finished scene is handed to, as the pieces it is made of. The
/// caller turns them into a mesh; this side has no opinion on how.
pub type PieceSink<'a> = &'a dyn Fn(&[&Submesh]);

/// The two things every step of a run needs from its caller: somewhere to
/// report to, and a way to ask whether it is still wanted.
///
/// One value rather than two parameters carried side by side down the call
/// chain — `apply_op` already takes the submeshes, the operation, the stack,
/// the model, the hidden set and the warnings, and a seventh and eighth
/// positional argument is where a signature stops being readable.
#[derive(Clone, Copy)]
pub struct RunContext<'a> {
    progress: OptProgressSink<'a>,
    cancel: Option<&'a CancelToken>,
    /// Where a part-finished mesh goes, when anyone is watching. Called on the
    /// thread that owns the run, never from a worker.
    preview: Option<PieceSink<'a>>,
}

impl<'a> RunContext<'a> {
    pub fn new(progress: OptProgressSink<'a>, cancel: Option<&'a CancelToken>) -> Self {
        Self {
            progress,
            cancel,
            preview: None,
        }
    }

    /// The same context, with somewhere to send part-finished meshes.
    pub fn watching(self, preview: PieceSink<'a>) -> Self {
        Self {
            preview: Some(preview),
            ..self
        }
    }

    /// Whether anything is watching. Checked before a preview is *built*, since
    /// splicing one together costs more than handing it over.
    pub(crate) fn wants_preview(&self) -> bool {
        self.preview.is_some()
    }

    /// Show the scene as it currently stands.
    pub(crate) fn preview(&self, pieces: &[&Submesh]) {
        if let Some(sink) = self.preview {
            sink(pieces);
        }
    }

    /// Say where the run has got to.
    pub(crate) fn report(&self, progress: OptProgress<'_>) {
        (self.progress)(progress);
    }

    /// Whether this run has been superseded and should stop.
    pub(crate) fn cancelled(&self) -> bool {
        crate::cancel::cancelled(self.cancel)
    }

    /// The token itself, for the places that hand it to a worker thread — the
    /// sink cannot go there (it is a `&dyn Fn` that posts to the event loop),
    /// but the token is `Sync` by construction.
    pub(crate) fn token(&self) -> Option<&'a CancelToken> {
        self.cancel
    }
}

/// The sink [`crate::process::process`] uses: a run nobody is watching.
pub(crate) fn ignore(_progress: OptProgress<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_without_a_denominator_has_no_fraction() {
        assert_eq!(OptProgress::stage(OptStage::Preparing).fraction(), None);
    }

    #[test]
    fn a_counted_stage_reports_its_fraction() {
        let progress = OptProgress::counted(OptStage::Assembling, 1, 4);
        assert_eq!(progress.fraction(), Some(0.25));
    }

    #[test]
    fn a_fraction_past_the_end_is_clamped() {
        let progress = OptProgress::counted(OptStage::Assembling, 9, 4);
        assert_eq!(progress.fraction(), Some(1.0));
    }
}
