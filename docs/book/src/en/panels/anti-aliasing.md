# Anti Aliasing

Scene multisampling, applied to the whole 3D viewport including the wireframe and
the line overlays.

## Settings

**Samples** - Off, 2x, 4x, 8x or 16x.

Levels the current GPU cannot render are left out of the menu rather than offered
and failing. Apple silicon typically shows up to 4x; a Windows GPU usually shows
all of them.

## Notes

This is real MSAA in the scene pass, not a post-process, so it antialiases
geometry edges without softening texture detail.

The [ambient occlusion](ambient-occlusion.md) pass deliberately runs on its own
single-sample buffer, so changing this setting does not disturb it or reset its
accumulation.
