# Debug overlays

An overlay is a helper drawn on top of the model, such as lines showing which
way each face points. Each one is a separate switch with its own options panel.
The viewer only builds what an overlay needs while it is switched on, and lets
it go when you switch it off, so the plain view stays light.

- **[Face normals](panels/face-normals.md)** - one line sticking out of each
  face, showing which way the face points. You can change the length and color.
- **[Vertex normals](panels/vertex-normals.md)** - one line at each point of
  the model, showing the direction used for smooth shading. Length and color
  can be changed too.
- **[Bounding box](panels/bounding-box.md)** - a box drawn tightly around the
  model with its size written on the edges. The labels hide behind the model
  when they should. You can choose whether the box wraps everything, only the
  selection, or only what is visible.
- **[UV seams](panels/uv-seams.md)** - every edge where the texture layout is
  cut.
- **Pivot marker** - the model's own center point.
- **[Skeleton](panels/skeleton.md)** - the bones of a rigged model, with
  markers at the joints.
- **Axis gizmo** and **grid**, both in the toolbar's display group.

Every overlay that is built from the model moves with it, so if the model is
animated, the overlays stay attached instead of being left behind in the
resting pose.

## Opening an overlay's options

Right-click any tool button to open that tool's options panel. Panels are small
windows that can be folded up or closed, and you can have several open at once.
Each one has a `?` that opens its page in this manual.
