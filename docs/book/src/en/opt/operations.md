# Operations

The stack is ordered and can be reordered; operations apply top to bottom, and
each can be switched off without deleting it.

| Operation | What it does |
| --- | --- |
| **Weld Vertices** | Widens what counts as a match beyond the exact merge every run already does, by dropping normals, UVs or colors from the comparison, or allowing a small tolerance. This one *does* change the mesh. |
| **Filter Triangles** | Removes broken triangles (two corners at one position) and exact duplicates. Duplicates wound the other way are kept, for double-sided geometry. |
| **Prune Components** | Removes disconnected pieces below a size threshold: stray shells and orphaned faces. |
| **Reduce** | [Simplifies](simplify.md) the mesh **in place**. Its output is what every later operation sees, what LOD levels start from, and what the export writes in the source mesh's place. |
| **Generate LODs** | Fans out into a LOD chain. Operations above it run once on the base mesh; operations below it run on every generated level. At most one per stack. The default chain is three levels at 50%, 25% and 12.5%. |
| **Bake AO to Vertex Colors** | [Raycast ambient occlusion](ao.md) baked into the vertex-color set. Changes no geometry. |
| **Optimize Vertex Cache** | Reorders triangles for the GPU's vertex cache. Watch ACMR and ATVR. |
| **Optimize Overdraw** | Reorders triangles front to back within cache-friendly clusters, so the GPU shades fewer hidden pixels. |
| **Optimize Vertex Fetch** | Reorders vertices into the order the index buffer reads them, and drops anything unreferenced. |

## The three reorder operations

Optimize Vertex Cache, Optimize Overdraw and Optimize Vertex Fetch change nothing
you can see in the viewport. What they change is on the second stats card: ACMR
and ATVR for the cache, overdraw for the pixel cost, overfetch for the vertex
buffer read pattern. Reading those figures is the only way to judge them, which is
why the card is up before you add anything.

A reorder that moved nothing is a reorder worth removing.

## What survives an operation

Everything the stack did not change is carried through: the source's faces and
edges with their smoothing, crease, hole, group and visibility layers, extra
color sets, vertex creases, skin weights and extra skin layers, and blend-shape
offsets.

Operations keep them by class. One that keeps triangles whole (filter, prune, the
reorders) reconciles the polygon data by triangle content and drops faces that
lost a triangle. A simplify clears it, and the [export](export.md) writes that
level as triangles with a note naming the operation that did it.
