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
    .description = Turning an operation off keeps it in the list, so you can compare the
        result with and without it instead of setting it up again.
ui-opt-move-up = Move up
    .description = Operations run from top to bottom, so this one will run earlier.
ui-opt-move-down = Move down
    .description = Operations run from top to bottom, so this one will run later.
ui-opt-remove-operation = Remove this operation
ui-opt-one-lod-only = A stack can hold only one LOD operation
ui-opt-save-preset = Save preset
    .description = Saves this list of operations, along with any per-object settings and
        the export settings, to a small file you can share or reuse.
ui-opt-load-preset = Load preset
    .description = Replaces this list with one loaded from a saved preset file.

## The Inspector, retargeted

ui-opt-no-settings = This operation has no settings.
ui-opt-operation-gone = This operation no longer exists.
ui-opt-object-overrides = How this object is treated differently from the rest.
ui-opt-exclude = Exclude from optimization
    .description = Leaves this object exactly as it is, at full detail, in every output
        level. It still casts shadows during an ambient-occlusion bake; it just does not
        get changed itself.

## Simplifier options

ui-opt-simplifier-options = Simplifier options
ui-opt-algorithm = Algorithm
    .description = Standard keeps the model's shape connected and only watches how far the
        surface moves. Preserve Attributes also tries to keep the shading, texture
        layout and colors from drifting. Sloppy does not care about keeping the shape
        connected: it is much faster and nearly always reaches the target, but it can
        close up holes.
ui-opt-triangles = Triangles
    .description = What share of the triangles to keep.
ui-opt-error-limit = Error limit
    .description = How far the surface is allowed to move. If reaching the triangle target
        would mean going past this, the simplifier stops early instead.
ui-opt-lock-border = Lock border
    .description = Never moves points that sit on an open edge of the model.
ui-opt-absolute-error = Absolute error
    .description = Reads the error limit as a real distance in the scene, instead of as a
        fraction of the model's size.
ui-opt-prune-while-simplifying = Prune while simplifying
    .description = Lets the simplifier throw away small loose pieces as it works.
ui-opt-regularize = Regularize
    .description = Tries to keep the triangles evenly sized and nicely shaped, even if that
        means the surface drifts a little more.
ui-opt-regularize-light = Regularize (light)
    .description = A gentler version of the same idea.
ui-opt-collapse-across-seams = Collapse across seams
    .description = Lets the simplifier work across texture seams and hard edges. If a
        simplify seems stuck and barely removes anything, this is usually what frees it.
ui-opt-preserve-folds = Preserve folds
    .description = Keeps the edge where two surfaces meet back to back, such as the rim of a
        double-sided leaf or cloth, from wearing away as the model gets simpler. Slightly
        slower.
ui-opt-clamp-attribute-error = Clamp attribute error
    .description = Stops the shading, texture layout and colors from counting for more than
        the shape itself. A busy texture area then no longer holds the whole model back, and
        the error figure stays close to how far the surface really moved.
ui-opt-attribute-weights = Attribute weights
    .description = How hard each kind of data pushes back against being changed. A higher
        number protects it more, at the cost of a less accurate shape. Zero ignores it.
ui-opt-normals = Normals
ui-opt-uvs = UVs
ui-opt-colors = Colors

## LOD chain

ui-opt-levels = Levels
ui-opt-add-level = Add a level
ui-opt-lod-level = LOD { $level }

## Weld and prune

ui-opt-tolerance = Tolerance
    .description = How close two points need to be before they are joined into one. Zero
        joins only points that match exactly.
ui-opt-compare-normals = Compare normals
ui-opt-compare-uvs = Compare UVs
ui-opt-compare-colors = Compare colors
ui-opt-size-threshold = Size threshold
    .description = Loose pieces smaller than this are removed: stray shells, and single
        faces left floating on their own.
ui-opt-cache-tolerance = Cache tolerance
    .description = How much of the vertex-cache benefit the overdraw reorder is allowed to
        give up.

## AO bake

ui-opt-ao-quality = Quality
    .description = How many rays are cast from each point. More rays take longer but give a
        smoother result.
ui-opt-ao-max-distance = Max distance
    .description = How far away a surface can be and still cast a shadow, in meters. Zero
        means there is no limit.
ui-opt-ao-intensity = Intensity
    .description = How dark the shadows get. This works the same way as the viewport's own
        ambient-occlusion panel.
ui-opt-ao-write-to = Write to
    .description = Which part of each point's color the baked shadow goes in. The alpha
        channel is always stored as-is; the color channels can be stored in sRGB.

## Remesh

ui-opt-remesh-topology = Made of
    .description = What the rebuilt surface is made of. Quads are what a modelling package
        expects to be handed - they subdivide, they carry edge loops, and they are what a
        rig deforms well. Where the surface folds too sharply for a flat quad to describe
        it, that part stays as triangles.
ui-opt-remesh-density = Density
    .description = How the target number of faces for each object is worked out - as a
        share of what it has now, or as one budget shared out across the objects by size.
ui-opt-remesh-ratio = Amount
    .description = The share of each object's current triangle count to aim for. 100% keeps
        about the same level of detail; lower numbers rebuild it coarser.
ui-opt-remesh-faces = Faces
    .description = How many faces to produce in total, shared out between the objects by
        surface area. The result lands near this number rather than exactly on it.
ui-opt-remesh-sharp-edges = Keep sharp edges
    .description = Lay the new edges along the model's own creases instead of running the
        pattern straight over them. Worth turning on for hard-surface and CAD parts; on
        organic shapes it only breaks the flow up.
ui-opt-remesh-crease-angle = Sharp above
    .description = How sharp a fold has to be, in degrees, before it counts as a crease.
ui-opt-remesh-align-boundaries = Follow open borders
    .description = Hold the new edges against any open border, so it comes back as one
        clean edge loop rather than a ragged fringe.
ui-opt-remesh-smoothing = Smoothing
    .description = How many rounds of evening-out to run over the rebuilt surface. A little
        makes the faces more uniform; a lot rounds off detail.
ui-opt-remesh-adaptive = Vary face size
    .description = How much smaller the faces get where the shape is detailed, and larger
        where it is flat. At 0 every face is the same size. Raising it does not add faces,
        it moves them: the flat parts give up what the curved parts take. 0.5 spends the
        same error everywhere; 1 gives every face the same turn, which is stronger and is
        what a hand retopology looks like.

## Shrinkwrap

ui-opt-shrinkwrap-resolution = Detail
    .description = How many grid steps across the object's longest side. Higher keeps more
        of the shape and takes longer; lower rounds it off and bridges wider gaps.
ui-opt-shrinkwrap-offset = Offset
    .description = How far to push the skin out from the object, in meters. A small push
        closes gaps a tight wrap leaves open; a negative one pulls the skin inside, for a
        collision shape.
ui-opt-shrinkwrap-largest-shell = Keep largest piece only
    .description = Throw away every closed piece but the biggest. Wrapping a messy object
        leaves small blobs around stray specks of geometry, and none of them is wanted.

## Export

ui-opt-export-settings = Export settings
    .description = Where and how the processed model is saved.
ui-opt-export-button = Export...
ui-opt-export-packaging = Packaging
ui-opt-export-hierarchy = Hierarchy
ui-opt-export-format = Format
ui-opt-packaging-single-explained = One file, with each mesh repeated as MeshName_LOD0 ... _LOD{ $last }
ui-opt-packaging-single-one-level = One file, with each mesh under its source name
ui-opt-hierarchy-rebuild-explained = The original tree of parts is rebuilt, with each part placed back where it belongs.
ui-opt-hierarchy-flat-explained = Each mesh becomes a top-level part, positioned in world space.
ui-opt-export-nothing = Nothing processed yet
    .description = Add an operation first. Right now there is no processed model to save.
ui-opt-export-waiting = Waiting for the current run to finish

## Fallbacks

ui-opt-unnamed-node = Node { $index }
ui-opt-packaging-file-per-lod-explained = { $count } files, one per level (..._LOD0.fbx, ...)

## The overlay comparison's legend

ui-opt-legend-solid = { $side } - shaded
ui-opt-legend-ghost = { $side } - { $style }
ui-opt-ao-srgb = sRGB encode
    .description = Stores the baked shadow in the color channels the way a picture would
        be stored. The alpha channel is always stored as-is, because that is how a game
        engine expects to read a mask.

## Longer explanations, shown above a group of parameters

ui-opt-weld-explained = Two points only join if they sit in exactly the same place; the
    tolerance applies to the other things being compared. Untick one of them to join
    across that kind of seam. For example, unticking normals joins the hard edges of a
    flat-shaded model, and those edges will then look smooth.
ui-opt-prune-explained = A fraction of the model's overall size. Loose pieces smaller than
    this are removed.
ui-opt-overdraw-explained = How much vertex-cache benefit may be traded away to reduce
    overdraw. 1.05 allows a 5% loss; 1.0 allows none.
ui-opt-remesh-explained = The surface is rebuilt from scratch, so the materials, texture
    layout and colors are read back off the original and copied across. Anything tied to
    the old points is not: bones, shape keys and per-point creases. Objects that bend are
    skipped and left as they are.
ui-opt-shrinkwrap-explained = The new skin is a fresh surface, so the materials, texture
    layout and colors are read back off the original and copied across. Anything tied to
    the old points is not: bones, shape keys and per-point creases. Objects that bend are
    skipped and left as they are.
ui-opt-bake-ao-explained = Objects whose names end in _LOD and a number are shaded only by
    objects in their own level (plus objects with no such ending), so you can bake a whole
    set of levels in one go. Hidden objects neither cast shadows nor receive them, so hide
    things like collision shapes in the Outliner first. Max distance is how far away a
    surface can be and still cast a shadow, in meters; 0 means no limit. Intensity above 1
    makes the shadows darker. The alpha channel is always stored as-is. To see the result,
    switch to the Vertex Colors material. If a Preserve Attributes simplify runs after
    this bake, raise its Colors weight above zero or it will ignore the baked shadows.
ui-opt-attribute-weights-explained = A higher weight protects that kind of data more, at
    the cost of a less accurate shape. Zero ignores it completely.
ui-opt-reduce-explained = Triangles is a share of this object's count at this point in the
    list. The simplifier stops early rather than go past the error limit. Unlike Generate
    LODs, this changes the model itself: every operation below it works on the reduced
    model, and an export with no LOD chain saves it in place of the original.
ui-opt-lod-explained = Triangles is a share of this object's count at this point in the
    list. Every level is made from that same model, not from the level above it, so one
    level's mistakes never pile up into the next. The simplifier stops early rather than
    go past the error limit.
ui-opt-export-explained = Where and how the processed model is saved, along with its
    levels if the list makes any. Everything the operations did not touch is saved just
    as it was in the original: faces, materials and textures, skins, blend shapes,
    animation curves and properties. Material changes made in the viewer are only a
    preview and are not saved.

ui-opt-remove-level = Remove this level
ui-opt-select-something = Pick an operation in the list to change its settings, or pick a
    part in the Outliner to give it settings of its own.
