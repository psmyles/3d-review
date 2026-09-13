# Material Mode

This panel chooses what the surface of the model shows. It works in every
shading mode, so a wireframe-over-shaded view and an unlit view both follow
it.

## Modes

**Source Material** - the materials that came with the file, plus any changes
you made in the Inspector. Clicking the toolbar button again steps through
three looks:

- *Source* - the materials as they came in, with your edits.
- *Standard* - every part painted the same plain grey, like clay. This is how
  you judge the shape and outline of a model without the texture distracting
  you.
- *Unique* - each part in its own random color, so you can tell the pieces
  apart at a glance.

The swap happens only in the viewer, so the file's materials are never touched
and switching back costs nothing.

**UV Checker** - see [UV Checker](uv-checker.md).

**Vertex Colors** - see [Vertex Colors](vertex-colors.md).

**Buffers** - one ingredient of the shading at a time, with no lighting. See
[Buffers](buffers.md).

**Skin Weights** - a heat map, from blue through green to red, of how strongly
the bones you have selected pull on each part of the model. It is drawn flat,
with no lighting and no color processing, so the color you see *is* the
number. Only offered for models with bones. Select bones in the Outliner's
Scene tab; the primary modifier adds to the selection and Shift selects a run
of bones.

See also [Materials and textures](../materials.md).
