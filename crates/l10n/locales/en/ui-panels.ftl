### The option panels' rows: each row's label and the paragraph saying what it is
### for. The panels' window *titles* are in `ui-panel-titles.ftl`, because the
### toolbar shows those too.

## Wireframe

ui-panels-wireframe-color = Wireframe color
    .description = The edge color. Changing it rebuilds nothing - the colour is a shader
        uniform, so the drag is immediate even on a very heavy mesh.

## Material mode

ui-panels-material-mode = Mode
    .description = What the filled faces show when Source Material is active: the imported
        materials, a plain matte grey, or a different hue per mesh part.

## Buffers

ui-panels-buffer = Buffer
    .description = Which shading input to draw flat, with no lighting and no tone mapping,
        so the pixel you see is the value itself.

## Bounding box

ui-panels-bounds-scope = Bounds
    .description = Which meshes the box encloses: every mesh, only the selection, or only
        what is still visible.
ui-panels-box-color = Box color
    .description = The box's edge color.

## UV checker

ui-panels-checker-texture = Checker Texture
    .description = Greyscale, or a colour checker. The colour one makes a mirrored shell
        obvious, because its hue sequence runs backwards.
ui-panels-checker-tiling = Checker Tiling
    .description = How many checker squares per UV unit. Raise it to judge texel density on
        a small prop, lower it on a large one.
ui-panels-uv-channel = Model UV channel
    .description = Which UV set to sample. A second set is usually a lightmap or a
        detail-texture layout.
# One entry of the UV-set dropdown, for a set the file left unnamed.
ui-panels-uv-channel-numbered = Channel { $channel }

## Normals

ui-panels-normal-length = Normal Length
    .description = How far each line extends, in metres. Updates live.
ui-panels-line-color = Line Color
    .description = The line color. Also live.

## Skeleton

ui-panels-bone-size = Bone Size
    .description = A multiplier on the drawn bone thickness. The base size follows the
        model, so this is a correction rather than an absolute.
ui-panels-bone-color = Bone Color
    .description = The bone color. The bone selected in the Outliner is drawn in the
        viewport's selection color instead, whatever this is set to.

## Vertex colors

ui-panels-color-mode = Color Mode
    .description = Show the colour channels, the alpha channel as greyscale, or the colours
        with alpha driving opacity.

## Anti-aliasing

ui-panels-msaa = MSAA
    .description = Scene multisampling, applied to the whole viewport including the
        wireframe and the line overlays. Levels this GPU cannot render are left out of
        the menu rather than offered and failing.

## Background

ui-panels-background = Background
    .description = The viewport's background fill. Judging a silhouette needs contrast;
        judging a material's value needs a neutral ground.

## Environment

ui-panels-environment = Environment
    .description = Which baked HDR environment lights the shaded view.
ui-panels-environment-background = Background
    .description = Draw the environment as the viewport background instead of the flat
        fill.
ui-panels-environment-intensity = Intensity
    .description = A multiplier on the environment's brightness.
ui-panels-environment-rotation = Rotation
    .description = Turn the environment around the model. Applied while sampling, so it
        never rebuilds the lighting maps - which is why the drag is immediate.

## Ambient occlusion

ui-panels-gtao-radius = Radius
    .description = A multiplier on the automatic radius, which follows what the viewport is
        showing rather than how big the model is. 1 is automatic.
ui-panels-gtao-intensity = Intensity
    .description = How strongly the occlusion darkens the ambient term.
ui-panels-gtao-thickness = Thickness
    .description = Compensation for thin occluders. Raise it when a thin surface casts far
        more occlusion than it should.
ui-panels-gtao-quality = Quality
    .description = Sample count and how many times the edge-aware denoise runs. The result
        converges over about half a second whenever the view is still, whatever this is
        set to.

## Tonemapper

ui-panels-tonemap-method = Method
    .description = The curve applied to the linear HDR radiance before it is encoded for
        display. Matching it to your target engine makes the viewport a fairer preview.
