# UV Checker

Paints a checkerboard over the model instead of its material. This is the
quickest way to see whether a texture would look stretched, mirrored, or
blurrier in some places than in others.

## Settings

**Pattern** - a grey checkerboard, or a colored one. The colored one makes a
mirrored part easy to spot, because its colors run in the opposite order.

**Scale** - how the squares are sized:

- **Per texture** - **Checker Tiling** repeats the checker that many times
  across the texture. Use more on a small object and fewer on a big one, so
  the squares stay a useful size.
- **By size** - every square is **Square size** across on the model, in
  metres, for a layout made at **UV density** (UV units per metre; 1 for a
  texture that tiles once per metre). Anywhere the squares come out bigger or
  smaller than that, the layout's density is off; anywhere they are not square,
  it is stretched. For one part against another, or one level of detail against
  the next, the squares should match.

**UV set** - which texture layout to use, if the model has more than one. A
second layout is often used for baked lighting or for fine detail, and
looking at it here is how you find out which.

## Reading the result

- Squares that come out as *rectangles* mean the texture is stretched in that
  direction.
- Squares that are clearly bigger on some parts than others mean the texture
  is spread unevenly; the parts with smaller squares get more of the picture's
  detail.
- A part where the colors run the other way round is mirrored.

To look at the flat layout itself rather than how it lands on the surface,
use the [UV workspace](../uv.md). To see where the layout is cut, use
[UV Seams](uv-seams.md).
