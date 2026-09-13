# Environment

The Shaded view lights the model with a photo of a real place wrapped all
around it. That is what gives it natural soft light and reflections. This panel
chooses that place and how it is used.

## Settings

**Environment** - one of six ready-made environments, each with a small preview
picture in the dropdown.

**Show skybox** - draw the environment behind the model, instead of the plain
[background fill](background.md).

**Intensity** - how bright the light is.

**Rotation** - spins the environment around the model, from 0 to 360 degrees.
Dragging it is instant.

## What it is for

Spinning the environment is the quickest way to see a material under different
light without changing the material. A surface that only looks right from one
angle usually has a problem with its normal map or its roughness.

## Notes

All the lighting information for each environment is worked out ahead of time
and shipped with the viewer in a compact form, so nothing has to be computed
when the viewer starts, and the intensity and rotation sliders cost nothing to
drag.
