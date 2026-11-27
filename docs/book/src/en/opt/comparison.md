# Comparing the result

The controls for comparing the original with the processed model sit in the
middle of the status bar, over the line that divides the view, because they
describe the view as a whole.

## Layouts

**Split** - the original on the left, the processed model on the right. You
can link the two cameras so they move together.

**Overlay** - both models in the same place, one solid and the other a faint
ghost, either see-through or as a wireframe. Press `X` to swap which one is
solid. This is the most reliable way to see exactly where a simplification
moved the surface. A small legend tells you which is which.

**UV view** - the processed model on the left under the
[UV checker](../panels/uv-checker.md), and its UV layout on the right: what an
[Unwrap UVs](unwrap.md) made, on the model and flat at once. It shows the UV
set the stack's Unwrap writes (the selected one's, or the last one's); the
dropdown beside the button shows another set instead, and **Unwrap's set**
goes back to following it. Adding an Unwrap switches to this view. Drag in the
layout half to pan it and use the wheel to zoom, as in the UV workspace; the
half is framed on the layout when you open the view or pick another set. A
diagnostic view, picked on the toolbar, replaces the checker.

**LOD picker** - which level of the chain to show. Once you pick a level, later
runs keep showing it.

## Diagnostic views

The toolbar's view group from the Aud workspace is here too: **Texel
density**, **Triangle density**, **Overdraw** and **Quad overdraw**, described
in [Diagnostic views](../aud/views.md). Each paints both models, so you can see
how a level's triangle sizes or overdraw compare with the original's. The views
and the material buttons before them are one choice: pick a material, such as
**Source Material**, to go back to the shaded model.

The views use the settings of the matching checks in the Aud profile. On a
simplified level the density colours blend from face to face rather than
changing at each face's edge, because its faces share their points.

In the overlay layout, an overdraw view counts the solid model only, and the
legend names only that model.

## The second stats card

This card shows the processed model's measured numbers next to some figures
about how efficiently a GPU can draw it. Each one shows its change
against the original, tinted green or red. Every figure here is one where
**lower is better**, so one rule covers them all: down is green, up is red.

- **ACMR** - how well the GPU can reuse points it has already worked
  on. 3.0 means it never reuses any; about 0.5 is the best a closed shape can
  reach.
- **ATVR** - how many times, on average, each point has to be worked on. 1.0 is
  perfect. Unlike ACMR, this does not change just because the model has more or
  fewer triangles, so it is the fairer one to watch for a single model.
- **Overdraw** - how often a spot on screen is painted and then painted over
  again, measured from several viewpoints around the model.
- **Overfetch** - how much the GPU has to re-read the same memory.
  Higher means the drawing order jumps around.
- **Error** - how far this level has drifted from the model it was made from,
  as measured by the simplifier.
- **Target** - the triangles the level was asked for. A target is a ceiling,
  not a promise: a level that stopped above it carries a note saying what
  held it. **error** means the shape could only go further by moving more
  than the [error limit](simplify.md#target) allows; raise the limit to reach
  the target. **seams** means the model's own seams and open borders allow no
  further simplifying at any limit; weld it first, or let the simplifier
  collapse across seams. **volume** and **surface** mean one of the
  [quality limits](simplify.md#quality-limits) you set held it.
- **Volume** - how much the volume the level encloses has changed from the
  model as it came, as a percentage. Only for a model that encloses one.
- **Distance** - the farthest the level's surface lies from the model as it
  came, as a percentage of the model's size, measured both ways at thousands
  of points.
- **Normal turn** - how far the shading directions have turned, in degrees:
  95% of the surface has turned less than this.

The card is there as soon as you open the workspace. A run with nothing
switched on makes no new model, but it still measures the original, so you can
read the starting numbers before you add anything. That is what makes the
three reorder operations, which change nothing you can see, worth having.
