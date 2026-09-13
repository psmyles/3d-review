# The Texture workspace

This is a simple picture viewer for the textures in your scene. It shows each
image exactly as it is stored in the file, with none of the lighting or color
processing the 3D view applies, so what you see on screen is what is really in
the file.

- A picker to choose any texture in the pool.
- **RGB / R / G / B / A** buttons to look at all the colors together, or just
  one channel at a time. Switching is instant. The A (alpha) button hides
  itself when the image has no see-through channel.
- Left-drag slides the image, the wheel or right-drag zooms, and `F` fits it to
  the window.
- A background choice: black, white, grey, or a checkerboard. The checkerboard
  shows through any see-through parts, which is the easiest way to see what the
  alpha channel is doing.
- A stats card with the real facts about the file: its type, its size in
  pixels, which channels it has, its bit depth, and how big it is on disk.

Because nothing is changed on the way to the screen, this is the place to
check things like: is the blue channel of a normal map what you expect? Is the
alpha channel actually filled in? Did a mask end up in the channel the material
is reading from?
