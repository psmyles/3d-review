# Tonemapper

The operator applied to the linear HDR radiance before it is encoded to sRGB for
display. On by default.

## Settings

**Operator**:

- *Khronos PBR Neutral* - the default. Designed to leave in-range colors alone
  and roll off only what would clip, so material colors stay faithful.
- *Linear* - a straight clamp. Useful for checking whether a bright area is
  genuinely blown out in the data or only in the display.
- *Reinhard* - the classic curve; desaturates highlights noticeably.
- *ACES* - the film-style curve much of the industry uses; contrasty, with warm
  highlights.
- *AgX* - a modern curve with a long, well-behaved highlight roll-off.

Turning tone mapping off is a straight pass-through with no operator at all.

## What it is for

For an audit, the operator matters because it changes what you conclude about a
material. A surface that looks correct under ACES may be clipping under Linear.
Matching the operator to the engine the asset is destined for makes the viewport
a fairer preview.
