# The Texture workspace

A 2D image viewer over the scene texture pool, drawn through its own small GPU
path outside the scene's HDR and tone-mapping work, so the pixel on screen equals
the pixel in the file.

- A texture picker for the pool.
- **RGB / R / G / B / A** channel isolation. It is a shader swizzle, so switching
  is instant. The alpha segment hides itself for an opaque image.
- Mipmapped, with pan (left-drag), zoom (wheel or right-drag), and `F` to fit.
- Background fill: black, white, grey or checker.
- A stats panel reporting the file's real properties: format, pixel dimensions,
  channel layout, bit depth and size on disk.

Because the displayed texel equals the stored texel, this is the view to use when
checking whether a normal map's blue channel is what you expect, whether an
alpha channel is actually populated, or whether a mask ended up in the channel
the material is reading.
