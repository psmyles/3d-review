# Rendering quality

The renderer works in linear HDR with reversed-Z depth, drawing into offscreen
buffers that a fullscreen composite pass resolves. The status-bar group on the
right controls it: left-click toggles or cycles, right-click opens the options.

## Image-based lighting

Six baked HDR environments, each with a preview thumbnail in the dropdown.
Optional skybox background, an intensity multiplier, and a live 0-360 degree
environment rotation that is applied while sampling and never rebuilds the maps.
The lighting maps are baked ahead of time and ship block-compressed, so startup
does no precompute. See [Environment](panels/environment.md).

## Ambient occlusion

Horizon-based occlusion over its own normal and depth buffer, with a prefiltered
depth chain for distant samples and an edge-aware denoise. The radius follows
what the viewport is showing rather than how big the file is, so it looks the
same on a 10 cm prop and a 10 km landscape.

Whenever the view is still it keeps averaging frames and settles to a clean
result in about half a second, then stops drawing entirely - so an idle viewer
showing ambient occlusion costs less than one without it, not more. It darkens
only the ambient light, so direct and emissive light are never dimmed. On by
default. See [Ambient Occlusion](panels/ambient-occlusion.md).

## Tone mapping

Khronos PBR Neutral, Linear, Reinhard, ACES or AgX, applied to the linear
radiance before sRGB encoding. Turning it off is a straight pass-through. On by
default. See [Tonemapper](panels/tonemapper.md).

## Anti-aliasing

Scene MSAA at 2x, 4x, 8x or 16x. Levels the current GPU cannot render are left
out of the menu rather than offered and failing, so Apple silicon shows up to 4x
and a Windows GPU usually shows all of them. See
[Anti Aliasing](panels/anti-aliasing.md).

## Viewport background

Black, three greys, white, or a gradient. See [Background](panels/background.md).
