# Profiles

A profile is the set of rules a model is checked against: which checks run,
how serious each one is, and their limits. There are three built in, one per
target: **Unity**, **Unreal** and **Generic**. They share their limits - four
bones per point, 80 bones per mesh, 1024 texture pixels per meter on a
1024-pixel texture, 200,000 triangles per object and 2 million in the file, 32
draw calls, 4 materials per mesh - and differ where the engines differ: Unreal
works in centimeters with Z up, Unity in meters with Y up, and each treats some
findings as more serious than the other does.

Click the **Profile** row at the top of the Issues list to open the profile in
the Inspector. Every change you make there re-checks the model at once; a check
whose settings did not change is not run again. Changes can be undone with
`Ctrl+Z`.

The profile you used last is remembered and used again the next time you start
the viewer.

## Sharing a profile

**Save profile** writes the profile to a small file. A team that checks every
model against the same file gets the same findings. **Load profile** replaces the
current profile with one from a file.

The file is a plain JSON document. Values outside what the editor allows are
brought back into range when it is loaded, and checks a newer viewer added are
simply skipped, so a profile can be edited by hand.

**Reset** puts every setting back to the built-in values for the profile's
engine.
