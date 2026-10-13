# Skinning and animation

Some models have a skeleton inside them. Skinning is the way the surface of
the model is attached to those bones, so that when a bone moves, the surface
bends with it. An animation is a recording of how the bones move over time.

3D Review shows a skinned model in the pose the file says it should rest in,
attached to its skeleton as drawn. All the bending is done on the graphics
card, using each point's own list of bones and weights, in the same way most
game engines do it. Simple whole-part movement and blend shapes (morphs, such
as facial expressions) are handled the same way.

## Clips

Every animation in the file becomes a clip. Clips are sampled at the file's
own frame rate, and nothing is thrown away. Frames are always counted at the
rate of the file a clip came from.

Click a clip in the Outliner's Animations tab to select it. The model pauses on
the clip's first frame and the playback controls appear in the status bar: go
to start, step back, play/pause, step forward, a scrubber you can drag, a
readout showing `frame / total   seconds`, a loop switch (on to begin with), a
speed control and the animation curves switch (see below). Click the clip again
to return the model to its resting pose.

If the window is narrow, the scrubber shrinks first, and then the readout
tucks itself into the scrubber's tooltip, so the controls always stay reachable.

`Space` plays and pauses; `,` and `.` step one frame back or forward.

## Animation files with no model in them

Animations are often saved in files of their own, apart from the character they
were made for. Such a file holds a skeleton and its clips but no surface, and
3D Review opens it as exactly that: the skeleton, with the
[skeleton](panels/skeleton.md) overlay switched on so there is something to
see, ready to play. `F` frames the skeleton's joints, and while a clip is
selected, the whole area they move through.

A file with neither a surface nor a skeleton in it has nothing to show, and is
refused with a message saying so.

## Animation curves

The last button in the playback controls shows the animation's curves: the path
each moving bone or animated object takes from the first frame to the last. It
is off to begin with, and it stays on while you switch between animations, so
you can compare their paths.

Every path has a color of its own, and while the curves are on the skeleton's
bones take the same colors (turn the [skeleton](panels/skeleton.md) on to see
them). Along each path there is a dot on every frame, so dots spread far apart
mean fast movement and bunched-up dots mean slow, and a larger dot sits where
the bone or object is on the current frame. It slides along its path as the
animation plays.

A path follows the point the bone or object turns around: a bone's joint, or an
object's pivot. A bone that only turns in place, or an object that never moves,
has no path.

To look at just a few paths, [select](selection.md) them: pick bones in the
Outliner, or click them in the viewport with the skeleton on, and only their
paths are drawn. Selecting an object or a group shows the paths of everything
inside it. If nothing you select has a path, all of them stay on screen. An
object you hide in the Outliner hides its path too.

## Framing an animation

While a clip is selected, `F` and the bounding box use the whole area the clip
moves through, not just the pose you are on. That area is measured once, just
after the model appears. If you select a clip in the first moments of a big
load, the framing covers the whole model to begin with and tightens up when
the measurement lands.

## Limitations

Some files ask for a fancier kind of skinning called dual quaternion. The
viewer shows it using the simpler linear kind, and the Inspector tells you when
that is the case.

The [Opt workspace](opt/index.md) shows the model in its bind pose (the pose
it was skinned in) on purpose. The models it produces still carry the skeleton
through to the export.

Playing an animation is not an edit, so undo never touches it.
