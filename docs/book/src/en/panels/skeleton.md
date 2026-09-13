# Skeleton

Draws the bones inside a rigged model, as pointed shapes with a marker at each
joint.

## Settings

**Size** - makes the drawn bones thicker or thinner. The starting size already
matches the model, so this is just a nudge.

**Color** - the color of the bones. A bone you have selected in the Outliner is
drawn in the highlight color instead, no matter what you set here, so you can
always find it.

## What it is for

Seeing the bones against the surface is how you check that the skeleton fits
the model: a bone that stops short of the end of a limb, a chain of bones that
does not follow the way the surface bends, or a stray bone left sitting at the
center of the world.

Selecting bones in the Outliner also drives the **Skin Weights** heat map,
which is the other half of checking a rig. See [Material Mode](material-mode.md).

## Notes

The bones follow the animation as it plays. In the Opt workspace the model is
shown in its bind pose on purpose, and the bones follow that.
