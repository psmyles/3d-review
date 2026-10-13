# Animation Curves

Sets how the [animation curves](../animation.md#animation-curves) are drawn: the
path each moving bone or object takes through the selected animation. Open it
by right-clicking the curves button at the end of the playback controls.

## Settings

**Shape** - how the line runs from one frame's dot to the next. **Straight**, to
begin with, joins each dot to the next with a straight line. **Smooth** draws a
curve through the dots.

**Line Width** - how thick the curves are. It is measured on the screen, not in
the model, so a curve keeps the same weight however close you zoom in.

**Line Opacity** - how solid the curves are. Lower it to see the model through a
dense tangle of paths. The dots stay solid, so the frames still read.

**Dot Size** - how large the dot on every frame is.

## What it is for

The dots are the measurements: each one is where the bone or object really is on
that frame. The spacing between them is the speed - close together where it
moves slowly, far apart where it moves fast - so the dot size is worth turning
up when the timing is what you are checking.

The line between the dots is drawing, not measurement. Between two frames the
player blends from one pose to the next, and neither shape traces exactly what
that does: **Smooth** shows the shape of the motion the way an animator draws
it, and **Straight** shows plainly where each frame is. On a fast, sweeping
movement with few frames the two differ most - which is also where it is worth
looking at both.

## Notes

Changing the shape redraws the curves. The width, opacity and dot size cost
nothing to change, so drag them freely while an animation plays.

A smooth curve only adds points where the path actually bends, so on most
animations it costs no more than the straight one.
