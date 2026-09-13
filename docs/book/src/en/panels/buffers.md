# Buffers

A shaded pixel is made from several ingredients, such as the base color, the
normal map and the roughness. The Buffers view shows one of those ingredients
on its own, with no lighting and no color processing, so the value you see on
screen is the raw value.

Clicking the toolbar button again moves to the next ingredient. This panel
lets you pick one directly.

## What each buffer shows

- **Base Color** - the plain surface color, after any texture and tint.
- **Normal (World)** - the final direction the surface faces at each pixel,
  after the normal map has been applied. This is what the lighting uses.
- **Normal Map (Tangent)** - the raw normal map picture, exactly as painted,
  before it is applied.
- **Geometric Normal** - the direction the model's own surface faces, with no
  normal map at all.
- **Tangent** - the helper directions the normal map is applied through.
- **Roughness** - how rough the surface is; shown as Smoothness on a material
  that uses that instead.
- **Metallic** - how metal-like the surface is.
- **Ambient Occlusion** - the material's own baked shadow map, not the live
  effect in the viewport.
- **Emission** - how much the surface glows.
- **Opacity** - how see-through the surface is, which the transparency mode
  reads.
- **UV** - the texture layout coordinates shown as color, so you can see
  wrapping and mirroring.

## Tracking down a bad normal map

A normal map is a picture that fakes small bumps and dents in a surface. When
one looks wrong, seeing the final normal, the raw map, the geometric normal
and the tangent side by side is what lets you find out why:

- If Geometric Normal looks right but Normal (World) does not, the map or the
  tangents are the problem.
- If Normal Map (Tangent) is mostly a flat blue where you expect detail, the
  wrong file or the wrong channel is plugged in.
- If the lighting looks inside-out along one direction only, the green channel
  is the other way round (DirectX style against OpenGL style).
- If Tangent is black or noisy, the model came in without usable tangents.

Colors are shown the way a picture would be; plain numbers are shown as-is,
so a value of 0.5 comes out as mid-grey.
