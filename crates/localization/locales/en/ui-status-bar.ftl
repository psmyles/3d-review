### The status bar: the rendering-quality group on the right, the Tex viewport's
### own controls, and the Opt workspace's comparison controls in the centre span.

## Rendering quality

ui-status-bar-ibl = Image-based lighting
    .description = Lights the model using a photo of a real place, so reflections and
        soft light look natural. This is on in the normal Shaded view.
ui-status-bar-ao = Ambient occlusion
    .description = Adds soft shadows in corners and creases, where light has trouble
        reaching. It only darkens the soft, surrounding light, never direct light. The
        shadows clean themselves up about half a second after the view stops moving.
ui-status-bar-tonemap = Tone mapping
    .description = Squeezes the very bright and very dark parts of the scene into colors
        your screen can show. Turn it off to see the raw values, which is a good way to
        check whether a bright spot is really too bright.
ui-status-bar-anti-aliasing = Anti aliasing
    .description = Smooths jagged edges. Only the levels your GPU can handle are
        listed.
ui-status-bar-viewport-background = Viewport background
    .description = Click to step through black, three greys, white and a soft gradient.
ui-status-bar-model-stats = Model Stats
    .description = Shows a card of measured numbers about the model. Every number is a real
        measurement. Hover a row to learn what it means, or click one to copy it.

## Tex viewport

ui-status-bar-background-black = Black background
ui-status-bar-background-white = White background
ui-status-bar-background-grey = Grey background
ui-status-bar-background-checker = Checker background
    .description = Puts a checkerboard behind the image. See-through parts let the
        checkerboard show through, so this is the easiest way to see what the alpha
        channel is doing.
ui-status-bar-zoom = Zoom level
    .description = Click to switch between actual size (100%) and fitting the whole image
        into the view.
ui-status-bar-zoom-percent = { $percent }%

## Opt comparison

ui-status-bar-split-view = Split view
    .description = Shows the original on the left and the processed version on the right.
ui-status-bar-overlay-view = Overlay view
    .description = Shows both models in the same spot, one solid and the other as a faint
        ghost. This is the best way to see exactly where a simplification changed the
        shape.
ui-status-bar-sync-cameras = Move both views' cameras together
ui-status-bar-swap-sides = Showing { $side } solid
    .description = Click, or press X, to swap which model is solid and which is the ghost.
ui-status-bar-lod-full = LOD 0 (full)
ui-status-bar-lod-level = LOD { $level }
ui-status-bar-nothing-processed = Nothing processed yet
    .description = Add an operation to the stack and the result will show up here as soon
        as it is ready.
