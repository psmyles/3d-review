# Per-object overrides

Any object can be **excluded** from the stack, passing through untouched at full
detail in every output level. That is the way to keep a hero prop or a collision
shell out of the optimization.

An excluded object still blocks light during an [AO bake](ao.md); it simply is
not written to.

Individual operations can also be given replacement settings for one object.
Operations without an override use the global settings, so an override is a
targeted exception rather than a second configuration to maintain.

## Where to set them

Select the object in the Outliner, then use the Inspector, which retargets at the
selected object's overrides while the Opt workspace is active.

## Notes

Overrides are keyed to the operation they belong to. Reordering the stack keeps
them attached to the right operation, including across a preset save and load.

Overrides are part of the stack, so they are saved in a [preset](export.md) and
covered by undo.
