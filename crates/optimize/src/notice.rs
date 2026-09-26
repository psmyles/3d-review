//! What a run or an export has to say about its result, as values rather than
//! sentences.
//!
//! Invariant 12 keeps every string a user reads in the catalog, and this crate
//! must not know about catalogs. So a warning is an [`OptWarning`] and an export
//! note an [`ExportNote`], each carrying the figures and names its sentence
//! needs; `app` renders them through catalog messages, matching exhaustively, so
//! a variant added here is a compile error there rather than English on a
//! translated screen.
//!
//! `Display` is the English diagnostic - what the log records and what the tests
//! read - in the same words the notices used before they were catalogued.

use std::fmt;

use crate::stack::OpKind;

/// A condition of the mesh or the stack worth telling the user about. Each is
/// reported once per run however often it arises ([`crate::Warnings`]).
#[derive(Debug, Clone, PartialEq)]
pub enum OptWarning {
    /// The processed level's skin / blend-shape tables did not validate, so they
    /// were dropped from the level and its export. `detail` is the validator's
    /// diagnostic.
    DeformDropped { detail: String },
    /// Part of the mesh could not be measured, so the cache, overdraw and fetch
    /// figures cover only the rest.
    MeasureFailed { detail: String },
    /// The lossless index pass at the start of a run failed.
    IndexFailed { detail: String },
    /// An operation failed on part of the mesh.
    OperationFailed { operation: OpKind, detail: String },
    /// Bake AO sits above a simplifier, so it baked the pre-simplified geometry.
    BakeBeforeSimplify,
    /// A simplifier below Remesh rebuilt the quads Remesh produced as triangles.
    QuadsLostToSimplify,
    /// Shrinkwrap sits below an operation that changes the shape, whose work it
    /// then throws away.
    ShrinkwrapBelowShapeChange,
    /// A LOD level simplified away to nothing.
    LodEmpty { level: usize },
    /// A simplify removed almost nothing because every edge is an attribute seam.
    SimplifierStalledOnSeams,
    /// The tangent rebuild failed.
    TangentsFailed { detail: String },
    /// Recalculate Normals skipped an object with blend shapes.
    NormalsKeptForBlendShapes { object: String },
    /// Recalculate Normals failed on an object.
    NormalsFailed { object: String, detail: String },
    /// Shrinkwrap skipped a skinned or blend-shaped object.
    ShrinkwrapKeptDeforming { object: String },
    /// Shrinkwrap skipped an object too small to wrap.
    ShrinkwrapTooFewTriangles { object: String, triangles: usize },
    /// The distance-field wrap produced nothing: the object is thinner than a voxel.
    ShrinkwrapEmptyField { object: String, resolution: u32 },
    /// The voxel wrap produced nothing.
    ShrinkwrapEmptyVoxels { object: String, resolution: u32 },
    /// The voxel wrap's remesh refused an object.
    ShrinkwrapVoxelFailed { object: String, detail: String },
    /// Shrinkwrap ran at a lower resolution than asked, to bound its memory.
    ShrinkwrapResolutionLowered {
        object: String,
        requested: u32,
        resolution: u32,
    },
    /// Remesh skipped a skinned or blend-shaped object.
    RemeshKeptDeforming { object: String },
    /// Remesh skipped an object too small to rebuild.
    RemeshTooFewTriangles { object: String, triangles: usize },
    /// Remesh failed on an object and left it as it was.
    RemeshFailed { object: String, detail: String },
    /// Remesh was asked for more faces than the object has; a rebuild only merges.
    RemeshBudgetAboveInput {
        object: String,
        triangles: usize,
        asked: u32,
    },
    /// Remesh produced nothing for an object.
    RemeshEmpty { object: String },
    /// Some of an object's regions would not collapse as far as asked.
    RemeshStubborn { object: String, parts: usize },
    /// A quad rebuild came back mostly triangles.
    RemeshFewQuads {
        object: String,
        quads: usize,
        faces: usize,
    },
}

impl fmt::Display for OptWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeformDropped { detail } => write!(
                f,
                "The processed mesh's skin / blend-shape data did not reconcile ({detail}); it \
                 was dropped from this level and its export."
            ),
            Self::MeasureFailed { detail } => write!(
                f,
                "Couldn't measure part of the mesh: {detail}. The cache, overdraw and fetch \
                 figures cover only the parts that measured."
            ),
            Self::IndexFailed { detail } => {
                write!(f, "Couldn't index the mesh for processing: {detail}")
            }
            Self::OperationFailed { operation, detail } => {
                write!(f, "{}: {detail}", operation.label())
            }
            Self::BakeBeforeSimplify => f.write_str(
                "Bake AO runs before a simplifier, so the baked occlusion describes the \
                 pre-simplified geometry. Move the bake below the simplifier — or below \
                 Generate LODs to bake every level.",
            ),
            Self::QuadsLostToSimplify => f.write_str(
                "A simplifier runs after Remesh and rebuilds its faces as triangles, so the \
                 quads it produced are lost. Move Remesh below the simplifier to keep them.",
            ),
            Self::ShrinkwrapBelowShapeChange => f.write_str(
                "Shrinkwrap runs below an operation that changes the shape, and it replaces \
                 the whole object — so that operation's work is thrown away. Move Shrinkwrap \
                 to the top of the list.",
            ),
            Self::LodEmpty { level } => write!(
                f,
                "LOD {level} simplified away completely. Raise its target ratio, or lower its \
                 error limit so the simplifier stops sooner."
            ),
            Self::SimplifierStalledOnSeams => f.write_str(
                "The simplifier removed almost nothing: this mesh has an attribute seam at \
                 nearly every edge, which a topology-preserving collapse cannot cross. Add a \
                 Weld operation with 'Compare normals' off, or turn on 'Collapse across seams' \
                 in the simplifier options.",
            ),
            Self::TangentsFailed { detail } => {
                write!(f, "Couldn't rebuild the tangents: {detail}")
            }
            Self::NormalsKeptForBlendShapes { object } => write!(
                f,
                "Recalculate Normals: '{object}' has blend shapes, so its normals were left as \
                 they are. Its shapes store normal changes relative to the old normals, which \
                 new ones would contradict."
            ),
            Self::NormalsFailed { object, detail } => {
                write!(f, "Recalculate Normals: '{object}': {detail}")
            }
            Self::ShrinkwrapKeptDeforming { object } => write!(
                f,
                "Shrinkwrap: '{object}' is skinned or has blend shapes, so it was left as it \
                 is. Wrapping replaces the surface entirely, and no skin weight authored \
                 against the old vertices can follow it."
            ),
            Self::ShrinkwrapTooFewTriangles { object, triangles } => write!(
                f,
                "Shrinkwrap: '{object}' has too few triangles to wrap ({triangles}), so it was \
                 left as it is."
            ),
            Self::ShrinkwrapEmptyField { object, resolution } => write!(
                f,
                "Shrinkwrap: '{object}' came back empty — at resolution {resolution} the \
                 object is thinner than one voxel. Raise the resolution, or add a small offset \
                 to give it some thickness."
            ),
            Self::ShrinkwrapEmptyVoxels { object, resolution } => write!(
                f,
                "Shrinkwrap: '{object}' came back empty at voxel resolution {resolution}. \
                 Raise the resolution, or switch to the distance-field method."
            ),
            Self::ShrinkwrapVoxelFailed { object, detail } => write!(
                f,
                "Shrinkwrap: '{object}' could not be voxel remeshed ({detail}), so it was left \
                 as it is."
            ),
            Self::ShrinkwrapResolutionLowered {
                object,
                requested,
                resolution,
            } => write!(
                f,
                "Shrinkwrap: '{object}' would need more memory than a wrap is worth at \
                 resolution {requested}, so it ran at {resolution} instead."
            ),
            Self::RemeshKeptDeforming { object } => write!(
                f,
                "Remesh: '{object}' is skinned or has blend shapes, so it was left as it is. \
                 Remeshing rebuilds the surface from scratch, and no skin weight authored \
                 against the old vertices can follow it."
            ),
            Self::RemeshTooFewTriangles { object, triangles } => write!(
                f,
                "Remesh: '{object}' has too few triangles to rebuild ({triangles}), so it was \
                 left as it is."
            ),
            Self::RemeshFailed { object, detail } => {
                write!(f, "Remesh: '{object}' was left as it is — {detail}")
            }
            Self::RemeshBudgetAboveInput {
                object,
                triangles,
                asked,
            } => write!(
                f,
                "Remesh: '{object}' already has fewer faces ({triangles}) than the {asked} \
                 asked for, and a rebuild only ever merges - it cannot add detail that is not \
                 there. It came back at about its current density."
            ),
            Self::RemeshEmpty { object } => write!(
                f,
                "Remesh: '{object}' came back empty. Ask for more faces, or check that the \
                 object is not a handful of disconnected slivers."
            ),
            Self::RemeshStubborn { object, parts } => write!(
                f,
                "Remesh: {parts} parts of '{object}' could not be simplified as far as asked \
                 without breaking the surface, so it came back denser there."
            ),
            Self::RemeshFewQuads {
                object,
                quads,
                faces,
            } => write!(
                f,
                "Remesh: only {quads} of '{object}' came back as quads ({faces} faces) - the \
                 rebuilt surface folds too sharply at this density for the rest to pair up. \
                 Ask for more faces, or rebuild it as triangles."
            ),
        }
    }
}

/// Something an export lost, which the user should hear about now rather than
/// when the file reaches an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportNote {
    /// The viewer's source-property capture had not landed when the export ran.
    CaptureNotLoaded,
    /// A flat hierarchy has no bones for skins and blend shapes to bind to.
    DeformNotWrittenFlat,
    /// A flat hierarchy has no nodes for animation curves to drive.
    AnimationNotWrittenFlat,
    /// A whole mesh was written as triangles because the stack rebuilt it.
    WrittenAsTriangles { level: usize, mesh: String },
    /// Some of a mesh's triangles were written as triangles.
    TrianglesRebuilt {
        level: usize,
        mesh: String,
        count: usize,
    },
    /// Animation layers that belonged to no stack.
    OrphanedAnimationLayers { count: usize },
    /// Animated properties whose target has no counterpart in this export.
    UnmappedAnimatedProperties { count: usize },
    /// Selection-set entries that lost members the stack rebuilt or removed.
    SelectionMembersLost { count: usize },
    /// Poses that were not bind poses.
    NonBindPosesSkipped { count: usize },
    /// Skin clusters bound to nodes that could not be written.
    UnplacedClusters { mesh: String, count: usize },
    /// Layered textures nested inside another layered texture.
    NestedLayeredTextures { count: usize },
    /// A node's parent chain looped; that branch was exported flat.
    ParentChainLooped,
    /// A node's transform could not be inverted.
    NonInvertibleTransform { node: String },
}

impl fmt::Display for ExportNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CaptureNotLoaded => f.write_str(
                "The source's properties had not finished loading: node transforms, materials \
                 and scene settings were written from what the viewer shows, without textures \
                 or user properties.",
            ),
            Self::DeformNotWrittenFlat => f.write_str(
                "Skinning and blend shapes were not written: a flat hierarchy bakes the \
                 geometry into world space, which has no bones to bind to.",
            ),
            Self::AnimationNotWrittenFlat => f.write_str(
                "Animation was not written: a flat hierarchy has no nodes for the curves to \
                 drive.",
            ),
            Self::WrittenAsTriangles { level, mesh } => write!(
                f,
                "LOD {level}: '{mesh}' was written as triangles — the stack rebuilt its \
                 geometry."
            ),
            Self::TrianglesRebuilt { level, mesh, count } => write!(
                f,
                "LOD {level}: {count} triangle(s) of '{mesh}' were written as triangles — the \
                 stack rebuilt them."
            ),
            Self::OrphanedAnimationLayers { count } => write!(
                f,
                "{count} animation layer(s) belonged to no stack and were not written."
            ),
            Self::UnmappedAnimatedProperties { count } => write!(
                f,
                "{count} animated propert{} target elements this export has no counterpart \
                 for and were not written.",
                if *count == 1 { "y" } else { "ies" }
            ),
            Self::SelectionMembersLost { count } => write!(
                f,
                "{count} selection set entr{} lost some vertex / edge / face members: the \
                 stack rebuilt or removed them.",
                if *count == 1 { "y" } else { "ies" }
            ),
            Self::NonBindPosesSkipped { count } => write!(
                f,
                "{count} non-bind pose(s) were not written (only bind poses are)."
            ),
            Self::UnplacedClusters { mesh, count } => write!(
                f,
                "{count} skin cluster(s) of '{mesh}' bind to nodes that could not be written."
            ),
            Self::NestedLayeredTextures { count } => write!(
                f,
                "{count} layered texture(s) nested inside another layered texture were not \
                 written."
            ),
            Self::ParentChainLooped => {
                f.write_str("A node's parent chain looped; that branch was exported flat.")
            }
            Self::NonInvertibleTransform { node } => write!(
                f,
                "'{node}' has a non-invertible transform; its children were exported relative \
                 to it without it."
            ),
        }
    }
}
