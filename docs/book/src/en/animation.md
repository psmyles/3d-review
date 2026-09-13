# Skinning and animation

A skinned mesh rests in the file's *default* pose, skinned onto the skeleton as
drawn, not the bind pose its buffers hold. Deformation is GPU linear-blend
skinning over each vertex's exact list of influences, with weights normalized the
way `ufbx` does, alongside rigid node animation and blend shapes through the same
vertex shader stage.

## Clips

Every FBX animation stack imports as a clip, baked at the file's own frame rate
with no key reduction. Frame counts come from the stack's time range, not from
key counts.

Clicking a clip in the Outliner's Animations tab selects it, paused on its first
frame, and raises the playback transport in the status bar: go to start, step
back, play/pause, step forward, a scrubber, a `frame / total   seconds` readout,
looping (on by default) and a speed control. Clicking the clip again returns to
the rest pose.

On a narrow window the scrubber shrinks and then the readout moves to its
tooltip, so the controls stay reachable instead of running under the buttons
beside them.

`Space` plays and pauses; `,` and `.` step single frames.

## Framing an animation

While a clip is selected, `F` and the bounding box use that clip's full range of
motion rather than the frame you are on. That envelope is measured once, just
after the model appears, so a clip selected in the first moments of a large load
frames on the whole model and tightens up when the measurement lands.

## Limitations

Dual-quaternion skins are evaluated as linear blends, and the Inspector says so.
The [Opt workspace](opt/index.md) draws the bind pose on purpose, though its
processed meshes carry the skin through to the export.

Playback is view state and is never captured by undo.
