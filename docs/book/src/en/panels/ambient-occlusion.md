# Ambient Occlusion

Screen-space horizon-based occlusion, following Intel's XeGTAO. On by default.

It darkens only the ambient light, so direct and emissive light are never dimmed -
the effect adds contact shadow without flattening the lighting.

## Settings

**Radius** - a multiplier on the automatic radius. 1 is automatic.

**Intensity** - how strongly the occlusion darkens the ambient term.

**Thickness** - compensation for thin occluders. Raise it when a thin surface
casts far more occlusion than it should.

**Quality** - Low, Medium or High. This controls the sample count and how many
times the edge-aware denoise runs.

## The radius is derived from the view

The radius is a fraction of the world-space height the viewport covers at the
orbit target, capped at half the model's bounding sphere. That is what lets the
same settings look right on a 10 cm prop and a 10 km landscape.

Keying it to the model's size instead - which is what this used to do - made a
room interior ask for a four-metre radius, spreading the fixed number of samples
so thinly that contact occlusion was never found and the effect all but
vanished. Deriving it from the view also pins the radius's size in *pixels*, so
the search always spans the same screen distance and the sample count always
resolves it.

The Radius knob is a multiplier over that derived value, not an absolute.

## It converges while the view is still

Each still frame is folded into a running average over 24 frames, each with a
distinct sample pattern, and then every occlusion pass is skipped entirely. The
result settles in about half a second, after which an idle viewer showing
ambient occlusion costs *less* than one without it.

Moving the camera, changing a setting, or changing the pose starts it again.
