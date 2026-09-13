### Display names for values whose types live in `render`, `optimize`, `model` and
### `import`.
###
### Those crates keep their own `label()` as a stable English identifier - it goes
### into logs, warnings and the export report, where a translated string would make
### a bug report harder to read, and `model` is glam-only by invariant 10 and
### cannot depend on a catalog at all. `ui`'s `labels.rs` maps each variant to the
### message here, so the *screen* is translated and the *diagnostics* are not.

## Shading and material modes

ui-enums-material-mode-source = Source Material
ui-enums-material-mode-standard = Standard Material
ui-enums-material-mode-unique = Unique Mesh

## Buffer views

ui-enums-buffer-base-color = Base Color
ui-enums-buffer-world-normal = Normal (World)
ui-enums-buffer-normal-map = Normal Map (Tangent)
ui-enums-buffer-geometric-normal = Geometric Normal
ui-enums-buffer-tangent = Tangent
ui-enums-buffer-roughness = Roughness
ui-enums-buffer-metallic = Metallic
ui-enums-buffer-ao = Ambient Occlusion
ui-enums-buffer-emission = Emission
ui-enums-buffer-opacity = Opacity
ui-enums-buffer-uv = UV

## UV view

ui-enums-checker-greyscale = Greyscale
ui-enums-checker-color = Color

## Vertex colors

ui-enums-vertex-color-rgb = RGB channel
ui-enums-vertex-color-alpha = Alpha channel
ui-enums-vertex-color-rgb-alpha = RGB+A channel

## Scopes

ui-enums-scope-all-meshes = All Meshes
ui-enums-scope-only-selection = Only Selection
ui-enums-scope-visible-only = Only Visible

## Rendering quality

ui-enums-msaa-off = Off
ui-enums-msaa-2x = 2x
ui-enums-msaa-4x = 4x
ui-enums-msaa-8x = 8x
ui-enums-msaa-16x = 16x

ui-enums-env-hdr-01 = HDR 01
ui-enums-env-hdr-02 = HDR 02
ui-enums-env-hdr-03 = HDR 03
ui-enums-env-hdr-04 = HDR 04
ui-enums-env-hdr-05 = HDR 05
ui-enums-env-hdr-06 = HDR 06

ui-enums-quality-low = Low
ui-enums-quality-medium = Medium
ui-enums-quality-high = High

ui-enums-tonemap-pbr-neutral = PBR Neutral
ui-enums-tonemap-linear = Linear
ui-enums-tonemap-reinhard = Reinhard
ui-enums-tonemap-aces = ACES
ui-enums-tonemap-agx = AgX

## Backgrounds

ui-enums-background-black = Black
ui-enums-background-grey-25 = 25% Grey
ui-enums-background-grey-50 = 50% Grey
ui-enums-background-grey-75 = 75% Grey
ui-enums-background-white = White
ui-enums-background-gradient = Gradient
ui-enums-background-grey = Grey
ui-enums-background-checker = Checker

## Materials

ui-enums-workflow-roughness = Roughness
ui-enums-workflow-smoothness = Smoothness

ui-enums-alpha-opaque = Opaque
ui-enums-alpha-blend = Blend
ui-enums-alpha-clip = Clip

ui-enums-slot-base-color = Base Color
ui-enums-slot-normal = Normal
ui-enums-slot-roughness = Roughness
ui-enums-slot-metallic = Metallic
ui-enums-slot-ao = AO
ui-enums-slot-emissive = Emissive
ui-enums-slot-opacity = Opacity

# Channel picks are single letters that name the channel, not words.
ui-enums-channel-r = R
ui-enums-channel-g = G
ui-enums-channel-b = B
ui-enums-channel-a = A
ui-enums-channel-rgb = RGB

## Scene nodes

ui-enums-node-mesh = Mesh
ui-enums-node-bone = Bone
ui-enums-node-light = Light
ui-enums-node-camera = Camera
ui-enums-node-empty = Empty
ui-enums-node-other = Other

ui-enums-skinning-linear = Linear
ui-enums-skinning-rigid = Rigid
ui-enums-skinning-dq = Dual Quaternion (shown as linear)
ui-enums-skinning-blended-dq = Blended DQ / linear (shown as linear)

## Workspaces

ui-enums-workspace-3d = 3D
ui-enums-workspace-uv = UV
ui-enums-workspace-tex = Tex
ui-enums-workspace-opt = Opt

## Opt operations

# Each operation's name and what it does. Shown on the stack row, in the
# "Add operation" menu, and as the heading over its parameters - one message, so
# the three cannot describe it differently.
ui-enums-op-weld = Weld Vertices
    .description = Merge vertices across a seam. Every run already merges vertices that
        match in every attribute; this widens what counts as a match - dropping normals
        or UVs from the comparison, or allowing a tolerance - which does change the mesh.
ui-enums-op-filter-triangles = Filter Triangles
    .description = Remove degenerate triangles (two corners at one position) and exact
        duplicates. Opposite-winding duplicates are kept for double-sided geometry.
ui-enums-op-prune-components = Prune Components
    .description = Remove disconnected pieces smaller than the size threshold - stray
        shells and orphaned faces left behind by modelling.
ui-enums-op-reduce = Reduce
    .description = Simplify the mesh in place. The same simplifier the LOD chain uses, but
        it replaces the mesh instead of generating extra ones - so the reduced geometry is
        what the rest of the stack works on, and what the export writes in the source
        mesh's place.
ui-enums-op-simplify-lod = Generate LODs
    .description = Generate the LOD chain. Each level is simplified independently from the
        mesh as it stands at this point in the stack.
ui-enums-op-bake-ao = Bake AO to Vertex Colors
    .description = Raycast ambient occlusion at each vertex and write it into the
        vertex-color set. Objects named with a _LOD suffix bake only against their own
        LOD's geometry, so a whole visible chain bakes correctly in one run; hidden
        objects don't take part - hide collision shells first. Changes no geometry.
ui-enums-op-vertex-cache = Optimize Vertex Cache
    .description = Reorder triangles so the GPU's post-transform vertex cache hits more
        often. Changes no geometry; watch ACMR and ATVR on the stats card.
ui-enums-op-overdraw = Optimize Overdraw
    .description = Reorder triangles front to back within cache-friendly clusters so the
        GPU shades fewer hidden pixels. Changes no geometry.
ui-enums-op-vertex-fetch = Optimize Vertex Fetch
    .description = Reorder vertices into the order the index buffer reads them, and drop
        any vertex nothing references. Changes no geometry.

ui-enums-simplify-standard = Standard
ui-enums-simplify-attributes = Preserve Attributes
ui-enums-simplify-sloppy = Sloppy (fast)

ui-enums-ao-quality-low = Low (32 rays)
ui-enums-ao-quality-medium = Medium (64 rays)
ui-enums-ao-quality-high = High (128 rays)
ui-enums-ao-quality-ultra = Ultra (512 rays)

ui-enums-ao-target-alpha = Alpha channel
ui-enums-ao-target-rgb = RGB (grayscale)
ui-enums-ao-target-multiply-rgb = Multiply into RGB
ui-enums-ao-target-red = Red channel
ui-enums-ao-target-green = Green channel
ui-enums-ao-target-blue = Blue channel

ui-enums-lod-single-file = Single file, suffixed nodes
ui-enums-lod-file-per-lod = One file per LOD

ui-enums-hierarchy-rebuild = Rebuild original hierarchy
ui-enums-hierarchy-flat = Flat, world-baked meshes

ui-enums-fbx-binary = Binary
ui-enums-fbx-ascii = ASCII

## Opt comparison

ui-enums-ghost-xray = X-ray
ui-enums-ghost-wireframe = Wireframe
ui-enums-side-source = Source
ui-enums-side-processed = Processed
