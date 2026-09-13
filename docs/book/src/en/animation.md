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
own frame rate, and nothing is thrown away.

Click a clip in the Outliner's Animations tab to select it. The model pauses on
the clip's first frame and the playback controls appear in the status bar: go
to start, step back, play/pause, step forward, a scrubber you can drag, a
readout showing `frame / total   seconds`, a loop switch (on to begin with) and
a speed control. Click the clip again to return the model to its resting pose.

If the window is narrow, the scrubber shrinks first, and then the readout
tucks itself into the scrubber's tooltip, so the controls always stay reachable.

`Space` plays and pauses; `,` and `.` step one frame back or forward.

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
