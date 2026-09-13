# The UV workspace

A separate 2D viewport for the UV layout, with its own pan, zoom and fit. The UV
edges draw in every mode; the shading choice picks what fills them.

- **Wire** - the layout alone.
- **Shaded** - solid islands.
- **Islands** - a different color per island, so overlaps and stray shells stand
  out.

A UV-set picker in the toolbar switches channels on a multi-set model.

Left-drag pans, right-drag or the wheel zooms, and `R` refits the layout.

## Reading a layout

The island coloring is the fastest way to spot a shell that was left outside the
0-1 square or one that overlaps another. For texel density and stretching, the
[UV Checker](panels/uv-checker.md) material mode in the 3D viewport is the better
read, because it shows the checker on the surface it is actually applied to.

For where the layout is *cut*, the [UV Seams](panels/uv-seams.md) overlay marks
every edge the chosen UV set breaks across.
