# Workspaces

Four modes, switched from the segmented control in the middle of the toolbar. The
toolbar swaps its tool groups to match the active workspace.

| Workspace | What it is |
| --- | --- |
| **3D** | The scene viewport: shading modes, debug overlays, materials, animation. |
| **UV** | A 2D [UV-layout viewer](uv.md) with its own pan and zoom camera. |
| **Tex** | A 2D [image viewer](tex.md) over the scene texture pool. |
| **Opt** | [Mesh optimization](opt/index.md), with a side-by-side or ghosted comparison against the source. |

The 3D workspace is where most review happens; the other three are focused views
of one aspect of the same asset. Selection, visibility and the loaded model are
shared across all four, so hiding a mesh in the Outliner hides it everywhere it
can be hidden.

Nothing for Opt is built until that workspace is first opened, so a session that
never uses it pays nothing for it.
