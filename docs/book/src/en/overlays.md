# Debug overlays

Independent toggles, each with its own options panel. Every extra buffer an
overlay needs is built when the overlay turns on and freed when it turns off, so
the plain shaded view carries none of them.

- **[Face normals](panels/face-normals.md)** - one line per face, adjustable
  length and color.
- **[Vertex normals](panels/vertex-normals.md)** - one line per vertex,
  adjustable length and color.
- **[Bounding box](panels/bounding-box.md)** - with live dimension labels that
  are correctly hidden behind the mesh. Scope is selectable: all meshes, only the
  selection, or only the visible meshes.
- **[UV seams](panels/uv-seams.md)** - every edge across which the chosen UV set
  is cut.
- **Pivot marker** - the object's origin.
- **[Skeleton](panels/skeleton.md)** - octahedral bones with joint markers for a
  rigged model.
- **Axis gizmo** and **grid**, both in the toolbar's display group.

Every overlay built from the mesh deforms with it, so overlays stay attached to a
skinned, animated model instead of sitting on the bind pose.

## Opening an overlay's options

Right-click any tool button to open that tool's options panel. Panels are native
windows that can be collapsed and closed, and several can be open at once. Each
one carries a `?` that opens its page in this manual.
