# Opt: the optimization workspace

A fourth workspace that turns the viewer into a mesh optimizer, built on
[meshoptimizer](https://meshoptimizer.org/). It shares every 3D control, so the
processed mesh can be inspected with the same shading modes, buffer views and
overlays as the source.

A skinned, animated or blend-shaped asset keeps all of that through the stack and
into the export; the workspace itself shows the bind pose.

Nothing for Opt is built until the workspace is first opened, and a stack with
nothing enabled schedules no work, so a session that never uses it pays nothing.

## The shape of the work

1. [A run starts by merging identical vertices](run.md), which is what makes
   every operation below able to do anything at all.
2. You build an ordered, re-orderable stack of [operations](operations.md).
3. [Per-object overrides](overrides.md) keep a hero prop or a collision shell out
   of it, or give one object different settings.
4. [The comparison view](comparison.md) and a second stats card show what changed.
5. [An explicit export](export.md) writes the chain to FBX.

The whole stack saves and loads as a versioned JSON preset, so a studio can share
one per asset class.
