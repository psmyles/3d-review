# Aud: the audit workspace

A model can load and look fine and still cause trouble later: a face that is
inside out, a part that is scaled by 100, UVs that overlap in the lightmap,
a vertex moved by nine bones when the engine only keeps four. The Aud
workspace checks the model for problems like these and shows you where they
are.

The checks run by themselves every time you open a file, in the background,
so the result is usually ready before you look. The **Aud** button in the
middle of the toolbar shows how many checks found an error or a warning,
from whichever workspace you are in. Checks that only found something worth
knowing (Info) are not counted there.

Aud never changes the model. It tells you what to fix, and where; the fixing
happens in your modelling program, or for some problems in the
[Opt workspace](../opt/index.md).

## How the work goes

1. Pick the [profile](profiles.md) for the engine the model is going to. The
   profile says which checks run, how serious each one is, and their limits.
2. Read the [Issues list](issues.md) on the left. Each check that found
   something is listed with how many problems it found.
3. Click a finding. The model turns plain grey and the problem is drawn on it
   in the finding's colour; the Inspector explains what it means and how to fix
   it. See [the checks](checks.md) for every check in detail.
4. Some findings are easier to judge as a colour map. The
   [diagnostic views](views.md) show texel density, triangle density and
   overdraw over the whole model.
5. [Copy a summary or save a report](report.md) to send with the asset.

Aud has every tool the 3D workspace has, so you can turn on the wireframe or
change the shading while you look. It also shows an animation the way 3D
does: if a clip is playing, the highlights move with the mesh.
