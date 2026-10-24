# Materials and textures

A material describes what a surface is made of: its color, how shiny it is,
whether it is metal, and so on. A texture is a picture that paints one of those
properties across the surface, so different spots can have different values.

In 3D Review you can change materials while you look at the model. The file
itself is never changed: your edits only live in the viewer, and an
[export](opt/export.md) writes the original values back out.

## The Material section

The Inspector's **Material** section holds the shader type, the transparency
mode (Opaque, Blend or Clip), the base color, the roughness, the metallic
value and the emissive (glow) color.

Roughness has a **workflow** setting. Some tools describe a surface by how
rough it is, others by how smooth it is; they are opposites of the same thing.
Unity, for example, uses smoothness. Pick the one your target uses and the
viewer flips the values for you, including any texture in that slot.

> **A note on metalness.** Older kinds of material (called Lambert and Phong)
> have no metalness setting at all. They do have a "reflection" setting, but
> modelling programs fill that with a default value nobody chose, so the viewer
> deliberately does not treat it as metalness. When it did, nearly every real
> game model showed up looking half or fully metal.

## Texture mapping

Here you choose which image, and which color channel of that image, feeds each
of the seven parts of the material: base color, normal, roughness, metallic,
ambient occlusion, emissive and opacity.

Parts that only need a single value take one channel of an image. That is how
one "packed" image can feed three parts at once, with, say, ambient occlusion
in red, roughness in green and metallic in blue. The viewer guesses the right
channel from the file name when a texture is loaded, and you can change it.

## Textures the file names

When a model loads, the viewer also loads the textures its own materials name
and puts each one in its slot, as if you had picked it yourself. It looks for
each image in this order:

1. inside the FBX file, for files saved with their media embedded;
2. at the path the file wrote down;
3. by its file name: next to the model, in a `textures` folder beside it, and in
   a `textures` folder one level up. An asset is often moved after it is
   exported, and its file name is what still finds its pictures.

Any texture it cannot find is listed in one warning card, so a missing file is
easy to spot. The model itself is shown either way.

A few choices follow what modelling programs do with the same file:

- A texture connected to the base color or the glow (emissive) color *replaces*
  that color, so the color it is multiplied by becomes white.
- A glossiness texture (shiny where it is bright) switches the material to the
  Smoothness workflow, and older Lambert and Phong materials from Maya and 3ds
  Max store theirs in a "shininess" slot that is read the same way.
- A texture in the old Bump slot is used as a normal map only when it looks like
  one (mostly blue). A height map is left out rather than lighting the surface
  wrongly.
- An opacity texture makes the material **Clip** (cut out), which suits leaves,
  hair and fences. Switch it to **Blend** in the Inspector for glass.

Loading these textures is part of opening the file, not an edit, so Undo does not
take them back.

## The texture pool

All the textures for a scene live in one shared pool. Each image is read from
disk once and shared by every material that uses it. The **Texture files**
section lists the pool with small previews and a way to remove an image.

The viewer reads these picture formats itself: PNG, JPEG, TGA, TIFF, PSD
(layered Photoshop files, shown flattened), BMP, GIF, HDR and PNM.

A texture that is in use is watched on disk. When you save it in Photoshop or
Substance, the viewer notices and reloads it, so the change appears without
reopening anything.

The [Texture workspace](tex.md) shows any image in the pool full-screen.
