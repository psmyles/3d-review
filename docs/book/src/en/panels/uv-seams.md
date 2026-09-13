# UV Seams

Marks every edge across which the chosen UV set is cut - Maya calls these texture
border edges.

## Settings

**Color** - the seam color. Seams get a color of their own rather than reusing
the wireframe's, because the two are usually on at once and the line width is
fixed at one pixel, so color is the only thing that separates them.

**UV set** - which channel's seams to show, on a multi-set model.

## What it is for

Every seam is a place where the mesh must store the vertex twice, so the seam
count is a direct part of the `GPU Verts` figure on the [stats card](../stats.md).
It is also where a texture will show a visible break if the two sides do not
match.

## Notes

A seam is an edge whose two faces disagree about the UVs, so the whole question is
which render corners are the same point. An imported mesh answers that from the
map import already built. An [Opt](../opt/index.md)-processed level has had its
vertex buffer rebuilt and carries no such map, so it welds by exact position
within each object instead - exact, not approximate, because the optimizer never
invents a position.

Where the two halves of an Opt split disagree about the seams, that is the
processing genuinely having removed a split, which is what the comparison is for.
