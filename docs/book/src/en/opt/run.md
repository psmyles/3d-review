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
nothing for the GPU to reuse, no edge a simplifier can fold, and a
LOD chain that removes nothing.

So before any operation runs, the viewer joins up points that are identical in
every way, down to the last bit. Nothing you can see changes. On that same
model it takes the 30,000 points down to about 5,000, after which a "keep 50%
of the triangles" target is hit exactly.

This step is not something you add to the list, because there is never a
reason to skip it. The [Weld Vertices](operations.md) operation is for the
kind of joining that _does_ change the model.

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

Every change you make starts a run on a background thread, and only one run
happens at a time. If you change something while a run is going, **that run is
abandoned** and the new settings go in straight away - you never wait out a
result you have already moved past. So you can drag a slider freely: the result
settles on its own, and a result that is out of date is thrown away rather than
shown to you.

A notice appears only if a run is taking a while, and it names the step the run
is on. Where a step works through the objects one at a time it names each as it
finishes, with a count, so a long run is visibly moving rather than merely
slow.

## Watching a rebuild settle

[Remesh](remesh.md) is the one operation whose work is worth watching. It
improves its answer in passes, and each pass is a complete mesh, so the viewport
shows the rebuild converging in place instead of holding the old result until
the end. What you see is the real thing getting steadily better.

The stats card is the exception and deliberately so: it keeps showing the last
*finished* run until this one lands. A half-measured count is worse than a
slightly old one.
