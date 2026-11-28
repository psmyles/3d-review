# Colliders

A game model often carries collision meshes: simple shapes, such as a box, a
capsule or a convex hull, that the game uses for physics and never shows. The
viewer picks them out by their names and draws them apart from the model.

When the model has any, a **Colliders** button appears at the end of the
display group on the right of the toolbar. Click it to hide or show them, and
right-click it to open this panel. A model with no collision meshes has no such
button, just as a model with no bones has no skeleton button.

The panel also opens from the menu, under **Preferences > Colliders...**,
whatever is loaded. That is the way in when a model's collision meshes go by
names this panel does not know yet, so the button has not appeared.

## Settings

**Colliders** - how collision meshes are drawn while they are shown. **As
wireframe**, to start with, draws their edges only, in their own color: solid
where you can see them, and faint where the model hides them, so a hull inside
the model still shows. This does not depend on the wireframe being on.
**Shaded** draws them like any other part. While they are not shaded they
cannot be clicked in the view; the Outliner still lists them.

**Collider color** - the color of the collision meshes' edges.

**Collider names** - the name patterns that mark a collision mesh, one to a
row. `*` matches any run of characters and `?` any one. To start with:
`*_box`, `*_sphere`, `*_capsule`, `*_hull` and `*_convex`; Unreal's `UBX_*`,
`USP_*`, `UCP_*` and `UCX_*`; and Godot's `*-col`, `*-colonly`, `*-convcol`
and `*-convcolonly`.

- Click a row to change it. The change is taken when you press Enter or click
  elsewhere, and `Esc` puts the row back as it was.
- **X** beside a row removes it. Emptying a row removes it too.
- Type into the **Add new** field at the end to add a name. Several names
  separated by commas become a row each.

A pattern that does not parse turns red, and is not taken until it does. A
name that is already in the list is not added twice.

**Reset All** puts the style, the color and the starting names back. It does
not touch the toolbar button: whether the collision meshes are shown is a
question about the model in front of you.

## Notes

The names are what make the toolbar's Colliders button appear, so the button
shows up the moment a name you add matches a part.

`review3d-cli render --colliders "*_hull,UCX_*"` draws them the same way in a
render.

See also [Wireframe](wireframe.md).
