# Environment

The image-based lighting that the Shaded mode uses, and the optional skybox.

## Settings

**Environment** - one of six baked HDR environments, each with a preview
thumbnail in the dropdown.

**Show skybox** - draw the environment as the viewport background instead of the
flat [background fill](background.md).

**Intensity** - a multiplier on the environment's brightness.

**Rotation** - 0 to 360 degrees. Applied while sampling, so it never rebuilds the
lighting maps and the drag is immediate.

## What it is for

Rotating the environment is the quickest way to check a material under different
light without changing the material: a surface that only looks right from one
angle usually has a normal-map or roughness problem.

## Notes

The lighting maps (the environment cube, its irradiance and prefiltered
convolutions, and the shared BRDF lookup table) are baked ahead of time and ship
block-compressed, so startup does no precompute and the intensity and rotation
controls cost nothing to drag.
