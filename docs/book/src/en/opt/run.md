# How a run works

A run is what happens every time you change the list of operations: the
viewer takes the original model, puts it through each operation in turn, and
shows you the result.

## Every run begins by joining identical points

This first step is what makes everything else work, so it is worth
understanding.

When an FBX file is read, every corner of every face becomes its own separate
point, even when several corners sit in exactly the same spot. A real model
with about 10,000 triangles arrives with about 30,000 points and no sharing at
all. Every operation in the list works by looking at which triangles share
which points, so on a model like that they would all do nothing: there is
nothing for the graphics card to reuse, no edge a simplifier can fold, and a
LOD chain that removes nothing.

So before any operation runs, the viewer joins up points that are identical in
every way, down to the last bit. Nothing you can see changes. On that same
model it takes the 30,000 points down to about 5,000, after which a "keep 50%
of the triangles" target is hit exactly.

This step is not something you add to the list, because there is never a
reason to skip it. The [Weld Vertices](operations.md) operation is for the
kind of joining that *does* change the model.

## The starting numbers are measured after that step

The "before" numbers on the second stats card, the ones every change is
compared against, come from the model after that joining step, not from the
raw file.

If they came from the raw file, your operations would get credit for a huge
"improvement" that every game engine's import step gets for free anyway, and
the starting cache figure would be a meaningless 3.0. Measuring after the join
also makes the "before" number match the `GPU Verts` row on the
[stats card](../stats.md), so the two cards agree with each other.

## Runs do not pile up

Every change you make starts a run on a background thread. Only one run happens
at a time. If you change something while a run is going, the viewer notes that
the run is out of date and starts a fresh one with your latest settings as soon
as the current one finishes. So you can drag a slider freely: the result
settles on its own, and a result that is already out of date is thrown away
rather than shown to you.

A notice appears only if a run is taking a while.
