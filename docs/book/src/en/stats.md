# Model statistics

The stats card is a small panel of numbers about the model. You can show or
hide it from the status bar. Every number on it is really measured: there are
no placeholders, and the counts come from the file as the artist made it, not
from what the viewer does to draw it.

The rows are: Draws, Polys, Tris, Verts, GPU Verts, Vtx Splits, UV Sets,
Bones, Clips, Unit, FPS.

Hover over a row to see what it means. Click a row to copy it.

## Verts against GPU Verts

`Verts` is the number of points in the model, counted the way your modelling
program counts them.

`GPU Verts` is the number of points a game would really have to store. Where
two faces meet at a sharp edge, or where the texture layout is cut, the
GPU needs a separate copy of the point for each side. So this number
is usually bigger.

`Vtx Splits` is the gap between the two, as a percentage. This is one of the
most useful numbers on the card. A well-made game model reads a few percent. A
model where every face has its own shading, such as a 3D scan, can read
several hundred percent, which means it is far heavier for a game than its
Verts count suggests.

## Scopes

The card has three columns: the whole file, whatever you have selected, and
whatever is still visible. When a row has nothing to say for a column, it
shows a dash rather than a zero.

The [Opt workspace](opt/comparison.md) adds a second card with the processed
model's numbers, and some extra figures about how efficiently a GPU
can draw it.
