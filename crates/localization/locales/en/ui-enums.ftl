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

## Animation curve shapes

ui-enums-curve-shape-smooth = Smooth
ui-enums-curve-shape-straight = Straight

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
ui-enums-workspace-aud = Aud

## Opt operations

ui-enums-shrinkwrap-method-winding = Distance field
ui-enums-shrinkwrap-method-voxel = Voxel

ui-enums-voxel-target-keep = Keep all
ui-enums-voxel-target-ratio = Share of original
ui-enums-voxel-target-triangles = Triangle count

ui-enums-normal-mode-project = From the original
ui-enums-normal-mode-generate = Worked out fresh

ui-enums-remesh-topology-triangles = Triangles
ui-enums-remesh-topology-quads = Quads

ui-enums-remesh-density-ratio = Ratio of current
ui-enums-remesh-density-absolute = Face count

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

## Viewport tool

#
# Shown by the mode notice when the tool is switched, so each of these is a
# whole phrase rather than a bare noun - "Select" alone reads as a command
# being issued rather than a state being reported.

ui-enums-tool-view = View mode
ui-enums-tool-select = Select mode

## Opt comparison

ui-enums-ghost-xray = X-ray
ui-enums-ghost-wireframe = Wireframe
ui-enums-side-source = Source
ui-enums-side-processed = Processed
