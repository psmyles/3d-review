# Comparing the result

The centre group in the status bar carries the comparison controls. It sits over
the split's divider because it describes the viewport as a whole.

## Layouts

**Split** - source on the left, processed on the right, with an option to link
both cameras so they move together.

**Overlay** - both in one space, one mesh shaded and the other a ghost, either
see-through or wireframe. `X` swaps which one is solid, which is the most
reliable way to spot where a simplification moved the silhouette. A legend names
which is which.

**LOD picker** - which level of the chain is shown. Once you pick a level, later
runs keep showing it.

## The second stats card

Reports the processed mesh's measured counts alongside the figures meshoptimizer
measured, each with its change against the source tinted green or red. Every
figure there is one where **lower is better**, so one rule covers them all: down
is green, up is red.

| Figure | What it means |
| --- | --- |
| **ACMR** | Average Cache Miss Ratio: vertex-shader runs per triangle, simulated against a 16-entry GPU vertex cache. 3.0 means no vertex is ever reused; about 0.5 is the best a closed mesh can reach. |
| **ATVR** | Average Transformed Vertex Ratio: how many times the average vertex gets shaded. Unlike ACMR it does not shift with the triangle count, so it is the fairer figure for judging one mesh's ordering against itself. |
| **Overdraw** | Pixels shaded divided by pixels covered, measured from viewpoints around the mesh. |
| **Overfetch** | Vertex-buffer bytes read divided by the buffer's size. Higher means the index order jumps around and the GPU re-reads the same memory. |
| **Error** | How far this LOD deviates from the mesh it was simplified from, as the simplifier measured it. |

The card is up as soon as the workspace opens: a run with nothing enabled produces
no mesh but still measures the source, so the baseline is readable before
anything is added. That is what makes the three reorder operations, invisible in
the viewport, worth having.
