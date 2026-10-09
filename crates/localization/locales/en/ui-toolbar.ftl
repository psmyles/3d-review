### The toolbar's tool buttons. Each is icon-only, so its tooltip is the only
### place its name appears - which is why every one of these carries a
### `.description` paragraph as well.
###
### "Right-click for options" is deliberately *not* repeated in each message: the
### `Tip` builder appends it once, so it is translated once.

## The menu

ui-toolbar-menu = Menu
    .description = Opens and closes files, sets whether your tool settings are kept for
        next time, shows the viewer's log, and leads to the manual, updates and the
        project's pages.

# The four submenus.
ui-toolbar-menu-file = File
ui-toolbar-menu-preferences = Preferences
ui-toolbar-menu-debug = Debug
ui-toolbar-menu-help = Help

ui-toolbar-menu-open-file = Open File...
ui-toolbar-menu-open-file-shortcut = { $modifier }+O
ui-toolbar-menu-open-recent = Open Recent
ui-toolbar-menu-clear-recent-files = Clear Recent Files
ui-toolbar-menu-close-file = Close File
ui-toolbar-menu-close-file-shortcut = { $modifier }+N
ui-toolbar-menu-save = Save Comments
ui-toolbar-menu-save-shortcut = { $modifier }+S
ui-toolbar-menu-save-as = Save Comments As...
ui-toolbar-menu-save-as-shortcut = { $modifier }+Shift+S
ui-toolbar-menu-exit = Exit

ui-toolbar-menu-remember-settings = Remember Settings
    .description = When this is ticked, the choices you make in the tools' options windows,
        such as colors, sizes, the environment and the ambient occlusion settings, are kept
        and come back the next time you start the viewer.

ui-toolbar-menu-view-log = View Log
    .description = Opens a window listing what the viewer has reported since it started:
        files opened, how long they took, warnings and errors. The same lines are saved to
        a file for the rest of the day, which is useful to attach to a bug report.

ui-toolbar-menu-tracy-profiler = Tracy Profiler
    .description = For developers measuring the viewer's performance. When this is ticked, the
        viewer starts with the Tracy profiler enabled, as if it had been launched with
        --tracy, and a Tracy window can connect to it. It takes effect the next time you
        start the viewer.

ui-toolbar-menu-about = About
ui-toolbar-menu-documentation = Documentation
ui-toolbar-menu-check-for-updates = Check for Updates
ui-toolbar-menu-report-issue = Report an Issue
ui-toolbar-menu-credits = Credits

## Shading modes

ui-toolbar-wireframe-only = Wireframe Only
    .description = Shows just the lines that make up the model, with nothing filled in.
        Think of it as looking at the model's skeleton of edges.
ui-toolbar-unlit = Unlit
    .description = Shows the model's colors with no lighting at all. This is handy when you
        want to see the plain painted colors without shadows or shine getting in the way.
ui-toolbar-shaded = Shaded
    .description = Shows the model with realistic lighting, the way a game would draw it.
        This is the normal view, and the closest to what a game engine will show you.

ui-toolbar-show-wireframe = Show Wireframe
    .description = Draws the model's edges on top of the filled surface. Edges that are
        hidden behind the model stay hidden, just like the surface they sit on.
ui-toolbar-backface-rendering = Backface Rendering
    .description = Every face of a model has a front and a back. With this off, the back
        sides are not drawn, which is a quick way to spot faces that are pointing the
        wrong way. With this on, both sides are drawn.

## Active material

ui-toolbar-source-material = Source Material
    .description = Shows the materials that came with the file, plus any changes you made in
        the Inspector. Click again to switch between Source, Standard and Unique.
ui-toolbar-uv-checker = UV Checker
    .description = Covers the model in a checkerboard pattern. This is the quickest way to
        see whether a texture would look stretched, mirrored, or blurrier in some places
        than others.
ui-toolbar-vertex-colors = Vertex Colors
    .description = Shows the colors that are stored on the model's points, instead of the
        material. You can look at the color, the alpha (see-through) value, or both.
ui-toolbar-buffers = Buffers
    .description = Shows one ingredient of the shading at a time, such as just the color or
        just the normal map, with no lighting added. Click again to move to the next one.
ui-toolbar-skin-weights = Skin Weights
    .description = Shows a heat map of how strongly the chosen bones pull on each part of
        the model. Pick some bones in the Outliner's Scene tab first.

## Debug overlays

ui-toolbar-bounding-box = Bounding Box
    .description = Draws a box that just fits around the model, with its size written on
        the edges. The labels hide behind the model when they should.
ui-toolbar-face-normals = Face Normals
    .description = Draws one short line sticking out of each face, showing which way the
        face is pointing. This is the fastest way to find faces that are flipped inside
        out.
ui-toolbar-vertex-normals = Vertex Normals
    .description = Draws one short line at each point of the model, showing the direction
        used for smooth shading. Helpful when a surface looks oddly shaded and you want to
        know why.
ui-toolbar-uv-seams = UV Seams
    .description = Highlights every edge where the texture layout is cut. Each of these
        cuts makes the game store an extra copy of a point, so fewer is usually better.
ui-toolbar-skeleton = Skeleton
    .description = Draws the bones inside a rigged model, with small markers at the joints.
        It follows the animation as it plays.

## Display

ui-toolbar-grid = Grid
    .description = Shows the floor grid with its axis lines, so you can tell which way is
        up and how big the model is.
ui-toolbar-axis-gizmo = Axis Gizmo
    .description = Shows the small direction marker in the corner. Click one of its arms to
        turn the camera to look straight down that axis.
ui-toolbar-pivot = Pivot
    .description = Shows a marker at the model's own center point, the spot it rotates
        around.
ui-toolbar-select-tool = Select
    .description = Switches the left mouse button between turning the camera and
        picking what it is over. Dragging still turns the camera either way - only
        a click that does not move selects. Press { $key } to switch.
# The key that toggles the Select tool, named inside the tooltip above. Its own
# message so a keyboard layout that needs a different letter can say so without
# the sentence having to be rebuilt around it.
ui-toolbar-select-tool-key = Q
ui-toolbar-comment-tool = Comment
    .description = Click the model to leave a review comment pinned to that spot, or click
        empty space for a comment about the view. Dragging still turns the camera. Press
        { $key } to switch.
# The key that toggles the Comment tool, named inside the tooltip above.
ui-toolbar-comment-tool-key = C

ui-toolbar-side-panels = Outliner & Inspector
    .description = Shows or hides the two side panels: the list of parts on the left and
        the details on the right.
ui-toolbar-help = Help
    .description = Opens the manual. You can also press F1 to open the page for whatever
        your mouse is pointing at.

## Camera

ui-toolbar-perspective = Perspective camera
    .description = You are looking through a camera that works like your eyes, where far
        things look smaller. Click to switch to a flat, technical-drawing style view,
        which is handy for checking whether two edges are really parallel.
ui-toolbar-orthographic = Orthographic camera
    .description = You are looking through a flat, technical-drawing style camera, where
        size does not change with distance. Click to switch back to the natural view.

## UV workspace

ui-toolbar-uv-wire = UV Wire
    .description = Shows only the outlines of the texture layout.
ui-toolbar-uv-shaded = UV Shaded
    .description = Fills in each piece of the texture layout as a solid shape.
ui-toolbar-uv-islands = UV Islands
    .description = Gives each separate piece of the layout its own color, so pieces that
        overlap or got lost stand out right away.

## Suffixes appended to a tool's tooltip

# Appended to a tool whose right-click opens an options panel.
ui-toolbar-has-options = Right-click for options.
# Appended to a tool whose left-click cycles through several values.
ui-toolbar-cycles = Click again to cycle.
