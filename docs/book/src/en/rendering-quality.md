# Rendering quality

These are the settings that decide how nice the picture looks: the lighting,
the soft shadows, how bright colors are handled, and how jagged edges are
smoothed. They live in the group of buttons on the right of the status bar.
Left-click a button to turn it on or off, or to step through its choices.
Right-click it to open its options.

## Image-based lighting

The model is lit by a photo of a real place wrapped all around it, which is
what gives it natural-looking soft light and reflections. There are six of
these environments to choose from, each with a small preview picture in the
dropdown. You can show the environment behind the model as a backdrop, make it
brighter or dimmer, and spin it around the model. Spinning is instant, so it is
a nice way to see the model lit from different sides. See
[Environment](panels/environment.md).

## Ambient occlusion

Ambient occlusion adds the soft shadows you see in corners, creases and
anywhere light has trouble reaching. It makes a model look grounded and solid.

The size of the effect follows what you are looking at rather than how big the
file is, so it looks right on a tiny prop and on a huge landscape alike.

While the view is still, the viewer keeps refining the shadows and settles on a
clean result in about half a second, then stops working on them entirely. So a
viewer sitting idle with this on costs less than one without it, not more. It
only darkens the soft surrounding light, never direct light or glowing parts.
It is on to begin with. See [Ambient Occlusion](panels/ambient-occlusion.md).

## Tone mapping

Real light has a far wider range of brightness than a screen can show. Tone
mapping is the recipe that squeezes that range into what your screen can
display. You can choose between several recipes: Khronos PBR Neutral, Linear,
Reinhard, ACES and AgX. Turning it off shows the raw values. It is on to begin
with. See [Tonemapper](panels/tonemapper.md).

## Anti-aliasing

Anti-aliasing smooths the jagged, stair-step look of edges. You can pick 2x,
4x, 8x or 16x; higher is smoother but costs more. Only the levels your
graphics card can handle are listed, so a Mac with Apple silicon shows up to 4x
and a Windows PC usually shows all of them. See
[Anti Aliasing](panels/anti-aliasing.md).

## Viewport background

Black, three shades of grey, white, or a soft gradient. See
[Background](panels/background.md).
