# Baking edge wear

**Bake Edge Wear to Vertex Colors** works out how sharply the surface turns at
each point and stores it as a color on that point: a value from 0 (flat) to 1
(a sharp edge). A shader can then wear paint off, brighten metal or add grime
exactly where a part's edges are, with no texture painted for it.

It works from the shape alone, so it is fast and gives the same result every
time. Put it below **Generate LODs** and every level gets values from its own
edges.

## How it measures

Wherever two faces meet, the angle between them says how sharp that edge is.
Each point takes the sharpest edge that meets there:

- an edge turning less than **Start angle** wears nothing;
- an edge turning **Full angle** or more wears fully;
- in between, the wear rises along a ramp, which **Falloff** bends: 1 is
  straight, more than 1 keeps the wear to the sharpest edges, less than 1 brings
  it on early.

Only edges that stick out count, as knocks and rubbing wear them. Turn on
**Concave edges too** to count edges that turn inward as well; crevices are
usually what [ambient occlusion](ao.md) darkens instead.

A border between two materials of one object is not an edge: the object is
measured as one surface.

## Bake from

**This level** measures each level's own edges. **Original model** measures the
model as it came in and gives each level's points the wear of the nearest point
on the same part there, so a rounded edge that simplifying has turned into one
sharp corner still wears as the rounded edge did, and every level matches.

## Where it is written

**Write to** and **sRGB** work as they do for the [AO bake](ao.md): one channel
of the vertex color (blue by default, so it can sit beside AO in alpha), all
three as gray, multiplied into the colors already there, or alpha.
