# Operations

An operation is one step that changes the model. You build a list of them, and
they run from top to bottom. You can drag them into a different order, and you
can switch one off without removing it, which is handy for comparing the result
with and without it.

- **Weld Vertices** - joins points that sit in the same place. Every run
  already joins points that match in every way; this operation goes further, by
  ignoring the shading direction, the texture layout or the colors when
  comparing, or by allowing a small gap. This one *does* change the model.
- **Filter Triangles** - removes broken triangles (ones squashed into a line)
  and exact copies of other triangles. Copies that face the other way are kept,
  because that is how double-sided surfaces are made.
- **Prune Components** - removes small loose pieces below a size you choose:
  stray shells, and single faces left floating around by mistake.
- **Reduce** - [simplifies](simplify.md) the model **in place**. Every
  operation below it works on the simplified model, LOD levels start from it,
  and the export saves it in place of the original.
- **Generate LODs** - makes a chain of simpler copies of the model (levels of
  detail) for a game to show at a distance. Operations above it run once on the
  base model; operations below it run on every level. You can only have one in
  the list. The starting chain is three levels at 50%, 25% and 12.5%.
- **Bake AO to Vertex Colors** - works out [soft shadows](ao.md) for each point
  and stores them as colors on the points. Does not change the shape.
- **Optimize Vertex Cache** - reorders the triangles so the graphics card can
  reuse work it has just done. Watch the ACMR and ATVR numbers.
- **Optimize Overdraw** - reorders the triangles so nearer ones tend to be
  drawn first, and the graphics card wastes less effort on parts that end up
  hidden.
- **Optimize Vertex Fetch** - reorders the points into the order they are read,
  and removes any point nothing uses.

## The three reorder operations

Optimize Vertex Cache, Optimize Overdraw and Optimize Vertex Fetch change
nothing you can see. What they change shows up on the second stats card: ACMR
and ATVR for how well the graphics card reuses its work, Overdraw for wasted
painting, and Overfetch for how tidily the model's data is read. Reading those
numbers is the only way to judge them, which is why the card is there before
you add anything.

A reorder that did not move any of those numbers is one you can remove.

## What survives an operation

Everything an operation does not change is carried through: the original
faces and edges with their smoothing, crease, hole, group and visibility
settings, extra color sets, vertex creases, bone weights, and blend shape data.

Each kind of operation handles that in its own way. One that keeps triangles
whole (filter, prune, the reorders) keeps the original faces and simply drops
any face that lost a triangle. A simplify throws the original faces away,
because it rebuilds the surface, and the [export](export.md) saves that level
as triangles with a note saying which operation was responsible.
