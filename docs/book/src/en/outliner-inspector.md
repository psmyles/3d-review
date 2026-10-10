# Outliner and Inspector

These are the two side panels. The Outliner, on the left, is a list of
everything in the file. The Inspector, on the right, shows details about
whatever you have selected.

Both are in every workspace, and the toolbar button that opens and closes
them is too. Each workspace shows the tabs that make sense for it:

- **3D** - Scene, Materials, Textures, and Animations when the file has any.
- **UV** - Scene and Textures.
- **Tex** - Textures.
- **Opt** - Scene and Materials.
- **Aud** - [Issues](aud/issues.md) and Scene. In the Scene tab each part
  carries a mark for the worst problem found on it.

Each workspace remembers which tab you last had open in it.

## The Outliner

The Outliner is a panel on the left that you can drag wider or narrower.

**Scene** lists every part of the model. A model is usually built as a tree,
where parts are attached to other parts (a hand is attached to an arm, which is
attached to a body). You can see it as that tree, with lines showing what is
attached to what, or as a plain list. A search box filters the list as you
type, and matches are shown flat so one is never hidden inside a folded branch.
Small filter buttons let you show only certain kinds of parts, such as just the
bones.

Clicking a row selects that part. The part lights up in the view, the
Inspector shows its details, and the bounding box and `F` framing follow it.
The arrow keys move up and down the rows.

Hold the primary modifier (`Ctrl` on Windows, `Cmd` on macOS) while clicking to
add a row to the selection or take it back out, and Shift to take every row
between the last one you clicked and this one. You can build the same selection
by [clicking in the viewport](selection.md), and mix the two freely.

Parts that have a shape carry an eye icon that shows or hides them. Hold the
primary modifier while clicking the eye to show only that one part and hide all
the others. Bones select the same way parts do, which is how you feed the
skin-weight heat map; a bone and a part are never selected at the same time, so
picking either kind clears the other.

**Materials** lists the materials, with duplicates merged into one entry.
Selecting one highlights every face that uses it and opens it in the Inspector.

**Textures** lists every image loaded for the scene, with a small preview and
its size in pixels. Clicking one shows it in the Inspector, without changing
what is selected in the scene; clicking it again, or picking anything in the
scene, hands the Inspector back. In the [Texture workspace](tex.md) this tab
is how you choose which image to look at.

**Animations** lists the animations in the file. This tab only appears in the
3D workspace, and only when the file has animations. See
[Skinning and animation](animation.md).

## The Inspector

The Inspector is the panel on the right. For a selected part it shows facts
about it that you can read but not change. For several selected parts, or
several bones, it shows a summary instead: how many there are, what they come to
in total, and their names. For a material it shows the three editable sections
described in [Materials and textures](materials.md). For a texture it shows a
preview and the real facts about the file - its type, size in pixels,
channels, bit depth, size on disk and where it was read from - and which
materials use it.

In the Opt workspace the Inspector changes job: it shows the settings of the
operation you have selected, the export settings, or the per-object settings of
the part you have selected. Per-object settings apply to one part, so with
several selected they follow the last one you clicked.

In the Aud workspace it explains the finding you picked in the Issues list -
what was measured against what limit, why it matters, and how to fix it - or
shows the [audit profile](aud/profiles.md) for editing.
