### What an optimization run or an export has to say about its result.
###
### The optimizer hands these over as values (`OptWarning`, `ExportNote`) and
### `app/src/explain.rs` picks the message for each. A `{ $detail }` is the
### library's own diagnostic, kept in English as every error is (invariant 12);
### an `{ $operation }` is the operation's name as the stack shows it.

## Warnings from a run

app-opt-warning-deform-dropped =
    The processed mesh's skin and blend-shape data did not match up ({ $detail }), so it
    was left out of this level and its export.
app-opt-warning-measure-failed =
    Couldn't measure part of the mesh: { $detail }. The cache, overdraw and fetch figures
    cover only the parts that measured.
app-opt-warning-index-failed = Couldn't index the mesh for processing: { $detail }
app-opt-warning-operation-failed = { $operation }: { $detail }
app-opt-warning-bake-before-simplify =
    Bake AO runs before a simplifier, so the baked occlusion describes the pre-simplified
    geometry. Move the bake below the simplifier - or below Generate LODs to bake every
    level.
app-opt-warning-quads-lost =
    A simplifier runs after Remesh and rebuilds its faces as triangles, so the quads it
    produced are lost. Move Remesh below the simplifier to keep them.
app-opt-warning-shrinkwrap-below-shape-change =
    Shrinkwrap runs below an operation that changes the shape, and it replaces the whole
    object - so that operation's work is thrown away. Move Shrinkwrap to the top of the
    list.
app-opt-warning-lod-empty =
    LOD { $level } simplified away completely. Raise its target ratio, or lower its error
    limit so the simplifier stops sooner.
app-opt-warning-simplifier-stalled =
    The simplifier removed almost nothing: this mesh has an attribute seam at nearly every
    edge, which a topology-preserving collapse cannot cross. Add a Weld operation with
    'Compare normals' off, or turn on 'Collapse across seams' in the simplifier options.
app-opt-warning-tangents-failed = Couldn't rebuild the tangents: { $detail }
app-opt-warning-normals-kept-for-blend-shapes =
    Recalculate Normals: '{ $object }' has blend shapes, so its normals were left as they
    are. Its shapes store normal changes relative to the old normals, which new ones would
    contradict.
app-opt-warning-normals-failed = Recalculate Normals: '{ $object }': { $detail }
app-opt-warning-shrinkwrap-kept-deforming =
    Shrinkwrap: '{ $object }' is skinned or has blend shapes, so it was left as it is.
    Wrapping replaces the surface entirely, and no skin weight authored against the old
    vertices can follow it.
app-opt-warning-shrinkwrap-too-few-triangles =
    Shrinkwrap: '{ $object }' has too few triangles to wrap ({ $triangles }), so it was left
    as it is.
app-opt-warning-shrinkwrap-empty-field =
    Shrinkwrap: '{ $object }' came back empty - at resolution { $resolution } the object is
    thinner than one voxel. Raise the resolution, or add a small offset to give it some
    thickness.
app-opt-warning-shrinkwrap-empty-voxels =
    Shrinkwrap: '{ $object }' came back empty at voxel resolution { $resolution }. Raise the
    resolution, or switch to the distance-field method.
app-opt-warning-shrinkwrap-voxel-failed =
    Shrinkwrap: '{ $object }' could not be voxel remeshed ({ $detail }), so it was left as it
    is.
app-opt-warning-shrinkwrap-resolution-lowered =
    Shrinkwrap: '{ $object }' would need more memory than a wrap is worth at resolution
    { $requested }, so it ran at { $resolution } instead.
app-opt-warning-remesh-kept-deforming =
    Remesh: '{ $object }' is skinned or has blend shapes, so it was left as it is.
    Remeshing rebuilds the surface from scratch, and no skin weight authored against the
    old vertices can follow it.
app-opt-warning-remesh-too-few-triangles =
    Remesh: '{ $object }' has too few triangles to rebuild ({ $triangles }), so it was left
    as it is.
app-opt-warning-remesh-failed = Remesh: '{ $object }' was left as it is - { $detail }
app-opt-warning-remesh-budget-above-input =
    Remesh: '{ $object }' already has fewer faces ({ $triangles }) than the { $asked } asked
    for, and a rebuild only ever merges - it cannot add detail that is not there. It came
    back at about its current density.
app-opt-warning-remesh-empty =
    Remesh: '{ $object }' came back empty. Ask for more faces, or check that the object is
    not a handful of disconnected slivers.
app-opt-warning-remesh-stubborn = { $parts ->
        [one] Remesh: one part of '{ $object }' could not be simplified as far as asked without breaking the surface, so it came back denser there.
       *[other] Remesh: { $parts } parts of '{ $object }' could not be simplified as far as asked without breaking the surface, so it came back denser there.
    }
app-opt-warning-remesh-few-quads =
    Remesh: only { $quads } of '{ $object }' came back as quads ({ $faces } faces) - the
    rebuilt surface folds too sharply at this density for the rest to pair up. Ask for more
    faces, or rebuild it as triangles.

## Notes on an export

app-opt-note-capture-not-loaded =
    The source's properties had not finished loading: node transforms, materials and scene
    settings were written from what the viewer shows, without textures or user properties.
app-opt-note-deform-not-written-flat =
    Skinning and blend shapes were not written: a flat hierarchy bakes the geometry into
    world space, which has no bones to bind to.
app-opt-note-animation-not-written-flat =
    Animation was not written: a flat hierarchy has no nodes for the curves to drive.
app-opt-note-written-as-triangles =
    LOD { $level }: '{ $mesh }' was written as triangles - the stack rebuilt its geometry.
app-opt-note-triangles-rebuilt = { $count ->
        [one] LOD { $level }: one triangle of '{ $mesh }' was written as a triangle - the stack rebuilt it.
       *[other] LOD { $level }: { $count } triangles of '{ $mesh }' were written as triangles - the stack rebuilt them.
    }
app-opt-note-orphaned-animation-layers = { $count ->
        [one] One animation layer belonged to no stack and was not written.
       *[other] { $count } animation layers belonged to no stack and were not written.
    }
app-opt-note-unmapped-animated-properties = { $count ->
        [one] One animated property targets an element this export has no counterpart for and was not written.
       *[other] { $count } animated properties target elements this export has no counterpart for and were not written.
    }
app-opt-note-selection-members-lost = { $count ->
        [one] One selection set entry lost some vertex / edge / face members: the stack rebuilt or removed them.
       *[other] { $count } selection set entries lost some vertex / edge / face members: the stack rebuilt or removed them.
    }
app-opt-note-non-bind-poses-skipped = { $count ->
        [one] One pose that is not a bind pose was not written (only bind poses are).
       *[other] { $count } poses that are not bind poses were not written (only bind poses are).
    }
app-opt-note-unplaced-clusters = { $count ->
        [one] One skin cluster of '{ $mesh }' binds to a node that could not be written.
       *[other] { $count } skin clusters of '{ $mesh }' bind to nodes that could not be written.
    }
app-opt-note-nested-layered-textures = { $count ->
        [one] One layered texture nested inside another layered texture was not written.
       *[other] { $count } layered textures nested inside another layered texture were not written.
    }
app-opt-note-parent-chain-looped = A node's parent chain looped; that branch was exported flat.
app-opt-note-non-invertible-transform =
    '{ $node }' has a non-invertible transform; its children were exported relative to it
    without it.
