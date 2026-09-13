# Buffers

Draws a single material or geometry input straight to the screen, skipping
lighting and tone mapping so the pixel you see is the value itself.

Clicking the toolbar button again cycles through the buffers; this panel picks
one directly.

## What each buffer shows

| Buffer | What it is |
| --- | --- |
| Base Color | The albedo the material feeds the shader, after any texture and tint. |
| Normal (World) | The final shading normal, after the normal map. |
| Normal Map (Tangent) | The raw authored normal map, before it is applied. |
| Geometric Normal | The interpolated mesh normal, with no map applied. |
| Tangent | The tangent basis the normal map is applied through. |
| Roughness | The roughness the shader uses; labelled Smoothness on a material set to that workflow. |
| Metallic | The metalness value. |
| Ambient Occlusion | The material's own AO map, not the screen-space effect. |
| Emission | The emissive contribution. |
| Opacity | The alpha the transparency mode reads. |
| UV | The UV coordinates as color, so wrapping and mirroring are visible. |

## Diagnosing a normal map

Offering the final shading normal, the raw map, the geometric normal and the
tangent side by side is what lets a misbehaving normal map be pinned down.

- If Geometric Normal looks right and Normal (World) does not, the map or the
  tangents are the problem.
- If Normal Map (Tangent) is mostly flat blue where you expect detail, the wrong
  file or the wrong channel is bound.
- If the lighting looks inverted along one axis only, it is the green-channel
  convention (DirectX against OpenGL).
- If Tangent is black or noisy, the mesh arrived without usable tangents.

Color buffers are sRGB-encoded for display; the rest are written raw, so a 0.5
scalar reads as mid-grey.
