### The status bar: the rendering-quality group on the right, the Tex viewport's
### own controls, and the Opt workspace's comparison controls in the centre span.

## Rendering quality

ui-status-bar-ibl = Image-based lighting
    .description = Light the scene from a baked HDR environment. The default Shaded look.
ui-status-bar-ao = Ambient occlusion
    .description = Screen-space contact shadow, darkening only the ambient light. It
        settles to a clean result about half a second after the view stops moving.
ui-status-bar-tonemap = Tone mapping
    .description = Apply a display curve to the linear HDR radiance. Off is a straight
        pass-through, which is how you check whether a bright area is really clipping.
ui-status-bar-anti-aliasing = Anti aliasing
    .description = Scene multisampling. Levels this GPU cannot render are left out of the
        menu rather than offered and failing.
ui-status-bar-viewport-background = Viewport background
    .description = Click to cycle black, three greys, white and a gradient.
ui-status-bar-model-stats = Model Stats
    .description = Show the measured counts card. Every figure on it is a real measurement
        - hover a row for what it means, click one to copy it.

## Tex viewport

ui-status-bar-texture-info = Texture Info
    .description = The viewed image's real properties: format, dimensions, channels, bit
        depth and size on disk.
ui-status-bar-background-black = Black background
ui-status-bar-background-white = White background
ui-status-bar-background-grey = Grey background
ui-status-bar-background-checker = Checker background
    .description = A checkerboard behind the image, which is how you see what its alpha
        channel is actually doing.
ui-status-bar-zoom = Zoom level
    .description = Click to toggle between 100% and fitting the image to the view.
ui-status-bar-zoom-percent = { $percent }%

## Opt comparison

ui-status-bar-split-view = Split view
    .description = Source on the left, processed on the right.
ui-status-bar-overlay-view = Overlay view
    .description = Both in one space, one shaded and the other a ghost. The most reliable
        way to see where a simplification moved the silhouette.
ui-status-bar-sync-cameras = Move both views' cameras together
ui-status-bar-swap-sides = Showing { $side } solid
    .description = Click, or press X, to swap which mesh is solid and which is the ghost.
ui-status-bar-lod-full = LOD 0 (full)
ui-status-bar-lod-level = LOD { $level }
ui-status-bar-nothing-processed = Nothing processed yet
    .description = Add an operation to the stack, and the result appears here as soon as
        the run lands.
