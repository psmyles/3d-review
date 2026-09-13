# Shading and review modes

Shading is how the surface of the model is drawn. There are three shading
modes, and exactly one is active at a time:

- **Wireframe only** - just the lines that make up the model, nothing filled
  in. Good for seeing how the model is built.
- **Unlit** - the surface colors with no lighting at all, so nothing is in
  shadow and nothing shines. Good for seeing the plain painted colors.
- **Shaded** - the model lit realistically, the way a game would draw it. This
  is the normal view.

Two extra switches sit either side of those and work with all three:

- **Show Wireframe** draws the model's edges on top of the filled surface.
  Edges that are hidden behind the model stay hidden, and the edge smoothing
  (see [Anti Aliasing](panels/anti-aliasing.md)) applies to them too. See
  [Wireframe](panels/wireframe.md).
- **Backface Rendering.** Every face has a front and a back. With this off
  (the normal setting) the backs are not drawn, which is a quick way to spot a
  face that is pointing inward by mistake, because it disappears. With it on,
  both sides are drawn.

## Active material

A second choice decides what the surface *shows*. It works in every shading
mode. See [Material Mode](panels/material-mode.md) for its options panel.

| Mode | What it shows |
| --- | --- |
| **Source Material** | The materials that came with the file, plus any changes you made in the Inspector. Clicking the button again steps through Source, Standard and Unique: Standard paints every part the same plain grey, like clay, and Unique gives each part its own random color so you can tell the pieces apart. The file itself is never changed. |
| **UV Checker** | A checkerboard painted over the model, in grey or in color, with a choice of how big the squares are. This is the quickest way to spot stretched, mirrored or uneven texturing. See [UV Checker](panels/uv-checker.md). |
| **Vertex Colors** | The colors stored on the model's own points, rather than a material. You can see the color, the alpha (see-through) value as grey, or both together. See [Vertex Colors](panels/vertex-colors.md). |
| **Buffers** | One ingredient of the shading at a time, with no lighting. See below. |
| **Skin Weights** | A heat map, from blue through green to red, of how strongly the bones you have selected pull on each part of the model. Drawn flat with no lighting, so the color you see *is* the number. Only offered for models that have bones. |

## Buffer inspection

A shaded pixel is made from several ingredients: the base color, the normal
map, the roughness, and so on. The **Buffers** view shows one of those
ingredients on its own, with no lighting and no color processing, so the value
you see on screen is the raw value. Clicking the toolbar button again moves to
the next ingredient; the [options panel](panels/buffers.md) lets you pick one
directly.

The list is: Base Color, Normal (World), Normal Map (Tangent), Geometric
Normal, Tangent, Roughness (shown as Smoothness if the material uses that),
Metallic, Ambient Occlusion, Emission, Opacity, UV.

Having the final normal, the raw normal map, the plain geometric normal and the
tangent all available side by side is what lets you track down a normal map
that looks wrong: whether it is flipped, whether its green channel is the
other way round, or whether the model is missing tangents. Colors are shown the
way a picture would be; plain numbers are shown as-is, so a value of 0.5 comes
out as mid-grey.
