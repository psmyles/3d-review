# Vertex Colors

Some models carry a color on each of their points, separate from any texture.
This panel shows those colors instead of the material.

## Settings

**Mode**:

- *RGB* - the colors as colors.
- *Alpha* - the alpha (see-through) value as shades of grey.
- *RGB with alpha* - the colors, with alpha making parts see-through.
- *Red channel*, *Green channel*, *Blue channel* - that one channel alone, as
  shades of grey. A bake written into one channel, such as
  [edge wear](../opt/wear.md), reads clearly here, where in RGB it would only
  tint the colors.
- *Palette index* - one channel read as an index into a palette, and each
  part shown in that palette entry's color: what a game that colors its models
  from a palette would draw. See [below](#palette-index).

**Color set** - which set to show, if the model carries more than one.

## What it is for

Artists use vertex colors for all sorts of things: a tint, a mask that blends
between two materials, a value that says how much a plant sways in the wind,
or baked-in soft shadows. Looking at the alpha channel on its own is usually
what tells you which, because a mask is almost always kept in a single channel.

The [Opt workspace](../opt/ao.md) can bake soft shadows into a vertex color
set, and switching that operation on picks the matching mode here for you.

## Palette index

A game can color a model from a palette of its own, storing in each point a
number that says which palette entry to use: entry `n` stored as `n / 255`, so
entry 3 is `3/255`. The *Palette index* mode shows those entries' colors, and
three more settings appear:

**Index channel** - the channel that holds the index: red, green, blue or
alpha.

**Painted in sRGB** - tick it when the index was painted as an sRGB color and
stored linear. Blender's byte color attributes export that way: index 1
arrives as about 0.0003 rather than 1/255, and only reads as 1 back through the
sRGB curve. When most of the model shows magenta, this is usually why.

**Palette** - the colors the indices look up. **Load...** reads one from a file:

- a JSON list of colors, or an object holding one under `colors` or `palette`.
  Each color is a `"#rrggbb"` string, an `[r, g, b]` list or an `{"r", "g", "b"}`
  object. Numbers are 0 to 255 if any of them is above 1, and 0 to 1 otherwise.
  Colors are sRGB, as a color picker shows them; an object can say
  `"linear": true` when they are not.
- a PNG with one pixel per entry: a strip, or a 16 x 16 grid read row by row.
  A larger image is read along its first row (or column, when it is taller
  than it is wide).

A palette holds at most 256 colors. Without one, a built-in palette gives
neighbouring indices clearly different colors (entry 0 is a dark grey).
**Use built-in** goes back to it. With **Remember Settings** on, the palette
file is read again the next time the viewer starts.

A value that is no whole index (a blend of two, say), or an index past the end
of the palette, shows **magenta**. Where the corners of one face disagree, the
face shows a blend. The audit's
[Indexed vertex color](../aud/checks.md#geometry) check finds both for you.

`review3d-cli render --look palette --index r:srgb` shows the same in a render,
with the built-in palette.
