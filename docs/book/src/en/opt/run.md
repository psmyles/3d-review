# How a run works

## Every run begins by merging identical vertices

This is what makes the rest work at all.

FBX import splits each face corner into its own vertex, so a mesh reaches the
optimizer with no shared vertices whatsoever: a real 10,006-triangle asset arrives
as 30,018 vertices. Every meshoptimizer operation works through the index buffer,
so on that mesh they would all be no-ops - nothing for the vertex cache to reuse,
no edge a collapse may cross, a LOD chain that removes nothing.

The merge pass joins vertices that are identical in *every* attribute, byte for
byte, before any operation runs. Nothing visible changes. On that asset it is
30,018 down to 5,284, after which a 50% LOD target is hit exactly.

It is deliberately not a stack operation, because skipping it is never useful.
The [Weld Vertices](operations.md) operation is for the *lossy* merges.

## The baseline is measured after that pass

The run's baseline figures - what every change on the second stats card is
measured against - come from the merged mesh, not the corner-split buffer import
handed over.

Quoting against the corner-split buffer credited your operations with a large
free win that any engine cooker also gets, and made the baseline cache figure a
meaningless 3.0. The indexed baseline also equals the
[stats panel](../stats.md)'s `GPU Verts` row, so the two cards agree.

## Runs are coalesced

Every edit reprocesses on a worker thread. Only one run is ever in flight: edits
arriving mid-run mark it out of date and it restarts once with the latest stack,
so dragging a slider settles on its own without a debounce timer, and a result
that has been overtaken is dropped rather than shown.

A notice appears only if a run takes a while.
