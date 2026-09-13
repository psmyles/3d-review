# Materials and textures

Materials can be edited live. The imported material data is never modified: the
edits live in the renderer, and the [export](opt/export.md) writes the source's
own authored values back.

## The Material section

The Inspector's **Material** section carries the shader type, transparency mode
(Opaque, Blend or Clip), base color, roughness, metallic and emissive.

Roughness follows a per-material **workflow** setting: native
metallic-roughness, or Unity-style smoothness, where the slider and any bound map
read as smoothness and the shader flips them.

> **Note on metalness.** A classic Lambert or Phong material declares no
> metalness at all. Its `ReflectionFactor` is Phong reflectivity, a slot that
> DCC packages fill with a non-zero default nobody authored, so it is deliberately
> *not* read as metalness - doing so made nearly every real game asset arrive
> half or fully metal.

## Texture mapping

Binds a pooled image plus a channel to each of the seven PBR slots: base color,
normal, roughness, metallic, ambient occlusion, emissive and opacity.

Single-value slots pick one channel, so an ORM-packed map can feed three slots
from one file. Channel routing is guessed from the filename on import and can be
overridden.

## The texture pool

Textures live in a scene-wide pool, decoded once and shared. The **Texture files**
section lists the pool with thumbnails and a remove action.

Source formats are decoded directly with no ImageMagick dependency: PNG, JPEG,
TGA, TIFF, PSD (layered files, read as their flattened composite), BMP, GIF, HDR
and PNM.

A bound texture is watched on disk and decoded again when it changes, so a save
in Photoshop or Substance shows up in the viewport without reopening anything.

The [Texture workspace](tex.md) is the full-screen view of any image in the pool.
