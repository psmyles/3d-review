# Bounding Box

Draws a box that just fits around the model, with its width, height and depth
written along the edges. The labels hide behind the model when it is in front
of them, so they stay easy to read from any angle.

## Settings

**Scope** - which parts the box wraps around:

- *All Meshes* - everything in the file.
- *Only Selection* - just what you have selected in the Outliner.
- *Only Visible* - everything that is still showing.

**Color** - the color of the box's lines.

## Notes

The sizes are in meters. The viewer converts every file to meters no matter
what unit the file says it uses. The [stats card](../stats.md) tells you what
the file claimed, which is where a model that comes in a hundred times too big
or too small usually gets its problem.

While an animation is selected, the box wraps the whole area the animation
moves through, not just the current pose, to match what `F` frames. See
[Skinning and animation](../animation.md).

In the Opt workspace's split view each half gets its own box, measured on the
model that half is showing.
