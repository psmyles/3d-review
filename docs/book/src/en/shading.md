# Shading and review modes

**Shading mode** is a three-way choice, exactly one active at a time:

- **Wireframe only** - edges alone.
- **Unlit** - flat surface color, no lighting.
- **Shaded** - environment-lit PBR, the default.

Two independent toggles sit either side of it and combine with all three:

- **Show Wireframe** - the edge overlay drawn on top of the filled surface. It is
  a real depth-tested line draw in the scene pass, so edges on hidden faces are
  correctly hidden and the scene's antialiasing smooths them. See
  [Wireframe](panels/wireframe.md).
- **Backface Rendering** - off culls back faces, which is the default and how you
  find inverted normals; on draws the mesh double-sided.

## Active material

A second choice, deciding what the filled faces show. It applies in every shading
mode. See [Material Mode](panels/material-mode.md) for the options panel.

| Mode | What it shows |
| --- | --- |
| **Source Material** | The imported materials plus any Inspector edits. Clicking the button again cycles Source, Standard, Unique: a plain matte mid-grey for every part, or a different random hue per mesh part so the pieces read apart. The replacement happens in the renderer, so the imported materials are never touched. |
| **UV Checker** | A greyscale or color checker at an adjustable tiling density, on any UV channel of a multi-set model. The fast read on stretching, mirroring and texel density. See [UV Checker](panels/uv-checker.md). |
| **Vertex Colors** | The mesh's vertex-color attribute as RGB, alpha as greyscale, or RGB with alpha driving opacity. See [Vertex Colors](panels/vertex-colors.md). |
| **Buffers** | One shading input at a time, drawn flat. See below. |
| **Skin Weights** | A blue-green-red heat map of how strongly the bones selected in the Outliner pull on each vertex. Flat, unlit and untonemapped, so the color you see *is* the weight. Offered only for a skinned model. |

## Buffer inspection

The **Buffers** view draws a single material or geometry input straight to the
screen, skipping lighting and tone mapping so the pixel you see is the value
itself. Clicking the toolbar button again cycles through them; the
[options panel](panels/buffers.md) picks one directly.

Base Color, Normal (World), Normal Map (Tangent), Geometric Normal, Tangent,
Roughness (or Smoothness, following the material's workflow), Metallic, Ambient
Occlusion, Emission, Opacity, UV.

Both the final shading normal and the raw authored normal map are offered, next
to the geometric normal and the tangent basis. That is what lets a misbehaving
normal map be pinned down: handedness, green-channel convention, or missing
tangents. Color buffers are sRGB-encoded for display; the rest are written raw,
so a 0.5 scalar reads as mid-grey.
