# Diagnostic views

Some problems are about a whole surface rather than particular faces. The
diagnostic views paint the model in a colour map instead, without the
severity tint the Issues view draws. Pick one from the view group at the
end of the toolbar's left-hand tools, or click the finding it belongs to; a
key at the top of the viewport shows what the colours mean.

The view group and the material buttons before it are one choice, because
only one of them can paint the model at a time. **Issues** shows the model in
plain grey with the finding you picked tinted on it; a material button, such
as **Source Material**, shows that material with no finding drawn. Picking a
finding in the Issues list switches to the view that shows it best. Letting go
of the finding leaves the view as it is.

- **Texel density** colours each face by its texture pixels per meter, for
  the texture size in the profile: blue below the target, green on it, red
  above it.
- **Triangle density** colours each face by how its size compares with the
  smallest triangle the profile allows, at the object's own size: dense
  areas such as bolts and bevels light up.
- **Overdraw** counts how many surfaces cover each pixel from where you are
  looking. Overlapping layers, such as foliage cards, cost a GPU time at every
  layer.
- **Quad overdraw** shows how much shading is wasted on small or thin
  triangles. A GPU shades pixels in groups of four, so a triangle that covers
  only one pixel of a group still pays for all four.

Both overdraw views replace the shaded model with the count, from blue at 1
to red at the top of the key (8 surfaces, or 4 shadings for one pixel) and
beyond. The triangle checks also offer one of them under **See also** in the
Inspector.

The two overdraw views need a graphics card that can blend a floating-point
target. On one that cannot, their buttons are greyed out.

Findings about UVs split the viewport: the model on the left and its UV
layout on the right, with the problem drawn in both.

The Opt workspace has the same views, without **Issues**, so you can compare a
simplified model with the original. See
[Comparing the result](../opt/comparison.md#diagnostic-views).
