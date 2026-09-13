# Tonemapper

Real light has a far wider range of brightness than a screen can show: sunlight
is thousands of times brighter than a shadow, but your monitor can only go from
black to "as bright as it gets". Tone mapping is the recipe that squeezes the
scene's full range into what the screen can display. It is on to begin with.

## Settings

**Operator** - the recipe to use:

- *Khronos PBR Neutral* - the normal choice. It leaves ordinary colors alone
  and only gently rolls off the parts that would be too bright, so material
  colors stay true.
- *Linear* - no curve at all, just a hard cut at the top. Useful for checking
  whether a bright area is genuinely too bright in the data or only looks that
  way on screen.
- *Reinhard* - the classic curve. Bright areas lose some of their color.
- *ACES* - the film-like curve much of the games industry uses. Punchy, with
  warm highlights.
- *AgX* - a modern curve with a long, gentle roll-off in the highlights.

Turning tone mapping off skips the recipe entirely and shows the raw values.

## What it is for

The recipe changes what you conclude about a material. A surface that looks
fine under ACES may be clipping to pure white under Linear. If you know which
game engine the model is headed for, picking the same recipe makes the viewer a
fairer preview of what the engine will show.
