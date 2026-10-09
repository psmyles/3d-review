# Wireframe

The wireframe is the set of lines that make up the model, drawn on top of the
filled surface. Turn it on and off from the toolbar or with the `` ` `` key.
Right-click the toolbar button to open this panel.

The lines are drawn as part of the model itself, not painted on afterwards, so
lines on the far side of the model are hidden by the surface in front of them,
just as they should be. Each line smooths its own edges, so the wireframe looks
the same whatever [Anti Aliasing](anti-aliasing.md) is set to, and the same on
Windows as on a Mac.

## Settings

**Line width** - how thick the lines are. The width is measured on the screen,
not in the model, so the lines keep their weight however close you zoom in.
It also allows for the display: a line is twice as many pixels wide on a
high-resolution (Retina) screen, which is what makes it the same thickness to
your eye as on an ordinary one. Below 1 the lines get fainter rather than
thinner.

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
