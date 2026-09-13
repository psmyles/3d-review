# UV Seams

To flatten a model's surface into a texture layout, it has to be cut somewhere,
just as you would cut a cardboard box to lay it flat. Those cuts are called
seams. This overlay highlights every edge the chosen texture layout is cut
along. (Maya calls them texture border edges.)

## Settings

**Color** - the color of the seam lines. Seams get their own color rather than
sharing the wireframe's, because the two are often on at the same time and
both are one pixel wide, so color is the only thing that tells them apart.

**UV set** - which layout's seams to show, if the model has more than one.

## What it is for

At every seam the GPU has to store the points twice, once for each
side, so the number of seams is a direct part of the `GPU Verts` figure on the
[stats card](../stats.md). A seam is also where a texture can show a visible
line if the two sides do not match up.

## Notes

A seam is an edge whose two faces disagree about where they sit in the texture
layout, so the real question is which points are the same point. For a model
straight from the file, the viewer already knows. For a model that has been
through the [Opt](../opt/index.md) workspace, it works it out by finding
points in exactly the same place within the same part. Exactly, not roughly:
the optimizer never invents a new position, so that is safe.

If the two halves of an Opt split view disagree about the seams, that means
the processing really did remove a split, which is just the kind of thing the
comparison view is for.
