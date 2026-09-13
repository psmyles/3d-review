### The model-stats card and the Opt workspace's second card.
###
### A row's message is its label; `.description` is what the row means, in the
### artist's terms rather than the renderer's. Both cards read the same messages,
### so the same figure cannot end up described two different ways.

## Scope columns

ui-stats-scope-all = All
    .description = Every part of the model, whether it is selected or hidden or not. These
        are the counts the file itself carries.
ui-stats-scope-sel = Sel
    .description = Only what you have selected in the Outliner: a part and everything under
        it, or every triangle that uses a material. Empty when nothing is selected. A
        material has no vertex count of its own, so that row stays empty for one.
ui-stats-scope-vis = Vis
    .description = Only the parts that are still showing. This matches All until you hide
        something. Tip: { $modifier }+click a part's eye icon to show only that part.

## Model rows

ui-stats-draws = Draws
    .description = How many separate drawing instructions the graphics card needs for this
        model - one for each different material. Fewer is cheaper. Combining materials is
        what brings this number down.

ui-stats-polys = Polys
    .description = The number of faces as they were made in the modelling program. Four-
        sided and many-sided faces each count once here.

ui-stats-tris = Tris
    .description = The number of triangles once every face is cut into triangles. This is
        what the graphics card actually draws, and what a game's triangle budget counts.

ui-stats-verts = Verts
    .description = The number of points, counted the way your modelling program counts
        them. It does not include the extra copies that hard edges and texture seams force
        the graphics card to keep.

ui-stats-gpu-verts = GPU Verts
    .description = The number of points a game would really have to store. A point sitting
        on a hard edge or a texture seam has to be stored once for each side, so this is
        usually higher than Verts.

ui-stats-vtx-splits = Vtx Splits
    .description = How much bigger GPU Verts is than Verts, as a percentage. It is the price
        of this model's hard edges and texture seams. A few percent is normal. Hundreds of
        percent means every face has its own shading or the texture layout is chopped into
        very many pieces.

ui-stats-uv-sets = UV Sets
    .description = How many texture layouts the model carries. A second one is often used
        for baked lighting or fine detail.

ui-stats-bones = Bones
    .description = How many bones are in the skeleton this model is attached to. Only
        shown for rigged models.

ui-stats-clips = Clips
    .description = How many animations the file holds. You can find them in the Outliner's
        Animations tab.

ui-stats-unit = Unit
    .description = The unit of measurement the file says it uses (centimeters, inches, and
        so on). The viewer converts everything to meters, but this is what the file
        claimed. When a model comes in far too big or too small, this is usually why.

ui-stats-fps = FPS
    .description = How many frames per second the viewer is drawing right now. This tells
        you about your computer, not about the model.

## The Opt card

ui-stats-mesh-verts = Mesh Verts
    .description = The number of points in the processed model, counted the same way as
        GPU Verts on the other card, so the two numbers can be compared fairly.

ui-stats-acmr = ACMR
    .description = A measure of how well the graphics card can reuse points it has already
        worked on. 3.0 means it never reuses any; about 0.5 is the best a closed shape can
        reach. Lower is cheaper. The Optimize Vertex Cache operation is what improves it.

ui-stats-atvr = ATVR
    .description = How many times, on average, each point has to be worked on. 1.0 means
        exactly once, which is perfect; 2.0 means everything is done twice over. Unlike
        ACMR, this number does not change just because the model has more or fewer
        triangles, so it is the fairer one to watch for a single model.

ui-stats-overdraw = Overdraw
    .description = How often the graphics card paints a spot on screen and then paints over
        it again. 1.0 means every spot is painted once; higher means wasted work on parts
        that end up hidden. The Optimize Overdraw operation lowers this.

ui-stats-overfetch = Overfetch
    .description = How much the graphics card has to re-read the same memory while drawing
        the model. 1.0 means it reads everything once; higher means the drawing order jumps
        around. The Optimize Vertex Fetch operation lowers this.

ui-stats-error = Error
    .description = How far this simplified level has drifted from the model it was made
        from, as measured by the simplifier. Normally it is a fraction of the model's
        overall size; with the absolute-error option on, it is in world units.

# Card headings on the Opt side.
ui-stats-source-nothing-applied = Source (nothing applied)
ui-stats-processed = Processed
ui-stats-processed-lod = Processed - LOD { $level }

## The Tex viewport's card

ui-stats-tex-format = Format
    .description = The kind of image file this is, such as PNG or TGA.
ui-stats-tex-dimension = Dimension
    .description = The width and height of the image in pixels. Some game engines prefer
        sizes like 512, 1024 or 2048, so this is where you check.
# The Dimension row's value. A message rather than a format string at the call
# site, because the mark between the two numbers is a typographic choice a locale
# makes for itself, and because a digit group separator belongs to the locale too.
ui-stats-tex-dimension-value = { $width } x { $height }

ui-stats-tex-channels = Channels
    .description = Which color channels the file stores: just grey, grey plus alpha, color,
        or color plus alpha.
ui-stats-tex-bit-depth = Bit depth
    .description = How many bits of information each pixel holds, adding up all the
        channels. This is the way most image tools show it.
ui-stats-tex-file-size = File size
    .description = How much space the file takes on your disk. This is not the same as how
        much memory it uses once loaded.

# Channel-layout labels.
ui-stats-channels-grey = Grey
ui-stats-channels-grey-alpha = Grey+A
ui-stats-channels-rgb = RGB
ui-stats-channels-rgba = RGBA

# Stands in for a value that was measured and has nothing to report.
ui-stats-unmeasured = -
