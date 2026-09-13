# Per-object overrides

Sometimes one part of a model should be treated differently from the rest.
That is what a per-object override is for.

Any part can be **excluded** from the list. It then passes through untouched,
at full detail, in every output level. That is how you keep a hero prop, or a
collision shell, out of the optimization.

An excluded part still casts shadows during an [AO bake](ao.md); it just does
not get written to.

You can also give a single part its own settings for one operation. Any
operation you do not override uses the shared settings, so an override is a
small exception rather than a whole second setup to look after.

## Where to set them

Select the part in the Outliner, then look at the Inspector. While the Opt
workspace is open, the Inspector shows that part's overrides.

## Notes

An override is tied to the operation it belongs to. Reordering the list keeps
it attached to the right operation, and so does saving and loading a preset.

Overrides are part of the list, so they are saved in a [preset](export.md)
and covered by undo.
