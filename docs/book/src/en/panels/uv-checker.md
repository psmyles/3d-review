# UV Checker

Replaces the surface with a checker pattern, which is the fast read on
stretching, mirroring and texel density.

## Settings

**Pattern** - greyscale or color. The color checker makes mirrored shells obvious
because the hue sequence runs backwards on them.

**Density** - how many checker squares per UV unit. Raise it to judge texel
density on a small prop, lower it on a large one.

**UV set** - which channel to apply it to, on a model that carries more than one.
A second set is usually a lightmap or a detail-texture layout, and checking it is
how you find out which.

## Reading the result

- Squares that are *rectangles* mean the UVs are stretched on that axis.
- Squares of visibly different sizes across parts mean inconsistent texel
  density; the parts with smaller squares get more texture resolution.
- A shell where the color sequence runs the other way is mirrored.

For the layout itself rather than its effect on the surface, use the
[UV workspace](../uv.md); for where the layout is cut, [UV Seams](uv-seams.md).
