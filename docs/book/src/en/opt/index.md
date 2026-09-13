# Opt: the optimization workspace

A game has to draw a model many times a second, so a model with fewer
triangles and a tidier layout is cheaper to draw. The Opt workspace is where
you make a model lighter. It is built on
[meshoptimizer](https://meshoptimizer.org/), a well-known library that game
studios use for exactly this.

Opt shares every tool the 3D workspace has, so you can look at the lighter
model with the same shading modes, buffer views and overlays as the original.

If your model has bones, animations or blend shapes, all of that comes through
the process and into the saved file. The workspace itself shows the model in
its bind pose (the pose it was skinned in).

Opt does no work at all until you open it for the first time, and a list of
operations with nothing switched on does nothing, so if you never use it, it
costs nothing.

## How the work goes

1. [Every run starts by joining up identical points](run.md). That is what
   makes all the other steps able to do anything.
2. You build a list of [operations](operations.md), in the order you want them
   to run. You can drag them into a different order any time.
3. [Per-object settings](overrides.md) let you keep a special part out of the
   process, or give one part different settings from the rest.
4. [The comparison view](comparison.md) and a second stats card show you what
   changed.
5. [Export](export.md) saves the result as an FBX file when you say so.

The whole list saves and loads as a preset file, so a team can share one setup
per kind of model.
