# Skeleton

Draws octahedral bones with joint markers for a rigged model.

## Settings

**Size** - a multiplier on the drawn bone thickness. The base size follows the
model, so this is a correction rather than an absolute.

**Color** - the bone color. The bone selected in the Outliner is drawn in the
viewport's selection color instead, whatever this is set to.

## What it is for

Seeing the joint chain against the mesh is how you check that the rig matches the
silhouette: bones ending short of a limb, a chain that does not follow the
deformation, or a stray joint left at the origin.

Selecting bones in the Outliner also drives the **Skin Weights** heat map, which
is the other half of a rig check. See [Material Mode](material-mode.md).

## Notes

The overlay attaches to the animated node transforms, so it follows a playing
clip. In the Opt workspace the bind pose is drawn on purpose.
