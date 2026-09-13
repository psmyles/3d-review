# Skeleton

Draws the bones inside a rigged model, as pointed shapes with a marker at each
joint.

## Settings

**Size** - makes the drawn bones thicker or thinner. The starting size already
matches the model, so this is just a nudge.

**Color** - the color of the bones. A bone you have selected is drawn in the
highlight color instead, no matter what you set here, so you can always find it,
and the bone under the pointer is drawn in a paler one while you are picking.

## What it is for

Seeing the bones against the surface is how you check that the skeleton fits
the model: a bone that stops short of the end of a limb, a chain of bones that
does not follow the way the surface bends, or a stray bone left sitting at the
center of the world.

While this overlay is on, [clicking in the viewport](../selection.md) picks
**bones** rather than parts of the mesh - which is the point of having the
skeleton drawn at all. Turn the overlay off and clicking goes back to the mesh.

Selecting bones also drives the **Skin Weights** heat map, which is the other
half of checking a rig. See [Material Mode](material-mode.md).

## Notes

The bones follow the animation as it plays. In the Opt workspace the model is
shown in its bind pose on purpose, and the bones follow that.
