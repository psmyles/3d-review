# Bounding Box

Draws the axis-aligned bounding box with live dimension labels along its edges.
The labels are correctly hidden when the mesh is in front of them, so they stay
readable from any angle.

## Settings

**Scope** - which meshes the box encloses:

- *All Meshes* - every mesh in the file.
- *Only Selection* - the current Outliner selection.
- *Only Visible* - everything the Outliner is still showing.

**Color** - the box's edge color.

## Notes

The dimensions are in meters. Import normalizes every file to meters regardless
of what the source declared; the [stats panel](../stats.md) reports the unit the
file itself claimed, which is where scale mismatches come from.

While an animation clip is selected the box covers that clip's whole range of
motion rather than the current frame, matching what `F` frames. See
[Skinning and animation](../animation.md).

In the Opt workspace's split layout each half gets its own box, measured on the
mesh that half is showing.
