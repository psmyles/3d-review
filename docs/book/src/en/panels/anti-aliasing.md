# Anti Aliasing

Edges drawn on a screen can look jagged, like little stair steps. Anti-aliasing
smooths them out on the model and everything filled in the view. Lines are the
exception - the [wireframe](wireframe.md), the grid and the overlays smooth
their own edges, so they look the same at every setting, and turning
anti-aliasing up costs nothing extra for them however many there are.

## Settings

**Samples** - Off, 2x, 4x, 8x or 16x. Higher is smoother but asks more of your
GPU.

Only the levels your GPU can actually draw are listed. A Mac with
Apple silicon usually shows up to 4x; a Windows GPU usually shows all
of them.

## Notes

This is true multi-sampling of the model as it is drawn, not a blur applied
afterwards, so it smooths the edges of the shape without softening the texture
detail.

[Ambient occlusion](ambient-occlusion.md) is worked out separately, so changing
this setting does not disturb it or make it start over.
