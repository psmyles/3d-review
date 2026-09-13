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

**LOD picker** - which level of the chain to show. Once you pick a level, later
runs keep showing it.

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

The card is there as soon as you open the workspace. A run with nothing
switched on makes no new model, but it still measures the original, so you can
read the starting numbers before you add anything. That is what makes the
three reorder operations, which change nothing you can see, worth having.
