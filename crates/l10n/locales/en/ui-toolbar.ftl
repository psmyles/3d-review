### The toolbar's tool buttons. Each is icon-only, so its tooltip is the only
### place its name appears - which is why every one of these carries a
### `.description` paragraph as well.
###
### "Right-click for options" is deliberately *not* repeated in each message: the
### `Tip` builder appends it once, so it is translated once.

## Shading modes

ui-toolbar-wireframe-only = Wireframe Only
    .description = Edges alone, with no filled surface.
ui-toolbar-unlit = Unlit
    .description = Flat surface colour with no lighting - the read on albedo and vertex
        colours without shading arguing with them.
ui-toolbar-shaded = Shaded
    .description = Environment-lit PBR. The default, and the closest preview of what an
        engine will show.

ui-toolbar-show-wireframe = Show Wireframe
    .description = Draw the edges over the filled surface. A real depth-tested line draw,
        so edges on hidden faces are correctly hidden.
ui-toolbar-backface-rendering = Backface Rendering
    .description = Off culls back faces, which is how you find inverted normals. On draws
        the mesh double-sided.

## Active material

ui-toolbar-source-material = Source Material
    .description = The imported materials plus any Inspector edits. Click again to cycle
        Source, Standard and Unique.
ui-toolbar-uv-checker = UV Checker
    .description = A checker pattern on the surface: the fast read on stretching,
        mirroring and texel density.
ui-toolbar-vertex-colors = Vertex Colors
    .description = The mesh's vertex-color attribute, as colour, as alpha, or as both.
ui-toolbar-buffers = Buffers
    .description = One shading input at a time, drawn flat with no lighting or tone
        mapping. Click again to cycle through them.
ui-toolbar-skin-weights = Skin Weights
    .description = A heat map of how strongly the bones selected in the Outliner pull on
        each vertex. Select bones in the Outliner's Scene tab first.

## Debug overlays

ui-toolbar-bounding-box = Bounding Box
    .description = The axis-aligned box with live dimension labels, correctly hidden
        behind the mesh.
ui-toolbar-face-normals = Face Normals
    .description = One line per face. The fastest way to find flipped faces.
ui-toolbar-vertex-normals = Vertex Normals
    .description = One line per vertex, along its shading normal - which is what smoothing
        actually uses.
ui-toolbar-uv-seams = UV Seams
    .description = Every edge the chosen UV set is cut across. Each one costs a duplicated
        vertex on the GPU.
ui-toolbar-skeleton = Skeleton
    .description = Octahedral bones with joint markers, following the animated pose.

## Display

ui-toolbar-grid = Grid
    .description = The floor grid with its axis lines.
ui-toolbar-axis-gizmo = Axis Gizmo
    .description = The orientation gizmo. Click one of its axes to snap the camera to it.
ui-toolbar-pivot = Pivot
    .description = A marker at the object's own origin.
ui-toolbar-side-panels = Outliner & Inspector
    .description = Show or hide the two side panels.
ui-toolbar-help = Help
    .description = Open the manual. F1 opens the page for whatever the pointer is over.

## Camera

ui-toolbar-perspective = Perspective camera
    .description = Click for an orthographic camera, which is what shows whether two edges
        are really parallel.
ui-toolbar-orthographic = Orthographic camera
    .description = Click for a perspective camera.

## UV workspace

ui-toolbar-uv-wire = UV Wire
    .description = The layout's edges alone.
ui-toolbar-uv-shaded = UV Shaded
    .description = Solid islands.
ui-toolbar-uv-islands = UV Islands
    .description = A different colour per island, so overlaps and stray shells stand out.

## Suffixes appended to a tool's tooltip

# Appended to a tool whose right-click opens an options panel.
ui-toolbar-has-options = Right-click for options.
# Appended to a tool whose left-click cycles through several values.
ui-toolbar-cycles = Click again to cycle.
