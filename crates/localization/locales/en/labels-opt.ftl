### The optimizer's typed values, as every surface that prints them names them:
### the viewer's Opt workspace and `review-cli optimize` alike (`crates/labels`).
###
### The optimizer hands these over as values (`OpKind`, `OptWarning`, `ExportNote`,
### `OptError`). A `{ $detail }` is the library's own diagnostic, kept in English as
### every error is (invariant 12); an `{ $operation }` is the operation's name as
### the stack shows it.

## Opt operations

# Each operation's name and what it does. Shown on the stack row, in the
# "Add operation" menu, and as the heading over its parameters - one message, so
# the three cannot describe it differently.
labels-opt-op-weld = Weld Vertices
    .description = Joins points that sit in the same place. Every run already joins points
        that match in every way; this operation widens what counts as a match - for
        example ignoring normals or texture layout, or allowing a small gap - which does
        change the model.
labels-opt-op-filter-triangles = Filter Triangles
    .description = Removes broken triangles (ones squashed into a line) and exact copies of
        other triangles. Copies that face the other way are kept, since they are how
        double-sided surfaces are made.
labels-opt-op-prune-components = Prune Components
    .description = Removes small loose pieces below a size you choose - stray shells and
        single faces left floating around by mistake.
labels-opt-op-reduce = Reduce
    .description = Simplifies the model in place, using the same simplifier that makes LOD
        levels. Instead of making extra copies, it replaces the model, so everything
        below it in the list works on the simplified version, and the export saves it in
        place of the original.
labels-opt-op-remesh = Remesh
    .description = Rebuilds each object's surface out of evenly sized faces, triangles or
        quads, sized to follow the shape's own curves. Unlike Reduce it does not take away what is there -
        it lays down a new surface and copies the materials, texture layout and colors
        across, which is what turns a scan or a CAD part into something a game can use.
        Open borders and sharp edges are kept where they were. Works on still objects
        only.
labels-opt-op-shrinkwrap = Shrinkwrap
    .description = Replaces each object with a single closed skin that hugs it. The shape is
        measured onto a grid and a new surface pulled back out of it, which fuses a pile of
        overlapping parts into one solid and seals holes; the Voxel method also keeps thin
        sheets such as leaves. What comes out is dense and
        irregular, so a Remesh below it is what makes it even. Materials, texture layout
        and colors are copied across. Works on still objects only.
labels-opt-op-recalculate-normals = Recalculate Normals
    .description = Works out fresh shading directions from the shape itself. Edges sharper
        than the crease angle stay hard and the rest are smoothed, and the hard and soft
        edge markings saved with the model are rewritten to match. Objects with blend
        shapes are left as they are. Does not change the shape.
labels-opt-op-simplify-lod = Generate LODs
    .description = Makes a set of simpler copies of the model (levels of detail) for a game
        to show at a distance. Each level is made from the model as it stands at this
        point in the list.
labels-opt-op-bake-ao = Bake AO to Vertex Colors
    .description = Works out how much light reaches each point of the model and stores it
        as a color on that point. Objects whose names end in _LOD and a number are shaded
        only by their own level, so a whole set of levels can bake in one go. Hidden
        objects are left out, so hide collision shapes first. Does not change the shape.
labels-opt-op-vertex-cache = Optimize Vertex Cache
    .description = Reorders the triangles so the GPU can reuse work it has just
        done. Does not change the shape; watch the ACMR and ATVR numbers on the stats
        card to see the effect.
labels-opt-op-overdraw = Optimize Overdraw
    .description = Reorders the triangles so nearer ones tend to be drawn first, and the
        GPU wastes less effort on parts that end up hidden. Does not change the
        shape.
labels-opt-op-vertex-fetch = Optimize Vertex Fetch
    .description = Reorders the points into the order they are read, and removes any point
        nothing uses. Does not change the shape.

## Warnings from a run

labels-opt-warning-deform-dropped =
    The processed mesh's skin and blend-shape data did not match up ({ $detail }), so it
    was left out of this level and its export.
labels-opt-warning-measure-failed =
    Couldn't measure part of the mesh: { $detail }. The cache, overdraw and fetch figures
    cover only the parts that measured.
labels-opt-warning-index-failed = Couldn't index the mesh for processing: { $detail }
labels-opt-warning-operation-failed = { $operation }: { $detail }
labels-opt-warning-bake-before-simplify =
    Bake AO runs before a simplifier, so the baked occlusion describes the pre-simplified
    geometry. Move the bake below the simplifier - or below Generate LODs to bake every
    level.
labels-opt-warning-quads-lost =
    A simplifier runs after Remesh and rebuilds its faces as triangles, so the quads it
    produced are lost. Move Remesh below the simplifier to keep them.
labels-opt-warning-shrinkwrap-below-shape-change =
    Shrinkwrap runs below an operation that changes the shape, and it replaces the whole
    object - so that operation's work is thrown away. Move Shrinkwrap to the top of the
    list.
labels-opt-warning-lod-empty =
    LOD { $level } simplified away completely. Raise its target ratio, or lower its error
    limit so the simplifier stops sooner.
labels-opt-warning-authored-lods-replaced = { $objects ->
        [one] This model already has LOD levels, so Generate LODs built its levels from each chain's _LOD0 and left out the one existing lower-detail object, which the generated levels replace.
       *[other] This model already has LOD levels, so Generate LODs built its levels from each chain's _LOD0 and left out the { $objects } existing lower-detail objects, which the generated levels replace.
    }
labels-opt-warning-simplifier-stalled =
    The simplifier removed almost nothing: this mesh has an attribute seam at nearly every
    edge, which a topology-preserving collapse cannot cross. Add a Weld operation with
    'Compare normals' off, or turn on 'Collapse across seams' in the simplifier options.
labels-opt-warning-tangents-failed = Couldn't rebuild the tangents: { $detail }
labels-opt-warning-normals-kept-for-blend-shapes =
    Recalculate Normals: '{ $object }' has blend shapes, so its normals were left as they
    are. Its shapes store normal changes relative to the old normals, which new ones would
    contradict.
labels-opt-warning-normals-failed = Recalculate Normals: '{ $object }': { $detail }
labels-opt-warning-shrinkwrap-kept-deforming =
    Shrinkwrap: '{ $object }' is skinned or has blend shapes, so it was left as it is.
    Wrapping replaces the surface entirely, and no skin weight authored against the old
    vertices can follow it.
labels-opt-warning-shrinkwrap-too-few-triangles =
    Shrinkwrap: '{ $object }' has too few triangles to wrap ({ $triangles }), so it was left
    as it is.
labels-opt-warning-shrinkwrap-empty-field =
    Shrinkwrap: '{ $object }' came back empty - at resolution { $resolution } the object is
    thinner than one voxel. Raise the resolution, or add a small offset to give it some
    thickness.
labels-opt-warning-shrinkwrap-empty-voxels =
    Shrinkwrap: '{ $object }' came back empty at voxel resolution { $resolution }. Raise the
    resolution, or switch to the distance-field method.
labels-opt-warning-shrinkwrap-voxel-failed =
    Shrinkwrap: '{ $object }' could not be voxel remeshed ({ $detail }), so it was left as it
    is.
labels-opt-warning-shrinkwrap-failed = Shrinkwrap: '{ $object }' was left as it is - { $detail }
labels-opt-warning-shrinkwrap-resolution-lowered =
    Shrinkwrap: '{ $object }' would need more memory than a wrap is worth at resolution
    { $requested }, so it ran at { $resolution } instead.
labels-opt-warning-remesh-kept-deforming =
    Remesh: '{ $object }' is skinned or has blend shapes, so it was left as it is.
    Remeshing rebuilds the surface from scratch, and no skin weight authored against the
    old vertices can follow it.
labels-opt-warning-remesh-too-few-triangles =
    Remesh: '{ $object }' has too few triangles to rebuild ({ $triangles }), so it was left
    as it is.
labels-opt-warning-remesh-failed = Remesh: '{ $object }' was left as it is - { $detail }
labels-opt-warning-remesh-budget-above-input =
    Remesh: '{ $object }' already has fewer faces ({ $triangles }) than the { $asked } asked
    for, and a rebuild only ever merges - it cannot add detail that is not there. It came
    back at about its current density.
labels-opt-warning-remesh-empty =
    Remesh: '{ $object }' came back empty. Ask for more faces, or check that the object is
    not a handful of disconnected slivers.
labels-opt-warning-remesh-stubborn = { $parts ->
        [one] Remesh: one part of '{ $object }' could not be simplified as far as asked without breaking the surface, so it came back denser there.
       *[other] Remesh: { $parts } parts of '{ $object }' could not be simplified as far as asked without breaking the surface, so it came back denser there.
    }
labels-opt-warning-remesh-few-quads =
    Remesh: only { $quads } of '{ $object }' came back as quads ({ $faces } faces) - the
    rebuilt surface folds too sharply at this density for the rest to pair up. Ask for more
    faces, or rebuild it as triangles.

## Notes on an export

labels-opt-note-capture-not-loaded =
    The source's properties had not finished loading: node transforms, materials and scene
    settings were written from what the viewer shows, without textures or user properties.
labels-opt-note-deform-not-written-flat =
    Skinning and blend shapes were not written: a flat hierarchy bakes the geometry into
    world space, which has no bones to bind to.
labels-opt-note-animation-not-written-flat =
    Animation was not written: a flat hierarchy has no nodes for the curves to drive.
labels-opt-note-orphaned-animation-layers = { $count ->
        [one] One animation layer belonged to no stack and was not written.
       *[other] { $count } animation layers belonged to no stack and were not written.
    }
labels-opt-note-unmapped-animated-properties = { $count ->
        [one] One animated property targets an element this export has no counterpart for and was not written.
       *[other] { $count } animated properties target elements this export has no counterpart for and were not written.
    }
labels-opt-note-selection-members-lost = { $count ->
        [one] One selection set entry lost some vertex / edge / face members: the stack rebuilt or removed them.
       *[other] { $count } selection set entries lost some vertex / edge / face members: the stack rebuilt or removed them.
    }
labels-opt-note-non-bind-poses-skipped = { $count ->
        [one] One pose that is not a bind pose was not written (only bind poses are).
       *[other] { $count } poses that are not bind poses were not written (only bind poses are).
    }
labels-opt-note-unplaced-clusters = { $count ->
        [one] One skin cluster of '{ $mesh }' binds to a node that could not be written.
       *[other] { $count } skin clusters of '{ $mesh }' bind to nodes that could not be written.
    }
labels-opt-note-nested-layered-textures = { $count ->
        [one] One layered texture whose layers could not be placed was not written.
       *[other] { $count } layered textures whose layers could not be placed were not written.
    }
labels-opt-note-parent-chain-looped = A node's parent chain looped; that branch was exported flat.
labels-opt-note-non-invertible-transform =
    '{ $node }' has a non-invertible transform; its children were exported relative to it
    without it.

## Errors a user can act on

# Everything else keeps its library diagnostic verbatim; these tell the reader
# what to *do*, which a diagnostic written for a bug report does not.
labels-opt-opt-unavailable = Mesh optimization is not available in this build.
    It was built without the optimizer library, so the Opt workspace can measure a
    model but cannot change one.
labels-opt-preset-newer = This preset was saved by a newer version of the viewer.
    It is version { $found }, and this build understands up to version { $supported }.
    Open it with the version that saved it, or set the operations up again here.

## A preset loaded against another model

# Per-object overrides that could not be reattached when a preset was loaded.
labels-opt-overrides-dropped = { $count ->
        [one] One per-object setting in the preset referred to an object this model doesn't have, so it was left out.
       *[other] { $count } per-object settings in the preset referred to objects this model doesn't have, so they were left out.
    }
labels-opt-overrides-by-position = { $count ->
        [one] One per-object setting came from an older preset that did not store object names, so it was matched by its place in the list - please check it landed on the object you meant.
       *[other] { $count } per-object settings came from an older preset that did not store object names, so they were matched by their place in the list - please check they landed on the objects you meant.
    }

# What loading a preset had to remove because it broke the rules the operation
# stack keeps - only a preset edited by hand can.
labels-opt-preset-lods-removed = { $count ->
        [one] The preset had a second Generate LODs operation. A stack runs only one, so it was left out.
       *[other] The preset had { $count } extra Generate LODs operations. A stack runs only one, so they were left out.
    }
labels-opt-preset-overrides-repaired = { $count ->
        [one] One per-object setting in the preset did not match the operation it named, or repeated another one, so it was left out.
       *[other] { $count } per-object settings in the preset did not match the operations they named, or repeated others, so they were left out.
    }
