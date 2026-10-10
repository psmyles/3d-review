### The Aud workspace's chrome: the Issues tab, the Inspector's finding and
### profile views, the toolbar's findings count, and the copied summary.
###
### Every check has four messages - its name, what it checks, why it matters
### (worded per target engine through `$engine`: unity, unreal or generic), and a
### hint for fixing it in the modelling program.

## The Issues tab

ui-audit-profile-row = Profile: { $name }
    .description = The rules and limits the model is checked against. Click to see and
        change them in the Inspector.
ui-audit-profile-modified = { $name } (changed)
ui-audit-copy-summary = Copy summary
    .description = Copies a short text summary of the findings, ready to paste into a
        ticket or a chat.
ui-audit-save-report = Save report...
    .description = Saves every finding to a JSON file a pipeline can read. Faces and
        points are numbered the way your modelling program numbers them.
ui-audit-group-by-check = By check
    .description = Lists each check, then the parts that fail it.
ui-audit-group-by-object = By object
    .description = Lists each part, then the checks it fails.
ui-audit-show-passed = Show passed
    .description = Also lists the checks that found nothing, and the ones that did not
        run, so you can see everything that was checked.
ui-audit-running = Checking the model...
ui-audit-no-model = Open a model to check it.
ui-audit-all-clear = Nothing to report.
    .description = Every check that ran passed. Turn on Show passed to see them.
ui-audit-scene-group = Whole file
ui-audit-row-count = { $count }
ui-audit-not-evaluated = Not checked

## Severities, statuses and why a check did not run

ui-audit-severity-error = Error
ui-audit-severity-warning = Warning
ui-audit-severity-info = Info
ui-audit-status-passed = Passed
ui-audit-skip-disabled = Switched off in this profile.
ui-audit-skip-no-properties = Needs the file's source properties, which could not be read.
ui-audit-skip-no-skin = The model has no skin.
ui-audit-skip-no-uv-set = The model does not have the UV set this check reads.
ui-audit-skip-invalid-pattern = One of this check's name patterns is not valid.
ui-audit-skip-empty = There is nothing in the model to check.
ui-audit-skip-cancelled = The check was interrupted.

## Categories

ui-audit-category-geometry = Geometry
ui-audit-category-transforms = Transforms
ui-audit-category-uv = UVs
ui-audit-category-skin = Skin
ui-audit-category-density = Density
ui-audit-category-naming = Naming
ui-audit-category-hierarchy = Hierarchy

## The toolbar's findings count

ui-audit-bubble = { $errors ->
        [one] One error
       *[other] { $errors } errors
    }, { $warnings ->
        [one] one warning
       *[other] { $warnings } warnings
    }
    .description = Checks the model fails, from the Aud workspace. Info findings are not
        counted.
ui-audit-bubble-running = Checking the model...
ui-audit-bubble-running-text = ...

## Diagnostic views and their legend

ui-audit-view-issues = Issues
    .description = Shows the finding picked in the Issues list on the model, over plain
        grey.
ui-audit-view-texel-density = Texel density
    .description = Colours each face by its texture pixels per meter: blue below the
        profile's target, green on it, red above it. The texture size and target are the
        Texel density check's, in the profile.
ui-audit-view-triangle-density = Triangle density
    .description = Colours each face by how small its triangles are for the size of their
        object: red where they are too small to draw efficiently, green at the limit, blue
        where there is room to spare. The limit is the Needs a LOD check's, in the profile.
ui-audit-legend-texel = Texel density, { $size } px texture
ui-audit-legend-triangle = Triangle size at full screen
ui-audit-legend-dense = denser
ui-audit-legend-coarse = coarser

## The Inspector: a finding

ui-audit-measured = Measured
ui-audit-limit = Limit
ui-audit-profile = Profile
ui-audit-what = What this checks
ui-audit-why = Why it matters
ui-audit-fix = How to fix it
ui-audit-objects = Parts
    .description = Click a part to show only its problems.
ui-audit-scene-wide = The whole file
ui-audit-truncated = Showing the first { $shown } of { $total }.
ui-audit-summary-heading = Summary
ui-audit-summary-errors = Errors
ui-audit-summary-warnings = Warnings
ui-audit-summary-info = Info
ui-audit-summary-passed = Passed
ui-audit-summary-not-evaluated = Not checked
ui-audit-summary-time = Time
ui-audit-summary-hint = Pick a finding in the Issues list to see it on the model.

## The Inspector: the profile

ui-audit-profile-heading = Audit profile
ui-audit-profile-name = Name
    .description = A name for this profile, shown in the Issues list.
ui-audit-profile-engine = Target engine
    .description = The engine the model is going to. It sets the starting values of every
        check, and how each finding explains why it matters.
ui-audit-profile-reset = Reset to { $engine }
    .description = Replaces every setting with the built-in values for this engine.
ui-audit-profile-save = Save profile...
    .description = Saves this profile to a file you can share, so a team checks every model
        the same way.
ui-audit-profile-load = Load profile...
    .description = Replaces this profile with one from a file.
ui-audit-rule-enabled = Check
    .description = Whether this check runs.
ui-audit-rule-severity = Severity
    .description = How serious a failure of this check is. Info findings are not counted on
        the toolbar.
ui-audit-pattern-hint = Patterns, separated by commas (SM_*, *_LOD?)
ui-audit-pattern-invalid = Not a valid pattern

ui-audit-engine-unity = Unity
ui-audit-engine-unreal = Unreal
ui-audit-engine-generic = Generic

## Parameter labels

ui-audit-param-tolerance = Tolerance
    .description = How far a value may be from what the check expects before it counts.
ui-audit-param-max-ratio = Most hard edges
    .description = The share of a part's edges that may be hard before it is reported.
ui-audit-param-max-per-object = Per part
    .description = The most triangles one part may have.
ui-audit-param-max-total = Whole file
    .description = The most triangles the whole file may have.
ui-audit-param-max = Maximum
    .description = The largest value that still passes.
ui-audit-param-max-distance = Farthest from origin
    .description = How far a top-level object's pivot may sit from the world origin.
ui-audit-param-target = Target
    .description = The value the file should have.
ui-audit-param-resolution = Raster size
    .description = The texture size the UV layout is checked at, in pixels. Bigger is more
        exact and slower.
ui-audit-param-channel = UV set
    .description = Which UV set holds the lightmap UVs, counting from 0.
ui-audit-param-min-padding = Padding
    .description = The smallest gap, in pixels at the raster size, allowed between two
        islands.
ui-audit-param-texture-size = Texture size
    .description = The size of the texture the density is worked out for, in pixels.
ui-audit-param-min-pixel-area = Smallest triangle
    .description = How many pixels a triangle should cover on screen, on average. Smaller
        triangles are expensive for a GPU to draw.
ui-audit-param-screen-height = Screen height
    .description = The height of the screen the game runs at, in pixels.
ui-audit-param-min-screen-size = Smallest screen size
    .description = Parts that would only need a LOD below this screen size are not
        reported.
ui-audit-param-mesh = Static meshes
    .description = Name patterns a static mesh must match. Leave empty for no rule.
ui-audit-param-skinned = Skinned meshes
    .description = Name patterns a skinned mesh must match. Leave empty for no rule.
ui-audit-param-bone = Bones
    .description = Name patterns a bone must match. Leave empty for no rule.
ui-audit-param-empty = Empties
    .description = Name patterns an empty must match. Leave empty for no rule.
ui-audit-param-light = Lights
    .description = Name patterns a light must match. Leave empty for no rule.
ui-audit-param-camera = Cameras
    .description = Name patterns a camera must match. Leave empty for no rule.
ui-audit-param-prefixes = Collision prefixes
    .description = Name patterns that mark a collision mesh. The rest of the name must be a
        render mesh's name.
ui-audit-param-static = Static assets
    .description = Name patterns a top-level static asset must match.
ui-audit-param-ignore = Ignore
    .description = Name patterns of empties that are there on purpose, such as sockets.

## Axes

ui-audit-axis-x = +X
ui-audit-axis-negative-x = -X
ui-audit-axis-y = +Y
ui-audit-axis-negative-y = -Y
ui-audit-axis-z = +Z
ui-audit-axis-negative-z = -Z

## Values

ui-audit-value-count = { $value }
ui-audit-value-percent = { $value } %
ui-audit-value-meters = { $value } m
ui-audit-value-degrees = { $value } deg
ui-audit-value-per-square-meter = { $value } tris per sq m
ui-audit-value-px-per-meter = { $value } px/m
ui-audit-value-square-pixels = { $value } sq px
ui-audit-value-screen-size = { $value } of the screen
ui-audit-value-unit = { $value } m per unit
ui-audit-value-milliseconds = { $value } ms
ui-audit-value-scale = { $x } x { $y } x { $z }
ui-audit-value-range = { $min } to { $max }
ui-audit-value-at-most = at most { $value }
ui-audit-value-at-least = at least { $value }

## The copied summary

ui-audit-text-heading = Audit of { $file } against the { $profile } profile
ui-audit-text-counts = { $errors } errors, { $warnings } warnings, { $info } info, { $passed } passed
ui-audit-text-finding = { $severity }: { $rule } ({ $count })
ui-audit-text-none = No findings.

## The checks

ui-audit-rule-degenerate-triangles = Degenerate triangles
ui-audit-rule-degenerate-triangles-what = Triangles with no area: their corners sit on one point or on one line.
ui-audit-rule-degenerate-triangles-why = They draw nothing but still cost a GPU time, and they can break normals, tangents and lightmap baking.
ui-audit-rule-degenerate-triangles-fix = Merge the collapsed corners, or delete the faces with your modelling program's mesh clean-up tool.

ui-audit-rule-non-manifold-edges = Non-manifold edges
ui-audit-rule-non-manifold-edges-what = Edges shared by more than two faces, like a fin standing on a surface.
ui-audit-rule-non-manifold-edges-why = Smoothing, simplifying and baking all assume each edge has one face on each side. A non-manifold edge can shade wrongly and stops many tools working on the mesh.
ui-audit-rule-non-manifold-edges-fix = Separate the extra face from the edge, or delete it.

ui-audit-rule-isolated-vertices = Loose points
ui-audit-rule-isolated-vertices-what = Points that no face uses.
ui-audit-rule-isolated-vertices-why = They are invisible but still stored, and they make the bounding box and the pivot wrong.
ui-audit-rule-isolated-vertices-fix = Delete loose vertices with your modelling program's clean-up tool.

ui-audit-rule-duplicate-vertices = Unwelded points
ui-audit-rule-duplicate-vertices-what = Separate points in one part that sit at the same place.
ui-audit-rule-duplicate-vertices-why = The surface looks joined but is not: it can split open when it bends, shade with a seam, and leak light in a lightmap.
ui-audit-rule-duplicate-vertices-fix = Merge vertices by distance.

ui-audit-rule-ngons = N-gons
ui-audit-rule-ngons-what = Faces with more than four corners.
ui-audit-rule-ngons-why = { $engine ->
        [unreal] Unreal triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
        [unity] Unity triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
       *[other] Every engine triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
    }
ui-audit-rule-ngons-fix = Cut them into quads and triangles yourself, or triangulate the mesh before export.

ui-audit-rule-missing-normals = Missing normals
ui-audit-rule-missing-normals-what = Parts whose file carries no normals; the viewer worked them out.
ui-audit-rule-missing-normals-why = { $engine ->
        [unreal] Unreal will calculate its own, so the shading you see in the game is not the shading you authored.
        [unity] Unity will calculate its own, so the shading you see in the game is not the shading you authored.
       *[other] The engine will calculate its own, so the shading you see in the game is not the shading you authored.
    }
ui-audit-rule-missing-normals-fix = Turn on normals in your FBX export settings.

ui-audit-rule-missing-tangents = Missing tangents
ui-audit-rule-missing-tangents-what = Parts whose file carries no tangents.
ui-audit-rule-missing-tangents-why = A normal map is applied through the tangents. If the engine builds different ones from the baker, the normal map shows seams and wrong lighting.
ui-audit-rule-missing-tangents-fix = Export tangents and binormals, using the same tangent space your baker used.

ui-audit-rule-inverted-normals = Inside-out faces
ui-audit-rule-inverted-normals-what = Faces whose normals point the opposite way to how they are wound.
ui-audit-rule-inverted-normals-why = The face is drawn from the wrong side: it disappears, or lights as if from behind.
ui-audit-rule-inverted-normals-fix = Flip these faces, or conform the normals of the part.

ui-audit-rule-hard-edges = Mostly hard edges
ui-audit-rule-hard-edges-what = Parts where most edges are hard (the faces on each side shade separately).
ui-audit-rule-hard-edges-why = Every hard edge doubles the points along it on the GPU, so a part that is mostly hard edges is mostly duplicate points. Faceted shading is often a sign normals were lost.
ui-audit-rule-hard-edges-fix = Soften the edges that are not meant to be sharp, or set smoothing groups by angle.

ui-audit-rule-triangle-budget = Triangle budget
ui-audit-rule-triangle-budget-what = Parts, or the whole file, with more triangles than the profile allows.
ui-audit-rule-triangle-budget-why = Triangles cost memory and draw time on every frame the asset is seen.
ui-audit-rule-triangle-budget-fix = Reduce the mesh, or split detail into LODs. The Opt workspace can do both.

ui-audit-rule-draw-call-budget = Draw calls
ui-audit-rule-draw-call-budget-what = How many separate draws the file needs: one per part and material.
ui-audit-rule-draw-call-budget-why = Each draw has a fixed cost on the CPU, whatever its size, so many small pieces are slower than a few big ones.
ui-audit-rule-draw-call-budget-fix = Merge parts that share a material, and share materials between parts.

ui-audit-rule-scale = Scaled objects
ui-audit-rule-scale-what = Objects whose scale is not 1.
ui-audit-rule-scale-why = { $engine ->
        [unity] Unity carries the scale onto the imported object, so physics, effects and anything attached inherit it.
        [unreal] Unreal bakes or carries the scale depending on import settings, so the same file can arrive at different sizes.
       *[other] Engines treat object scale differently, so the asset can arrive at the wrong size or carry the scale into everything attached to it.
    }
ui-audit-rule-scale-fix = Apply (freeze) the scale before export.

ui-audit-rule-negative-scale = Mirrored objects
ui-audit-rule-negative-scale-what = Objects flipped by a negative scale. Only the object where the flip starts is listed.
ui-audit-rule-negative-scale-why = A mirror turns the mesh inside out. Some engines correct it and some draw the faces from the wrong side.
ui-audit-rule-negative-scale-fix = Apply the scale, then check the faces still point outwards.

ui-audit-rule-unfrozen = Unfrozen transforms
ui-audit-rule-unfrozen-what = Meshes with a rotation, a geometric transform or an offset pivot left on them.
ui-audit-rule-unfrozen-why = The geometry is not where its object says it is, so snapping, pivots and collision behave unexpectedly in the engine.
ui-audit-rule-unfrozen-fix = Freeze or apply the transforms and reset the pivot.

ui-audit-rule-pivot-offset = Off-origin pivot
ui-audit-rule-pivot-offset-what = Top-level objects whose pivot is away from the world origin.
ui-audit-rule-pivot-offset-why = The engine places the asset by its pivot, so it lands somewhere other than where it was dropped.
ui-audit-rule-pivot-offset-fix = Move the object back to the origin before export.

ui-audit-rule-unit = Scene unit
ui-audit-rule-unit-what = Whether the file's unit matches the target engine's.
ui-audit-rule-unit-why = { $engine ->
        [unity] Unity works in meters. A file in another unit is scaled on import, which either changes its size or adds a scale to the object.
        [unreal] Unreal works in centimeters. A file in another unit is scaled on import, which either changes its size or adds a scale to the object.
       *[other] A file in a different unit from the engine is scaled on import, which either changes its size or adds a scale to the object.
    }
ui-audit-rule-unit-fix = Set the scene unit, or the export unit, to the engine's.

ui-audit-rule-up-axis = Up axis
ui-audit-rule-up-axis-what = Whether the file's up axis matches the target engine's.
ui-audit-rule-up-axis-why = { $engine ->
        [unity] Unity is Y-up. A Z-up file is rotated on import, which leaves a rotation on the root object.
        [unreal] Unreal is Z-up. A Y-up file is rotated on import, which can leave a rotation on the root object.
       *[other] A file whose up axis differs from the engine's is rotated on import, which can leave a rotation on the root object.
    }
ui-audit-rule-up-axis-fix = Export with the engine's up axis.

ui-audit-rule-uv-missing = Missing UVs
ui-audit-rule-uv-missing-what = Parts with no UVs, with fewer UV sets than the other parts, or with their sets in a different order.
ui-audit-rule-uv-missing-why = Without UVs nothing can be textured. Parts with mismatched sets get the wrong UVs wherever they share a material or a lightmap.
ui-audit-rule-uv-missing-fix = Unwrap the part, and give every part the same UV sets in the same order.

ui-audit-rule-uv-out-of-range = UVs outside 0-1
ui-audit-rule-uv-out-of-range-what = Faces whose UVs reach outside the 0 to 1 square.
ui-audit-rule-uv-out-of-range-why = Fine for tiling textures, but for a unique texture or a bake those faces read pixels that belong to something else.
ui-audit-rule-uv-out-of-range-fix = If the texture is not meant to tile, pack these islands back into the square.

ui-audit-rule-uv-overlap = Overlapping UVs
ui-audit-rule-uv-overlap-what = Faces of one material that share texture space.
ui-audit-rule-uv-overlap-why = Overlaps share pixels, which is fine for mirrored or stacked parts but breaks texture painting and baking, where each face needs its own.
ui-audit-rule-uv-overlap-fix = If the overlap is not deliberate, move the islands apart.

ui-audit-rule-uv-flipped = Flipped UVs
ui-audit-rule-uv-flipped-what = UV islands that are mirrored, and faces folded over inside their island.
ui-audit-rule-uv-flipped-why = Mirroring is a normal way to save texture space, but a normal map on a mirrored island needs tangents that agree with it, and a fold always shows as a seam.
ui-audit-rule-uv-flipped-fix = Flip the island back, unless it is mirrored on purpose; unfold folded faces.

ui-audit-rule-lightmap-missing = Missing lightmap UVs
ui-audit-rule-lightmap-missing-what = Parts without the UV set the profile uses for lightmaps.
ui-audit-rule-lightmap-missing-why = { $engine ->
        [unity] Unity can generate lightmap UVs on import, but hand-made ones pack better and bleed less.
        [unreal] Unreal generates lightmap UVs if there are none, but hand-made ones pack better and bleed less.
       *[other] Baked lighting needs its own UV set with no overlaps; without one the engine has to generate it.
    }
ui-audit-rule-lightmap-missing-fix = Add a second UV set laid out for lightmaps.

ui-audit-rule-lightmap-overlap = Overlapping lightmap UVs
ui-audit-rule-lightmap-overlap-what = Faces of one part that share lightmap pixels.
ui-audit-rule-lightmap-overlap-why = Each pixel of a lightmap stores the light for one place. Faces that share one get the same light, which shows as dark or bright patches.
ui-audit-rule-lightmap-overlap-fix = Unwrap the lightmap UVs again so no two faces overlap.

ui-audit-rule-lightmap-padding = Lightmap padding
ui-audit-rule-lightmap-padding-what = Islands closer together than the padding, at the lightmap's resolution.
ui-audit-rule-lightmap-padding-why = Light bleeds from one island into its neighbour when the lightmap is filtered or shrunk, which shows as light leaks along edges.
ui-audit-rule-lightmap-padding-fix = Pack the lightmap UVs again with more spacing.

ui-audit-rule-lightmap-out-of-range = Lightmap UVs outside 0-1
ui-audit-rule-lightmap-out-of-range-what = Faces whose lightmap UVs reach outside the 0 to 1 square.
ui-audit-rule-lightmap-out-of-range-why = A lightmap never tiles, so those faces read light that belongs to another face.
ui-audit-rule-lightmap-out-of-range-fix = Pack the lightmap UVs inside the square.

ui-audit-rule-influences = Too many bone influences
ui-audit-rule-influences-what = Points moved by more bones than the profile allows.
ui-audit-rule-influences-why = { $engine ->
        [unity] Unity keeps 4 bones per point by default and drops the lightest ones, so the mesh bends differently in the game.
        [unreal] Unreal keeps 8 bones per point by default and drops the lightest ones, so the mesh bends differently in the game.
       *[other] Engines keep a fixed number of bones per point and drop the lightest ones, so the mesh bends differently in the game.
    }
ui-audit-rule-influences-fix = Limit the influences per vertex to the engine's number and normalize the weights.

ui-audit-rule-unnormalized-weights = Unnormalized weights
ui-audit-rule-unnormalized-weights-what = Points whose bone weights do not add up to 1.
ui-audit-rule-unnormalized-weights-why = The viewer, like most engines, quietly rescales them, so the bend you see is not the weights you painted.
ui-audit-rule-unnormalized-weights-fix = Normalize the skin weights.

ui-audit-rule-bones-per-mesh = Bones per mesh
ui-audit-rule-bones-per-mesh-what = Skinned parts moved by more bones than the profile allows.
ui-audit-rule-bones-per-mesh-why = { $engine ->
        [unreal] On mobile, Unreal splits a mesh into sections of 75 bones or fewer, which costs extra draws.
       *[other] Some platforms limit how many bones one draw can use, so a mesh over the limit is split into more draws.
    }
ui-audit-rule-bones-per-mesh-fix = Split the mesh, or move weights onto fewer bones.

ui-audit-rule-unused-bones = Unused bones
ui-audit-rule-unused-bones-what = Bones that move nothing: no weights on them or on any bone below them.
ui-audit-rule-unused-bones-why = The engine still updates them every frame. End joints are often left in by accident.
ui-audit-rule-unused-bones-fix = Delete the bones you do not need, unless they are sockets or used by animation.

ui-audit-rule-bind-pose-mismatch = Bind pose differs from the rest pose
ui-audit-rule-bind-pose-mismatch-what = Bones whose bind pose (where the mesh was skinned) is not where the skeleton stands.
ui-audit-rule-bind-pose-mismatch-why = Engines that rebuild the bind pose from the skeleton deform the mesh on import, or drift it as soon as an animation plays.
ui-audit-rule-bind-pose-mismatch-fix = Return the skeleton to its bind pose, or re-bind the skin, before export.

ui-audit-rule-texel-density = Texel density
ui-audit-rule-texel-density-what = Parts whose texture pixels per meter are far from the profile's target, for the texture size it names.
ui-audit-rule-texel-density-why = Parts sharpen or blur next to each other when their texel density differs, which reads as an inconsistent asset.
ui-audit-rule-texel-density-fix = Scale the part's UV islands towards the target density.

ui-audit-rule-triangle-lod = Needs a LOD
ui-audit-rule-triangle-lod-what = Parts whose triangles get too small on screen below some size, worked out from their size and triangle count.
ui-audit-rule-triangle-lod-why = Tiny triangles make the GPU shade many pixels for nothing. A LOD that switches in below the listed screen size keeps them big enough.
ui-audit-rule-triangle-lod-fix = Add a LOD that switches in at the listed screen size. The Opt workspace can generate one.

ui-audit-rule-triangle-reduce = Too dense for its size
ui-audit-rule-triangle-reduce-what = Parts whose triangles are too small on screen even when the part fills the screen.
ui-audit-rule-triangle-reduce-why = A LOD cannot help, because the full-detail mesh is already too dense. It costs draw time and memory for detail no one can see.
ui-audit-rule-triangle-reduce-fix = Reduce the mesh, or move the fine detail into a normal map. The Opt workspace can reduce it.

ui-audit-rule-invalid-characters = Unsafe characters in names
ui-audit-rule-invalid-characters-what = Names with spaces, accents or symbols.
ui-audit-rule-invalid-characters-why = Engines, file systems and scripts handle them differently; some rename the object on import and break references to it.
ui-audit-rule-invalid-characters-fix = Use only letters, digits, underscores, dashes and dots.

ui-audit-rule-name-pattern = Naming convention
ui-audit-rule-name-pattern-what = Names that do not match the profile's patterns for their kind of object.
ui-audit-rule-name-pattern-why = A shared naming convention is how a team and its tools find assets.
ui-audit-rule-name-pattern-fix = Rename the object to match.

ui-audit-rule-lod-suffix = LOD names
ui-audit-rule-lod-suffix-what = _LOD chains with a missing level, no LOD0, two parts for one level, or the base name beside its own LODs.
ui-audit-rule-lod-suffix-why = { $engine ->
        [unreal] Unreal builds a LOD group from these names on import; a broken chain imports the wrong levels or none.
        [unity] Unity builds a LOD group from these names on import; a broken chain imports the wrong levels or none.
       *[other] Engines build LOD groups from these names on import; a broken chain imports the wrong levels or none.
    }
ui-audit-rule-lod-suffix-fix = Number the levels from _LOD0 up with no gaps, one part per level.

ui-audit-rule-collision-prefix = Orphaned collision
ui-audit-rule-collision-prefix-what = Collision meshes whose name does not point at a render mesh.
ui-audit-rule-collision-prefix-why = { $engine ->
        [unreal] Unreal attaches UCX_ collision to the mesh it names and drops one that names nothing, so the asset has no collision.
       *[other] Collision is attached to the mesh its name points at; one that points nowhere is dropped.
    }
ui-audit-rule-collision-prefix-fix = Rename the collision mesh after its render mesh, as UCX_<mesh>_01.

ui-audit-rule-asset-prefix = Asset prefix
ui-audit-rule-asset-prefix-what = Top-level assets whose name lacks the prefix for their kind.
ui-audit-rule-asset-prefix-why = { $engine ->
        [unreal] Unreal projects conventionally name static meshes SM_ and skeletal meshes SK_, so they sort and search by type.
       *[other] A type prefix lets assets sort and search by type.
    }
ui-audit-rule-asset-prefix-fix = Rename the asset with the right prefix.

ui-audit-rule-empty-nodes = Empty groups
ui-audit-rule-empty-nodes-what = Empties with nothing that draws or bends below them.
ui-audit-rule-empty-nodes-why = They add objects to the scene and to every transform update for nothing.
ui-audit-rule-empty-nodes-fix = Delete them, or rename them if they are sockets the engine needs.

ui-audit-rule-duplicate-names = Duplicate names
ui-audit-rule-duplicate-names-what = Objects that share a name.
ui-audit-rule-duplicate-names-why = Engines and scripts find objects by name, so only one of them can be found; some importers rename the others.
ui-audit-rule-duplicate-names-fix = Give each object a unique name.

ui-audit-rule-multiple-roots = Several top-level objects
ui-audit-rule-multiple-roots-what = Files with more than one object at the top of the hierarchy.
ui-audit-rule-multiple-roots-why = An engine may import them as separate assets, or under a root it adds itself.
ui-audit-rule-multiple-roots-fix = Parent everything under one root, unless the file is meant to hold several assets.

ui-audit-rule-lights-cameras = Lights and cameras
ui-audit-rule-lights-cameras-what = Lights and cameras in the file.
ui-audit-rule-lights-cameras-why = A model file usually should not carry them; they import as extra objects the scene has to manage.
ui-audit-rule-lights-cameras-fix = Delete them, or leave them out of the export selection.

ui-audit-rule-materials-per-mesh = Materials per mesh
ui-audit-rule-materials-per-mesh-what = Parts using more materials than the profile allows.
ui-audit-rule-materials-per-mesh-why = Each material on a part is a separate draw.
ui-audit-rule-materials-per-mesh-fix = Combine materials into an atlas, or split the part.
