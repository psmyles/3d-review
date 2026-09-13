# Wireframe

Controls the edge overlay drawn on top of the filled surface. Toggle it from the
toolbar or with the `` ` `` key; right-click the toolbar button for this panel.

The wireframe is a real depth-tested line draw inside the scene pass, not a
post-process. Two things follow from that: edges on faces pointing away from you
are correctly hidden by the mesh in front of them, and the scene's
[antialiasing](anti-aliasing.md) smooths them along with everything else. The
trade-off is that the line width is fixed at one hardware pixel.

## Settings

**Color** - the edge color. Changing it rebuilds nothing: the color is a shader
uniform, so the drag is immediate even on a very heavy mesh.

## Notes

The edge list is the one derived view the viewer keeps after you switch it off,
because toggling the wireframe is a per-second gesture on exactly the models
where rebuilding it costs most. It is stored as an index buffer over the mesh's
own vertices, so a 250k-triangle asset holds about 6 MB for it.

See also [Shading and review modes](../shading.md).
