### The model-stats card and the Opt workspace's second card.
###
### A row's message is its label; `.description` is what the row means, in the
### artist's terms rather than the renderer's. Both cards read the same messages,
### so the same figure cannot end up described two different ways.

## Scope columns

ui-stats-scope-all = All
    .description = Every mesh in the file - selected or not, hidden or not. These are the
        counts the source file itself reports.
ui-stats-scope-sel = Sel
    .description = Only what is selected in the Outliner: a node and everything under it,
        or every triangle of a material slot. Empty when nothing is selected, and a
        material has no authored vertex count to report.
ui-stats-scope-vis = Vis
    .description = Only the meshes the Outliner is still showing. Identical to All until
        you hide something - { $modifier }+click a row's eye to isolate one mesh.

## Model rows

ui-stats-draws = Draws
    .description = Draw calls this mesh costs - one per distinct material. Each is a
        separate command to the GPU, so fewer is cheaper; merging materials is what
        brings it down.

ui-stats-polys = Polys
    .description = Polygons as authored in the source file: quads and n-gons counted once
        each, before triangulation.

ui-stats-tris = Tris
    .description = Triangles after triangulation - what the GPU actually rasterizes, and
        what an engine's triangle budget counts.

ui-stats-verts = Verts
    .description = Vertices as the source file counts them (control points) - the number
        your DCC's stats show. It ignores the extra vertices that hard edges and UV
        seams force the GPU to store.

ui-stats-gpu-verts = GPU Verts
    .description = Vertices an engine would actually upload: one per unique combination of
        position, normal, UVs and colour, per material. A vertex on a hard edge or a UV
        seam is stored once per side.

ui-stats-vtx-splits = Vtx Splits
    .description = How much larger the GPU vertex count is than the authored one - the
        price of this asset's hard edges and UV seams. A few percent is normal; hundreds
        of percent means per-face normals or heavily fragmented UVs.

ui-stats-uv-sets = UV Sets
    .description = UV channels the mesh carries. A second set is usually a lightmap or a
        detail-texture layout.

ui-stats-bones = Bones
    .description = Joints in the skeleton this mesh is bound to. Rigged meshes only.

ui-stats-clips = Clips
    .description = Animation clips (FBX animation stacks) the file carries. Listed in the
        Outliner's Animations tab.

ui-stats-unit = Unit
    .description = The world unit the source file declared (centimetres, inches...). Import
        normalises every model to metres; this is what the file itself claimed, which is
        where scale mismatches come from.

ui-stats-fps = FPS
    .description = Frames per second this preview is drawing at - a property of the viewer
        and your GPU, not of the asset.

## The Opt card

ui-stats-mesh-verts = Mesh Verts
    .description = Vertices in the processed mesh's own buffer, measured the same way as
        the source card's GPU Verts so the two figures compare like with like.

ui-stats-acmr = ACMR
    .description = Average Cache Miss Ratio: vertex-shader runs per triangle, simulated
        against a 16-entry GPU vertex cache. 3.0 means no vertex is ever reused; about
        0.5 is the best a closed mesh can reach. Lower is cheaper - Optimize Vertex Cache
        is the operation that moves it.

ui-stats-atvr = ATVR
    .description = Average Transformed Vertex Ratio: how many times the average vertex gets
        shaded. 1.0 means each is shaded exactly once; 2.0 means the mesh is transformed
        twice over. Unlike ACMR it does not shift with the triangle count, so it is the
        fairer figure for judging one mesh's ordering against itself.

ui-stats-overdraw = Overdraw
    .description = Pixels shaded divided by pixels covered, measured from viewpoints around
        the mesh. 1.0 means nothing is drawn over anything; higher means the GPU shades
        pixels a later triangle then hides. Optimize Overdraw trades cache behaviour for
        this.

ui-stats-overfetch = Overfetch
    .description = Vertex-buffer bytes read divided by the buffer's size. 1.0 means each
        byte is fetched once; higher means the index order jumps around and the GPU
        re-reads the same memory. Optimize Vertex Fetch reorders the buffer to bring it
        down.

ui-stats-error = Error
    .description = How far this LOD deviates from the mesh it was simplified from, as the
        simplifier measured it - a fraction of the model's overall size (or world units,
        under the absolute-error flag).

# Card headings on the Opt side.
ui-stats-source-nothing-applied = Source (nothing applied)
ui-stats-processed = Processed
ui-stats-processed-lod = Processed - LOD { $level }

## The Tex viewport's card

ui-stats-tex-format = Format
    .description = The image's container format, as the decoder read it.
ui-stats-tex-dimension = Dimension
    .description = Pixel width and height. Powers of two matter to some engines and
        compressors; this is where you check.
# The Dimension row's value. A message rather than a format string at the call
# site, because the mark between the two numbers is a typographic choice a locale
# makes for itself, and because a digit group separator belongs to the locale too.
ui-stats-tex-dimension-value = { $width } x { $height }

ui-stats-tex-channels = Channels
    .description = The channel layout the file stores, before anything the material does
        with it.
ui-stats-tex-bit-depth = Bit depth
    .description = Total bits per pixel: per-channel depth times channels, the convention
        texture tools display.
ui-stats-tex-file-size = File size
    .description = The file's size on disk, not its size in memory or on the GPU.

# Channel-layout labels.
ui-stats-channels-grey = Grey
ui-stats-channels-grey-alpha = Grey+A
ui-stats-channels-rgb = RGB
ui-stats-channels-rgba = RGBA

# Stands in for a value that was measured and has nothing to report.
ui-stats-unmeasured = -
