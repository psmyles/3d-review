# Wireframe

The wireframe is the set of lines that make up the model, drawn on top of the
filled surface. Turn it on and off from the toolbar or with the `` ` `` key.
Right-click the toolbar button to open this panel.

The lines are drawn as part of the model itself, not painted on afterwards.
That has two nice results: lines on the far side of the model are hidden by
the surface in front of them, just as they should be, and the edge smoothing
from [Anti Aliasing](anti-aliasing.md) applies to them too. The one thing you
give up is that the lines are always one pixel thick.

## Settings

**Color** - the color of the lines. Changing it is instant, even on a very big
model, so feel free to drag the color around until it stands out well against
your surface.

## Notes

Most overlays are thrown away the moment you switch them off. The wireframe
is the one exception: the viewer keeps its edge list, because people flip the
wireframe on and off constantly, and rebuilding it costs the most on exactly
the big models where you do that. It is stored compactly, so a model of a
quarter of a million triangles keeps about 6 MB for it.

See also [Shading and review modes](../shading.md).
