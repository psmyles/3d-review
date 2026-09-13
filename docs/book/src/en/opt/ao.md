# Baking ambient occlusion

Ambient occlusion is the soft shadow that gathers in corners, creases and
anywhere light has trouble reaching. Baking it means working it out once and
storing the answer in the model, as a color on each point, so a game can show
it without doing any work.

This operation does that by firing many rays out from each point and counting
how many hit something nearby. The work is spread across all your processor's
cores, and the result is the same every time. Adding the operation switches the
view to the matching [vertex color mode](../panels/vertex-colors.md) so you
can see the result right away, and the export already saves vertex colors, so
nothing else is needed.

Unlike the [live effect](../panels/ambient-occlusion.md) in the viewport,
this is part of the model and travels with it.

## Settings

**Quality** - 32, 64, 128 or 512 rays per point. More rays are slower and
smoother.

**Max distance** - how far away a surface can be and still cast a shadow, in
meters. Zero means no limit.

**Intensity** - how dark the shadows get, working the same way as the viewport
panel.

**Target** - where to store the result: the alpha channel, all three color
channels as grey, multiplied into the existing colors, or a single red, green
or blue channel. The color channels can optionally be stored in sRGB; the
alpha channel is always stored as-is.

## Two rules that make it work on real game files

**Excluded parts still cast shadows.** A part you have
[excluded](overrides.md) from the list is not written to, but it still blocks
light. That is what you want for a hero prop that should keep its own bake.

**Hidden parts do neither.** A part hidden in the Outliner neither casts a
shadow nor receives one. So hide the collision shell first. Clicking an eye
icon reruns the list while a bake is switched on, for exactly this reason.

## Parts are grouped by their LOD name

Game files often carry every level of detail of a model as separate parts in
the same file, all sitting in the same place. If rays from one level hit the
nearly identical surface of another level, the result is a mess.

So the viewer groups parts by the `_LOD` and number at the end of their names,
taken from the part itself or from the nearest named part above it:

- each level is shadowed only by its own group, plus every part with no such
  name,
- parts with no such name are shadowed by the lowest level present.

That means you can bake a whole visible set of levels correctly in one go.
