# Ambient Occlusion

Ambient occlusion adds the soft shadows you see in corners, creases, and
anywhere light has trouble reaching. It makes a model look grounded and solid
instead of floating and flat. It is on to begin with.

It only darkens the soft, surrounding light. Direct light and glowing parts are
never dimmed, so the effect adds depth without making the lighting look muddy.

## Settings

**Radius** - how far the shadow reaches into a crease. The viewer works out a
good size on its own; 1 means "use that", 2 doubles it, 0.5 halves it.

**Intensity** - how dark the shadows get.

**Thickness** - helps with very thin surfaces, like a leaf or a sheet of paper.
Raise it if a thin surface is casting a much bigger shadow than it should.

**Quality** - Low, Medium or High. This decides how much work goes into each
frame and how many times the result is cleaned up.

## Why the size follows the view

The shadow size is based on how much of the scene is on screen, not on how big
the model is. That is what lets the same settings look right on a tiny prop
and on a huge landscape.

An earlier version based it on the model's size instead. A whole room interior
then asked for a shadow several meters wide, which spread the work so thinly
that the small shadows in corners were never found and the effect all but
disappeared. Following the view also means the shadow always covers about the
same distance on your screen, so there is always enough detail to find it.

The Radius setting scales that automatic size; it is not a distance on its own.

## It cleans itself up while the view is still

Every frame while the camera is still, the viewer adds a little more detail
to the shadows and averages it in, for 24 frames. After that it stops working
on them entirely. The result settles in about half a second, and from then on
a viewer sitting idle with this on costs *less* than one without it.

Moving the camera, changing a setting, or changing the pose starts it over.
