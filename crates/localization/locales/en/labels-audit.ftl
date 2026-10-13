### The audit's typed values, as every surface that prints them names them: the
### viewer's Aud workspace and `review-cli audit` alike (`crates/labels`).
###
### Every check has four messages - its name, what it checks, why it matters
### (worded per target engine through `$engine`: unity, unreal or generic), and a
### hint for fixing it in the modelling program.

## Profile names

labels-audit-profile-modified = { $name } (changed)

## Severities, statuses and why a check did not run

labels-audit-severity-error = Error
labels-audit-severity-warning = Warning
labels-audit-severity-info = Info
labels-audit-skip-disabled = Switched off in this profile.
labels-audit-skip-no-properties = Needs the file's source properties, which could not be read.
labels-audit-skip-no-skin = The model has no skin.
labels-audit-skip-no-uv-set = The model does not have the UV set this check reads.
labels-audit-skip-invalid-pattern = One of this check's name patterns is not valid.
labels-audit-skip-empty = There is nothing in the model to check.
labels-audit-skip-cancelled = The check was interrupted.

## Categories

labels-audit-category-geometry = Geometry
labels-audit-category-transforms = Transforms
labels-audit-category-uv = UVs
labels-audit-category-skin = Skin
labels-audit-category-density = Density
labels-audit-category-naming = Naming
labels-audit-category-hierarchy = Hierarchy

## Diagnostic views and their legend

labels-audit-view-issues = Issues
    .description = Shows the finding picked in the Issues list on the model, over plain
        grey.
labels-audit-view-texel-density = Texel density
    .description = Colours each face by its texture pixels per meter: blue below the
        profile's target, green on it, red above it. The texture size and target are the
        Texel density check's, in the profile.
labels-audit-view-triangle-density = Triangle density
    .description = Colours each face by how small its triangles are for the size of their
        object: red where they are too small to draw efficiently, green at the limit, blue
        where there is room to spare. The limit is the Needs a LOD check's, in the profile.
labels-audit-view-overdraw = Overdraw
    .description = Replaces the view with a count of the surfaces under each pixel, hidden
        ones included: how many times a GPU with no depth rejection would shade it. Where
        parts of the model are stacked behind each other, the count climbs.
labels-audit-view-quad-overdraw = Quad overdraw
    .description = Replaces the view with the shading each visible pixel really costs. A
        GPU shades pixels in 2x2 quads, so a triangle that covers one pixel of a quad
        still pays for four: 1 is a surface whose triangles fill their quads, 4 is one
        made of triangles too small for the screen.

## Target engines

labels-audit-engine-unity = Unity
labels-audit-engine-unreal = Unreal
labels-audit-engine-generic = Generic

## Parameter labels

labels-audit-param-tolerance = Tolerance
    .description = How far a value may be from what the check expects before it counts.
labels-audit-param-max-ratio = Most hard edges
    .description = The share of a part's edges that may be hard before it is reported.
labels-audit-param-max-per-object = Per part
    .description = The most triangles one part may have.
labels-audit-param-max-total = Whole file
    .description = The most triangles the whole file may have.
labels-audit-param-max = Maximum
    .description = The largest value that still passes.
labels-audit-param-max-distance = Farthest from origin
    .description = How far a top-level object's pivot may sit from the world origin.
labels-audit-param-target = Target
    .description = The value the file should have.
labels-audit-param-resolution = Raster size
    .description = The texture size the UV layout is checked at, in pixels. Bigger is more
        exact and slower.
labels-audit-param-channel = UV set
    .description = Which UV set holds the lightmap UVs, counting from 0.
labels-audit-param-min-padding = Padding
    .description = The smallest gap, in pixels at the raster size, allowed between two
        islands.
labels-audit-param-texture-size = Texture size
    .description = The size of the texture the density is worked out for, in pixels.
labels-audit-param-min-pixel-area = Smallest triangle
    .description = How many pixels a triangle should cover on screen, on average. Smaller
        triangles are expensive for a GPU to draw.
labels-audit-param-screen-height = Screen height
    .description = The height of the screen the game runs at, in pixels.
labels-audit-param-min-screen-size = Smallest screen size
    .description = Parts that would only need a LOD below this screen size are not
        reported.
labels-audit-param-mesh = Static meshes
    .description = Name patterns a static mesh must match. Leave empty for no rule.
labels-audit-param-skinned = Skinned meshes
    .description = Name patterns a skinned mesh must match. Leave empty for no rule.
labels-audit-param-bone = Bones
    .description = Name patterns a bone must match. Leave empty for no rule.
labels-audit-param-empty = Empties
    .description = Name patterns an empty must match. Leave empty for no rule.
labels-audit-param-light = Lights
    .description = Name patterns a light must match. Leave empty for no rule.
labels-audit-param-camera = Cameras
    .description = Name patterns a camera must match. Leave empty for no rule.
labels-audit-param-prefixes = Collision prefixes
    .description = Name patterns that mark a collision mesh. The rest of the name must be a
        render mesh's name.
labels-audit-param-static = Static assets
    .description = Name patterns a top-level static asset must match.
labels-audit-param-ignore = Ignore
    .description = Name patterns of empties that are there on purpose, such as sockets.

## Axes

labels-audit-axis-x = +X
labels-audit-axis-negative-x = -X
labels-audit-axis-y = +Y
labels-audit-axis-negative-y = -Y
labels-audit-axis-z = +Z
labels-audit-axis-negative-z = -Z

## Values

labels-audit-value-count = { $value }
labels-audit-value-percent = { $value } %
labels-audit-value-meters = { $value } m
labels-audit-value-degrees = { $value } deg
labels-audit-value-per-square-meter = { $value } tris per sq m
labels-audit-value-px-per-meter = { $value } px/m
labels-audit-value-square-pixels = { $value } sq px
labels-audit-value-screen-size = { $value } of the screen
labels-audit-value-unit = { $value } m per unit
labels-audit-value-scale = { $x } x { $y } x { $z }
labels-audit-value-range = { $min } to { $max }
labels-audit-value-at-most = at most { $value }
labels-audit-value-at-least = at least { $value }

## The copied summary

labels-audit-text-heading = Audit of { $file } against the { $profile } profile
labels-audit-text-counts = { $errors } errors, { $warnings } warnings, { $info } info, { $passed } passed
labels-audit-text-finding = { $severity }: { $rule } ({ $count })
labels-audit-text-none = No findings.

## The checks

labels-audit-rule-degenerate-triangles = Degenerate triangles
labels-audit-rule-degenerate-triangles-what = Triangles with no area: their corners sit on one point or on one line.
labels-audit-rule-degenerate-triangles-why = They draw nothing but still cost a GPU time, and they can break normals, tangents and lightmap baking.
labels-audit-rule-degenerate-triangles-fix = Merge the collapsed corners, or delete the faces with your modelling program's mesh clean-up tool.
labels-audit-rule-non-manifold-edges = Non-manifold edges
labels-audit-rule-non-manifold-edges-what = Edges shared by more than two faces, like a fin standing on a surface.
labels-audit-rule-non-manifold-edges-why = Smoothing, simplifying and baking all assume each edge has one face on each side. A non-manifold edge can shade wrongly and stops many tools working on the mesh.
labels-audit-rule-non-manifold-edges-fix = Separate the extra face from the edge, or delete it.
labels-audit-rule-isolated-vertices = Loose points
labels-audit-rule-isolated-vertices-what = Points that no face uses.
labels-audit-rule-isolated-vertices-why = They are invisible but still stored, and they make the bounding box and the pivot wrong.
labels-audit-rule-isolated-vertices-fix = Delete loose vertices with your modelling program's clean-up tool.
labels-audit-rule-duplicate-vertices = Unwelded points
labels-audit-rule-duplicate-vertices-what = Separate points in one part that sit at the same place.
labels-audit-rule-duplicate-vertices-why = The surface looks joined but is not: it can split open when it bends, shade with a seam, and leak light in a lightmap.
labels-audit-rule-duplicate-vertices-fix = Merge vertices by distance.
labels-audit-rule-ngons = N-gons
labels-audit-rule-ngons-what = Faces with more than four corners.
labels-audit-rule-ngons-why = { $engine ->
        [unreal] Unreal triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
        [unity] Unity triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
       *[other] Every engine triangulates them on import its own way, which can differ from how your modelling program showed them and bend shading or a bake.
    }
labels-audit-rule-ngons-fix = Cut them into quads and triangles yourself, or triangulate the mesh before export.
labels-audit-rule-missing-normals = Missing normals
labels-audit-rule-missing-normals-what = Parts whose file carries no normals; the viewer worked them out.
labels-audit-rule-missing-normals-why = { $engine ->
        [unreal] Unreal will calculate its own, so the shading you see in the game is not the shading you authored.
        [unity] Unity will calculate its own, so the shading you see in the game is not the shading you authored.
       *[other] The engine will calculate its own, so the shading you see in the game is not the shading you authored.
    }
labels-audit-rule-missing-normals-fix = Turn on normals in your FBX export settings.
labels-audit-rule-missing-tangents = Missing tangents
labels-audit-rule-missing-tangents-what = Parts whose file carries no tangents.
labels-audit-rule-missing-tangents-why = A normal map is applied through the tangents. If the engine builds different ones from the baker, the normal map shows seams and wrong lighting.
labels-audit-rule-missing-tangents-fix = Export tangents and binormals, using the same tangent space your baker used.
labels-audit-rule-inverted-normals = Inside-out faces
labels-audit-rule-inverted-normals-what = Faces whose normals point the opposite way to how they are wound.
labels-audit-rule-inverted-normals-why = The face is drawn from the wrong side: it disappears, or lights as if from behind.
labels-audit-rule-inverted-normals-fix = Flip these faces, or conform the normals of the part.
labels-audit-rule-hard-edges = Mostly hard edges
labels-audit-rule-hard-edges-what = Parts where most edges are hard (the faces on each side shade separately).
labels-audit-rule-hard-edges-why = Every hard edge doubles the points along it on the GPU, so a part that is mostly hard edges is mostly duplicate points. Faceted shading is often a sign normals were lost.
labels-audit-rule-hard-edges-fix = Soften the edges that are not meant to be sharp, or set smoothing groups by angle.
labels-audit-rule-triangle-budget = Triangle budget
labels-audit-rule-triangle-budget-what = Parts, or the whole file, with more triangles than the profile allows.
labels-audit-rule-triangle-budget-why = Triangles cost memory and draw time on every frame the asset is seen.
labels-audit-rule-triangle-budget-fix = Reduce the mesh, or split detail into LODs. The Opt workspace can do both.
labels-audit-rule-draw-call-budget = Draw calls
labels-audit-rule-draw-call-budget-what = How many separate draws the file needs: one per part and material.
labels-audit-rule-draw-call-budget-why = Each draw has a fixed cost on the CPU, whatever its size, so many small pieces are slower than a few big ones.
labels-audit-rule-draw-call-budget-fix = Merge parts that share a material, and share materials between parts.
labels-audit-rule-scale = Scaled objects
labels-audit-rule-scale-what = Objects whose scale is not 1.
labels-audit-rule-scale-why = { $engine ->
        [unity] Unity carries the scale onto the imported object, so physics, effects and anything attached inherit it.
        [unreal] Unreal bakes or carries the scale depending on import settings, so the same file can arrive at different sizes.
       *[other] Engines treat object scale differently, so the asset can arrive at the wrong size or carry the scale into everything attached to it.
    }
labels-audit-rule-scale-fix = Apply (freeze) the scale before export.
labels-audit-rule-negative-scale = Mirrored objects
labels-audit-rule-negative-scale-what = Objects flipped by a negative scale. Only the object where the flip starts is listed.
labels-audit-rule-negative-scale-why = A mirror turns the mesh inside out. Some engines correct it and some draw the faces from the wrong side.
labels-audit-rule-negative-scale-fix = Apply the scale, then check the faces still point outwards.
labels-audit-rule-unfrozen = Unfrozen transforms
labels-audit-rule-unfrozen-what = Meshes with a rotation, a geometric transform or an offset pivot left on them.
labels-audit-rule-unfrozen-why = The geometry is not where its object says it is, so snapping, pivots and collision behave unexpectedly in the engine.
labels-audit-rule-unfrozen-fix = Freeze or apply the transforms and reset the pivot.
labels-audit-rule-pivot-offset = Off-origin pivot
labels-audit-rule-pivot-offset-what = Top-level objects whose pivot is away from the world origin.
labels-audit-rule-pivot-offset-why = The engine places the asset by its pivot, so it lands somewhere other than where it was dropped.
labels-audit-rule-pivot-offset-fix = Move the object back to the origin before export.
labels-audit-rule-unit = Scene unit
labels-audit-rule-unit-what = Whether the file's unit matches the target engine's.
labels-audit-rule-unit-why = { $engine ->
        [unity] Unity works in meters. A file in another unit is scaled on import, which either changes its size or adds a scale to the object.
        [unreal] Unreal works in centimeters. A file in another unit is scaled on import, which either changes its size or adds a scale to the object.
       *[other] A file in a different unit from the engine is scaled on import, which either changes its size or adds a scale to the object.
    }
labels-audit-rule-unit-fix = Set the scene unit, or the export unit, to the engine's.
labels-audit-rule-up-axis = Up axis
labels-audit-rule-up-axis-what = Whether the file's up axis matches the target engine's.
labels-audit-rule-up-axis-why = { $engine ->
        [unity] Unity is Y-up. A Z-up file is rotated on import, which leaves a rotation on the root object.
        [unreal] Unreal is Z-up. A Y-up file is rotated on import, which can leave a rotation on the root object.
       *[other] A file whose up axis differs from the engine's is rotated on import, which can leave a rotation on the root object.
    }
labels-audit-rule-up-axis-fix = Export with the engine's up axis.
labels-audit-rule-uv-missing = Missing UVs
labels-audit-rule-uv-missing-what = Parts with no UVs, with fewer UV sets than the other parts, or with their sets in a different order.
labels-audit-rule-uv-missing-why = Without UVs nothing can be textured. Parts with mismatched sets get the wrong UVs wherever they share a material or a lightmap.
labels-audit-rule-uv-missing-fix = Unwrap the part, and give every part the same UV sets in the same order.
labels-audit-rule-uv-out-of-range = UVs outside 0-1
labels-audit-rule-uv-out-of-range-what = Faces whose UVs reach outside the 0 to 1 square.
labels-audit-rule-uv-out-of-range-why = Fine for tiling textures, but for a unique texture or a bake those faces read pixels that belong to something else.
labels-audit-rule-uv-out-of-range-fix = If the texture is not meant to tile, pack these islands back into the square.
labels-audit-rule-uv-overlap = Overlapping UVs
labels-audit-rule-uv-overlap-what = Faces of one material that share texture space.
labels-audit-rule-uv-overlap-why = Overlaps share pixels, which is fine for mirrored or stacked parts but breaks texture painting and baking, where each face needs its own.
labels-audit-rule-uv-overlap-fix = If the overlap is not deliberate, move the islands apart.
labels-audit-rule-uv-flipped = Flipped UVs
labels-audit-rule-uv-flipped-what = UV islands that are mirrored, and faces folded over inside their island.
labels-audit-rule-uv-flipped-why = Mirroring is a normal way to save texture space, but a normal map on a mirrored island needs tangents that agree with it, and a fold always shows as a seam.
labels-audit-rule-uv-flipped-fix = Flip the island back, unless it is mirrored on purpose; unfold folded faces.
labels-audit-rule-lightmap-missing = Missing lightmap UVs
labels-audit-rule-lightmap-missing-what = Parts without the UV set the profile uses for lightmaps.
labels-audit-rule-lightmap-missing-why = { $engine ->
        [unity] Unity can generate lightmap UVs on import, but hand-made ones pack better and bleed less.
        [unreal] Unreal generates lightmap UVs if there are none, but hand-made ones pack better and bleed less.
       *[other] Baked lighting needs its own UV set with no overlaps; without one the engine has to generate it.
    }
labels-audit-rule-lightmap-missing-fix = Add a second UV set laid out for lightmaps.
labels-audit-rule-lightmap-overlap = Overlapping lightmap UVs
labels-audit-rule-lightmap-overlap-what = Faces of one part that share lightmap pixels.
labels-audit-rule-lightmap-overlap-why = Each pixel of a lightmap stores the light for one place. Faces that share one get the same light, which shows as dark or bright patches.
labels-audit-rule-lightmap-overlap-fix = Unwrap the lightmap UVs again so no two faces overlap.
labels-audit-rule-lightmap-padding = Lightmap padding
labels-audit-rule-lightmap-padding-what = Islands closer together than the padding, at the lightmap's resolution, in the UV set the profile names - the second one by default, where engines expect the lightmap.
labels-audit-rule-lightmap-padding-why = Light bleeds from one island into its neighbour when the lightmap is filtered or shrunk, which shows as light leaks along edges.
labels-audit-rule-lightmap-padding-fix = Pack the lightmap UVs again with more spacing.
labels-audit-rule-lightmap-out-of-range = Lightmap UVs outside 0-1
labels-audit-rule-lightmap-out-of-range-what = Faces whose lightmap UVs reach outside the 0 to 1 square.
labels-audit-rule-lightmap-out-of-range-why = A lightmap never tiles, so those faces read light that belongs to another face.
labels-audit-rule-lightmap-out-of-range-fix = Pack the lightmap UVs inside the square.
labels-audit-rule-influences = Too many bone influences
labels-audit-rule-influences-what = Points moved by more bones than the profile allows.
labels-audit-rule-influences-why = { $engine ->
        [unity] Unity keeps 4 bones per point by default and drops the lightest ones, so the mesh bends differently in the game.
        [unreal] Unreal keeps 8 bones per point by default and drops the lightest ones, so the mesh bends differently in the game.
       *[other] Engines keep a fixed number of bones per point and drop the lightest ones, so the mesh bends differently in the game.
    }
labels-audit-rule-influences-fix = Limit the influences per vertex to the engine's number and normalize the weights.
labels-audit-rule-unnormalized-weights = Unnormalized weights
labels-audit-rule-unnormalized-weights-what = Points whose bone weights do not add up to 1.
labels-audit-rule-unnormalized-weights-why = The viewer, like most engines, quietly rescales them, so the bend you see is not the weights you painted.
labels-audit-rule-unnormalized-weights-fix = Normalize the skin weights.
labels-audit-rule-bones-per-mesh = Bones per mesh
labels-audit-rule-bones-per-mesh-what = Skinned parts moved by more bones than the profile allows.
labels-audit-rule-bones-per-mesh-why = { $engine ->
        [unreal] On mobile, Unreal splits a mesh into sections of 75 bones or fewer, which costs extra draws.
       *[other] Some platforms limit how many bones one draw can use, so a mesh over the limit is split into more draws.
    }
labels-audit-rule-bones-per-mesh-fix = Split the mesh, or move weights onto fewer bones.
labels-audit-rule-unused-bones = Unused bones
labels-audit-rule-unused-bones-what = Bones that move nothing: no weights on them or on any bone below them.
labels-audit-rule-unused-bones-why = The engine still updates them every frame. End joints are often left in by accident.
labels-audit-rule-unused-bones-fix = Delete the bones you do not need, unless they are sockets or used by animation.
labels-audit-rule-bind-pose-mismatch = Bind pose differs from the rest pose
labels-audit-rule-bind-pose-mismatch-what = Bones whose bind pose (where the mesh was skinned) is not where the skeleton stands.
labels-audit-rule-bind-pose-mismatch-why = Engines that rebuild the bind pose from the skeleton deform the mesh on import, or drift it as soon as an animation plays.
labels-audit-rule-bind-pose-mismatch-fix = Return the skeleton to its bind pose, or re-bind the skin, before export.
labels-audit-rule-texel-density = Texel density
labels-audit-rule-texel-density-what = Parts whose texture pixels per meter are far from the profile's target, for the texture size it names.
labels-audit-rule-texel-density-why = Parts sharpen or blur next to each other when their texel density differs, which reads as an inconsistent asset.
labels-audit-rule-texel-density-fix = Scale the part's UV islands towards the target density.
labels-audit-rule-triangle-lod = Needs a LOD
labels-audit-rule-triangle-lod-what = Parts whose triangles get too small on screen below some size, worked out from their size and triangle count.
labels-audit-rule-triangle-lod-why = Tiny triangles make the GPU shade many pixels for nothing. A LOD that switches in below the listed screen size keeps them big enough.
labels-audit-rule-triangle-lod-fix = Add a LOD that switches in at the listed screen size. The Opt workspace can generate one.
labels-audit-rule-triangle-reduce = Too dense for its size
labels-audit-rule-triangle-reduce-what = Parts whose triangles are too small on screen even when the part fills the screen.
labels-audit-rule-triangle-reduce-why = A LOD cannot help, because the full-detail mesh is already too dense. It costs draw time and memory for detail no one can see.
labels-audit-rule-triangle-reduce-fix = Reduce the mesh, or move the fine detail into a normal map. The Opt workspace can reduce it.
labels-audit-rule-invalid-characters = Unsafe characters in names
labels-audit-rule-invalid-characters-what = Names with spaces, accents or symbols.
labels-audit-rule-invalid-characters-why = Engines, file systems and scripts handle them differently; some rename the object on import and break references to it.
labels-audit-rule-invalid-characters-fix = Use only letters, digits, underscores, dashes and dots.
labels-audit-rule-name-pattern = Naming convention
labels-audit-rule-name-pattern-what = Names that do not match the profile's patterns for their kind of object.
labels-audit-rule-name-pattern-why = A shared naming convention is how a team and its tools find assets.
labels-audit-rule-name-pattern-fix = Rename the object to match.
labels-audit-rule-lod-suffix = LOD names
labels-audit-rule-lod-suffix-what = _LOD chains with a missing level, no LOD0, two parts for one level, or the base name beside its own LODs.
labels-audit-rule-lod-suffix-why = { $engine ->
        [unreal] Unreal builds a LOD group from these names on import; a broken chain imports the wrong levels or none.
        [unity] Unity builds a LOD group from these names on import; a broken chain imports the wrong levels or none.
       *[other] Engines build LOD groups from these names on import; a broken chain imports the wrong levels or none.
    }
labels-audit-rule-lod-suffix-fix = Number the levels from _LOD0 up with no gaps, one part per level.
labels-audit-rule-collision-prefix = Orphaned collision
labels-audit-rule-collision-prefix-what = Collision meshes whose name does not point at a render mesh.
labels-audit-rule-collision-prefix-why = { $engine ->
        [unreal] Unreal attaches UCX_ collision to the mesh it names and drops one that names nothing, so the asset has no collision.
       *[other] Collision is attached to the mesh its name points at; one that points nowhere is dropped.
    }
labels-audit-rule-collision-prefix-fix = Rename the collision mesh after its render mesh, as UCX_<mesh>_01.
labels-audit-rule-asset-prefix = Asset prefix
labels-audit-rule-asset-prefix-what = Top-level assets whose name lacks the prefix for their kind.
labels-audit-rule-asset-prefix-why = { $engine ->
        [unreal] Unreal projects conventionally name static meshes SM_ and skeletal meshes SK_, so they sort and search by type.
       *[other] A type prefix lets assets sort and search by type.
    }
labels-audit-rule-asset-prefix-fix = Rename the asset with the right prefix.
labels-audit-rule-empty-nodes = Empty groups
labels-audit-rule-empty-nodes-what = Empties with nothing that draws or bends below them.
labels-audit-rule-empty-nodes-why = They add objects to the scene and to every transform update for nothing.
labels-audit-rule-empty-nodes-fix = Delete them, or rename them if they are sockets the engine needs.
labels-audit-rule-duplicate-names = Duplicate names
labels-audit-rule-duplicate-names-what = Objects that share a name.
labels-audit-rule-duplicate-names-why = Engines and scripts find objects by name, so only one of them can be found; some importers rename the others.
labels-audit-rule-duplicate-names-fix = Give each object a unique name.
labels-audit-rule-multiple-roots = Several top-level objects
labels-audit-rule-multiple-roots-what = Files with more than one object at the top of the hierarchy.
labels-audit-rule-multiple-roots-why = An engine may import them as separate assets, or under a root it adds itself.
labels-audit-rule-multiple-roots-fix = Parent everything under one root, unless the file is meant to hold several assets.
labels-audit-rule-lights-cameras = Lights and cameras
labels-audit-rule-lights-cameras-what = Lights and cameras in the file.
labels-audit-rule-lights-cameras-why = A model file usually should not carry them; they import as extra objects the scene has to manage.
labels-audit-rule-lights-cameras-fix = Delete them, or leave them out of the export selection.
labels-audit-rule-materials-per-mesh = Materials per mesh
labels-audit-rule-materials-per-mesh-what = Parts using more materials than the profile allows.
labels-audit-rule-materials-per-mesh-why = Each material on a part is a separate draw.
labels-audit-rule-materials-per-mesh-fix = Combine materials into an atlas, or split the part.

## Errors a user can act on

labels-audit-profile-newer = This audit profile was saved by a newer version of 3D Review
    (profile version { $found }; this version reads up to { $supported }). Open it with the
    version that saved it.
