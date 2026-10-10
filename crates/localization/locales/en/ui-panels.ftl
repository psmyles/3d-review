### The option panels' rows: each row's label and the paragraph saying what it is
### for. The panels' window *titles* are in `ui-panel-titles.ftl`, because the
### toolbar shows those too.

## Wireframe

ui-panels-wireframe-width = Line width
    .description = How thick the edge lines are. It is measured on the screen, not in the
        model, so the lines keep the same weight however close you zoom in, and look the
        same on a high-resolution display as on an ordinary one.
ui-panels-wireframe-color = Wireframe color
    .description = The color of the edge lines. Feel free to drag it around: changing the
        color is instant, even on a very big model.

## Material mode

ui-panels-material-mode = Mode
    .description = What the surface shows while Source Material is active: the materials
        that came with the file, a plain grey clay look, or a different color for each
        part of the model.

## Buffers

ui-panels-buffer = Buffer
    .description = Which single ingredient of the shading to show on its own, with no
        lighting added. What you see on screen is the raw value itself.

## Bounding box

ui-panels-bounds-scope = Bounds
    .description = Which parts the box should wrap around: everything, only what you have
        selected, or only what is still visible.
ui-panels-box-color = Box color
    .description = The color of the box's lines.

## UV checker

ui-panels-checker-texture = Checker Texture
    .description = A grey checkerboard, or a colored one. The colored one makes it easy to
        spot a part whose texture is mirrored, because its colors run in the opposite
        order.
ui-panels-checker-tiling = Checker Tiling
    .description = How many squares fit across the texture. Use more squares for a small
        object and fewer for a large one, so the squares stay a sensible size.
ui-panels-uv-channel = Model UV channel
    .description = Which texture layout to use. Some models carry more than one; a second
        layout is often used for baked lighting or fine detail.
# One entry of the UV-set dropdown, for a set the file left unnamed.
ui-panels-uv-channel-numbered = Channel { $channel }

## Normals

ui-panels-normal-length = Normal Length
    .description = How long each line is, in meters. The change shows right away.
ui-panels-line-color = Line Color
    .description = The color of the lines. This also changes right away.

## Skeleton

ui-panels-bone-size = Bone Size
    .description = Makes the drawn bones thicker or thinner. The starting size already
        matches the model, so this just nudges it.
ui-panels-bone-color = Bone Color
    .description = The color of the bones. A bone you have selected in the Outliner is
        drawn in the highlight color instead, so you can always find it.

## Vertex colors

ui-panels-color-mode = Color Mode
    .description = Show the stored colors, show the alpha (see-through) value as shades of
        grey, or show the colors with alpha making parts see-through.

## Anti-aliasing

ui-panels-msaa = MSAA
    .description = Smooths jagged edges on the model. The wireframe, the grid and the
        other lines smooth their own edges, so they look the same at every level.
        Only the levels your GPU can handle are listed.

## Background

ui-panels-background = Background
    .description = What fills the empty space behind the model. A bright or dark
        background makes the outline easy to see; a mid-grey one is best for judging
        colors.

## Environment

ui-panels-environment = Environment
    .description = Which surrounding scene lights the model. Each one is a real photo of a
        place, used as the light source.
ui-panels-environment-background = Background
    .description = Show the surrounding scene behind the model, instead of a plain fill.
ui-panels-environment-intensity = Intensity
    .description = How bright the surrounding light is.
ui-panels-environment-rotation = Rotation
    .description = Spins the surrounding light around the model. Dragging it is instant,
        so it is a nice way to see how the model looks lit from different sides.

## Ambient occlusion

ui-panels-gtao-radius = Radius
    .description = How far the soft shadow reaches into corners and creases. The viewer
        works out a good size on its own; 1 means "use that", and other values scale it.
ui-panels-gtao-intensity = Intensity
    .description = How dark the soft shadows get.
ui-panels-gtao-thickness = Thickness
    .description = Helps with very thin surfaces, like a leaf or a sheet of paper. Raise it
        if a thin surface is casting a much bigger shadow than it should.
ui-panels-gtao-quality = Quality
    .description = How much work goes into each frame. Whatever you choose here, the shadows
        clean themselves up about half a second after the view stops moving.

## Tonemapper

ui-panels-tonemap-method = Method
    .description = The recipe used to turn the bright, wide range of light in the scene
        into colors a screen can show. If you know which game engine the model is for,
        picking the same recipe gives you a fairer preview.
