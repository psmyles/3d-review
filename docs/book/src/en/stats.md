# Model statistics

A panel of measured counts that can be toggled on and off. Every figure is a real
measured value: there are no placeholder rows, and the counts the DCC package
recorded are reported rather than post-triangulation render counts.

Draws, Polys, Tris, Verts, GPU Verts, Vtx Splits, UV Sets, Bones, Clips, Unit,
FPS.

Hovering a row explains what it measures. Clicking one copies it to the
clipboard.

## Verts against GPU Verts

`Verts` is the file's own vertex count - the number your DCC's stats show. It
ignores the extra vertices that hard edges and UV seams force the GPU to store.

`GPU Verts` is the engine cost: unique vertices per draw group, one per distinct
combination of position, normal, UVs and color.

`Vtx Splits` is the gap between them, as a percentage. This is the figure an
audit exists to surface. A healthy game asset reads a few percent; a scan with
per-face normals reads several hundred.

## Scopes

The three columns are the whole file, the current selection, and whatever is
still visible. A row with nothing to say for a scope shows a hyphen, never a
zero.

The [Opt workspace](opt/comparison.md) adds a second card with the processed
mesh's counts and the GPU-behaviour figures beside them.
