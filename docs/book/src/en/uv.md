# The UV workspace

Every textured model has a UV layout. Imagine peeling the model's surface off
and flattening it out onto a square sheet of paper: that flat sheet is the UV
layout, and it tells the computer which part of a picture goes on which part of
the model. The pieces of the flattened surface are called islands.

The UV workspace shows that flat sheet. It has its own camera: left-drag slides
the view around, right-drag or the mouse wheel zooms, and `R` fits the whole
layout back into view.

Pick a texture in the Outliner's **Textures** tab and it is drawn across the
square, behind the layout, so you can see which part of the picture each island
covers. It stays there while you select parts to narrow the layout down, until
you pick another texture or press `Esc`. Its colors are shown as they are, and
its see-through parts let the background show through. With **Shaded** or
**Islands**, the fills are drawn faint over it, so the islands and the picture
can both be read.

The buttons at the bottom right choose the background around the layout, as in
the [Tex workspace](tex.md): **B** black, **W** white, **G** grey, or **C** a
checkerboard, which is the clearest way to see a texture's see-through parts.

The outlines of the layout are always drawn. The toolbar choice decides what
fills them:

- **Wire** - just the outlines.
- **Shaded** - each island filled in as a solid shape.
- **Islands** - each island in its own color, so ones that overlap, or ones
  that wandered off the sheet, stand out right away.

If the model has more than one UV layout, a picker in the toolbar switches
between them.

The [Outliner](outliner-inspector.md) decides which parts are laid out. With
nothing selected you see every part, and with parts selected you see only
theirs, so you can check one piece of a model without the others drawn over
it. A part hidden with its eye icon is left out either way.

## Reading a layout

The island coloring is the quickest way to spot a piece that was left outside
the square, or one that sits on top of another. To judge whether the texture
will look stretched or blurry, the [UV Checker](panels/uv-checker.md) in the
3D workspace is the better tool, because it shows the checkerboard on the
actual surface.

To see where the layout was *cut* to flatten it, the
[UV Seams](panels/uv-seams.md) overlay marks every edge it was cut along.
