### The Opt workspace's chrome: the operation stack pane and the Inspector's
### retargeted parameter view.
###
### An operation's *name* and its one-line explanation live in `ui-enums.ftl`
### beside the other `optimize` values, because the stack pane, the Inspector and
### the export report all name the same operations.

## The stack pane

ui-opt-add-operation-button = + Add operation
ui-opt-empty = No operations yet.
    .description = Add one above to start optimizing.
ui-opt-enable-operation = Include this operation when processing
    .description = Switching an operation off leaves it in the stack, so you can compare
        with and without it rather than rebuilding it.
ui-opt-move-up = Move up
    .description = Operations apply top to bottom, so this one runs earlier.
ui-opt-move-down = Move down
    .description = Operations apply top to bottom, so this one runs later.
ui-opt-remove-operation = Remove this operation
ui-opt-one-lod-only = A stack can hold only one LOD operation
ui-opt-save-preset = Save preset
    .description = Write this operation stack, its per-object overrides and the export
        settings to a JSON file.
ui-opt-load-preset = Load preset
    .description = Replace this stack with one loaded from a JSON file.

## The Inspector, retargeted

ui-opt-no-settings = This operation has no settings.
ui-opt-operation-gone = This operation no longer exists.
ui-opt-object-overrides = How this object deviates from the global stack.
ui-opt-exclude = Exclude from optimization
    .description = The object passes through untouched at full detail in every output
        level. It still blocks light during an ambient-occlusion bake; it is simply not
        written to.

## Simplifier options

ui-opt-simplifier-options = Simplifier options
ui-opt-algorithm = Algorithm
    .description = Standard keeps topology and measures position error only. Preserve
        Attributes also penalizes normal, UV and colour drift. Sloppy ignores topology:
        much faster and far more likely to hit the target, but it can close holes.
ui-opt-triangles = Triangles
    .description = The share of triangles to keep.
ui-opt-error-limit = Error limit
    .description = The deviation the simplifier may not exceed. It stops short of the
        triangle target rather than going past this.
ui-opt-lock-border = Lock border
    .description = Never move vertices on an open boundary.
ui-opt-absolute-error = Absolute error
    .description = Read the error limit in world units, not as a fraction of the mesh size.
ui-opt-prune-while-simplifying = Prune while simplifying
    .description = Let the simplifier delete disconnected parts as it goes.
ui-opt-regularize = Regularize
    .description = Even out triangle size and shape, at some cost to accuracy.
ui-opt-regularize-light = Regularize (light)
    .description = A gentler regularization.
ui-opt-collapse-across-seams = Collapse across seams
    .description = Allow collapses across UV and normal discontinuities. This is what
        unblocks a simplify that has stalled on attribute seams.
ui-opt-attribute-weights = Attribute weights
    .description = How hard each attribute resists being distorted. Higher preserves it
        harder at the cost of geometric accuracy; zero ignores it.
ui-opt-normals = Normals
ui-opt-uvs = UVs
ui-opt-colors = Colors

## LOD chain

ui-opt-levels = Levels
ui-opt-add-level = Add a level
ui-opt-lod-level = LOD { $level }

## Weld and prune

ui-opt-tolerance = Tolerance
    .description = How far apart two vertices may be and still merge. Zero merges only
        exact matches.
ui-opt-compare-normals = Compare normals
ui-opt-compare-uvs = Compare UVs
ui-opt-compare-colors = Compare colors
ui-opt-size-threshold = Size threshold
    .description = Disconnected pieces smaller than this are removed: stray shells and
        orphaned faces.
ui-opt-cache-tolerance = Cache tolerance
    .description = How much vertex-cache efficiency the overdraw reorder may trade away.

## AO bake

ui-opt-ao-quality = Quality
    .description = Rays cast per vertex. More is slower and smoother.
ui-opt-ao-max-distance = Max distance
    .description = How far a surface can be and still block light, in world metres. Zero
        means the whole scene.
ui-opt-ao-intensity = Intensity
    .description = A power curve on visibility, matching the viewport's own panel.
ui-opt-ao-write-to = Write to
    .description = Which part of the RGBA vertex colour the baked value goes in. Alpha is
        always linear; the RGB targets can be sRGB-encoded.

## Export

ui-opt-export-settings = Export settings
    .description = Where and how the processed mesh is written.
ui-opt-export-button = Export...
ui-opt-export-packaging = Packaging
ui-opt-export-hierarchy = Hierarchy
ui-opt-export-format = Format
ui-opt-packaging-single-explained = One file, with each mesh repeated as MeshName_LOD0 ... _LOD{ $last }
ui-opt-packaging-single-one-level = One file, with each mesh under its source name
ui-opt-hierarchy-rebuild-explained = The source node hierarchy is rebuilt, geometry moved back into each node's local space.
ui-opt-hierarchy-flat-explained = Each mesh becomes a root-level node with world-space geometry.
ui-opt-export-nothing = Nothing processed yet
    .description = Add an operation first: there is no processed mesh to write.
ui-opt-export-waiting = Waiting for the current run to finish

## Fallbacks

ui-opt-unnamed-node = Node { $index }
ui-opt-packaging-file-per-lod-explained = { $count } files, one per level (..._LOD0.fbx, ...)

## The overlay comparison's legend

ui-opt-legend-solid = { $side } - shaded
ui-opt-legend-ghost = { $side } - { $style }
ui-opt-ao-srgb = sRGB encode
    .description = Encode the baked value for the RGB targets. Alpha is always written
        linear, because that is how an engine reads a mask.

## Longer explanations, shown above a group of parameters

ui-opt-weld-explained = Positions must always match exactly; the tolerance applies to the
    compared attributes. Unchecking one merges across that kind of seam - unchecking
    normals, for instance, welds a flat-shaded mesh's hard edges and flattens their
    shading.
ui-opt-prune-explained = A fraction of the mesh's overall size. Disconnected pieces smaller
    than this are removed.
ui-opt-overdraw-explained = How much vertex-cache efficiency may be given up to reduce
    overdraw. 1.05 allows a 5% regression; 1.0 forbids any.
ui-opt-bake-ao-explained = Objects named with a _LOD suffix bake only against their own
    LOD's geometry (plus objects with no suffix), so a whole visible chain bakes in one
    run. Hidden objects neither occlude nor bake - hide collision shells in the Outliner
    first. Max distance is how far a surface can be and still occlude, in world metres; 0
    is unlimited. Intensity is a power on the visibility - above 1 darkens. The alpha
    channel is always written linear. Preview it through the Vertex Colors material. If a
    Preserve Attributes simplify runs after this bake, raise its Colors weight above zero
    or it will ignore the AO.
ui-opt-attribute-weights-explained = Higher weights protect that attribute at the cost of
    geometric accuracy. Zero ignores it entirely.
ui-opt-reduce-explained = Triangles is a fraction of this object's count at this point in
    the stack; the simplifier stops short of it rather than exceed the error limit. Unlike
    Generate LODs this rewrites the mesh itself - every operation below it works on the
    reduced geometry, and an export with no LOD chain writes it in the source mesh's place.
ui-opt-lod-explained = Triangles is a fraction of this object's count at this point in the
    stack. Every level is simplified from that same mesh, not from the level above, so one
    level's error never compounds into the next. The simplifier stops short of the target
    rather than exceed the error limit.
ui-opt-export-explained = Where and how the processed mesh - and its LOD chain, if the
    stack generates one - is written. Everything the stack did not change is written as
    authored: polygons, materials and textures, skins, blend shapes, animation curves and
    properties. Material edits made in the viewer stay previews.

ui-opt-remove-level = Remove this level
ui-opt-select-something = Select an operation in the stack to edit its settings, or a node
    in the Outliner to give it its own.
