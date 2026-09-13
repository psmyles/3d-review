# Material Mode

Chooses what the filled faces show. It applies in every shading mode, so a
wireframe-over-shaded view and an unlit view both follow it.

## Modes

**Source Material** - the imported materials plus any Inspector edits. Clicking
the toolbar button again cycles three variants:

- *Source* - the materials as imported and edited.
- *Standard* - a plain matte mid-grey for every part, which is how you judge
  silhouette and form without texture detail arguing with it.
- *Unique* - a different random hue per mesh part, so the pieces read apart.

The replacement happens in the renderer, so the imported materials are never
touched and switching back is free.

**UV Checker** - see [UV Checker](uv-checker.md).

**Vertex Colors** - see [Vertex Colors](vertex-colors.md).

**Buffers** - one shading input at a time, drawn flat. See [Buffers](buffers.md).

**Skin Weights** - a blue-green-red heat map of how strongly the bones selected in
the Outliner pull on each vertex. Flat, unlit and untonemapped, so the color you
see *is* the weight. Offered only for a skinned model. Select bones in the
Outliner's Scene tab; the primary modifier adds to the selection and Shift takes
a range.

See also [Materials and textures](../materials.md).
