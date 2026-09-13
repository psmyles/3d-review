# Vertex Colors

Some models carry a color on each of their points, separate from any texture.
This panel shows those colors instead of the material.

## Settings

**Mode**:

- *RGB* - the colors as colors.
- *Alpha* - the alpha (see-through) value as shades of grey.
- *RGB with alpha* - the colors, with alpha making parts see-through.

**Color set** - which set to show, if the model carries more than one.

## What it is for

Artists use vertex colors for all sorts of things: a tint, a mask that blends
between two materials, a value that says how much a plant sways in the wind,
or baked-in soft shadows. Looking at the alpha channel on its own is usually
what tells you which, because a mask is almost always kept in a single channel.

The [Opt workspace](../opt/ao.md) can bake soft shadows into a vertex color
set, and switching that operation on picks the matching mode here for you.
