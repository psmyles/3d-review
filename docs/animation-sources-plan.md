# Plan: animation from separate files

Status: agreed with the user on 2026-10-13, being built. When it ships, fold the
settled parts into `ARCHITECTURE.md` (crate map + settled decisions) and the
traps into `GOTCHAS.md`, then delete this file, as was done with
`mac-port-plan.md`.

## The problem

Game characters are commonly delivered as one **rig** file (meshes, skeleton,
skins, blend shapes, no clips) and any number of **animation files** next to it:
`SK_Eve_Rig.fbx` beside `AN_Eve_Animations.fbx`, `AN_Eve_Intro.fbx`,
`AN_VO_Eve_DynamicAnimations.fbx`. Before this work the viewer refused an
animation file outright ("no triangulatable mesh data found in FBX") and had no
way to play one file's clips on another file's meshes.

## What was decided

- **A session is one rig plus any number of animation files.** Never two meshes
  side by side; everything built around one `ModelData` (the renderer's source
  slot, materials, textures, stats, audit, Opt, export, undo) stays as it is.
- **An animation file opened on its own shows its skeleton playing**, with no
  mesh. The skeleton overlay turns itself on, and `F` frames the bones.
- **Clips play one at a time.** A face clip plays on a body at rest and a body
  clip with a neutral face. Playing a body clip and a face clip together
  (layering) is a later step, not v1.
- **Each clip keeps its own file's frame rate.** The rig and its animations
  routinely disagree (the Eve rig is 24 fps, its animations 30), and a rig with
  no clips carries its rate only as an unused scene setting, so a difference is
  never a warning.
- **The rest pose comes from the rig.** A channel an animation file does not key
  keeps the rig's rest value, never the animation file's default pose (which is
  whatever frame the exporter was parked on).
- **Exports leave attached clips out.** An Opt LOD export writes the rig file's
  own clips only.
- **The viewer does not remember sets of files.** Recent Files lists each file on
  its own; which animations go with which rig is the user's to manage.

## Which file is which

A file's role is decided by whether it *binds*, not by whether it has geometry:
the facial sample carries the rig's six head meshes (identical vertex counts)
purely to host its blend-shape curves.

- **A file binds to the rig** when at least one of its tracks resolves onto the
  rig: a node track by node name, or a blend-shape weight track by mesh node name
  plus channel name — **and** every mesh it carries has a mesh of the same name
  in the rig. A binding file is attached as an animation file. Its own meshes are
  ignored, and only its clips are used. (The mesh condition was added while
  building step 2: without it, a second character sharing the rig's bone names —
  every Character Creator export does — would attach to the first instead of
  replacing it. The facial sample still binds: its six meshes are the rig's.)
- **Several files dropped at once** load as one batch. The rig is the geometry
  file with no clips, ties going to the one with the most triangles; every other
  file attaches if it binds. A second geometry file that binds to nothing is not
  loaded, and one card lists what was left out.
- **A file dropped onto a loaded session** attaches if it binds. Otherwise it
  replaces the whole session, as a drop always did.
- **With no rig at all**, the first animation file's skeleton stands in as the
  rig and the others attach to it. A real rig arriving later takes over, and
  every animation file re-attaches to it.
- **File > Open** always starts a new session, and now takes several files.
  **File > Add Animations...** attaches files to the current session. A Recent
  Files entry behaves like a drop.

## Binding

- **Re-point, don't resample.** Attaching rewrites each of the file's clips so
  its tracks index the rig's nodes and blend channels; tracks with no target are
  dropped. Everything downstream (`anim::evaluate_pose`, GPU skinning, the clip
  envelope, the motion-path overlay, the transport) already works on one
  `AnimationClip` against one `ModelData`, so none of it changes.
- **Node names match with Maya namespaces removed** (`Eve:CC_Base_Hip` matches
  `CC_Base_Hip`). Agreement on parent names is the confidence check.
- **Blend-shape weights match by mesh node name plus channel name.** ufbx names a
  channel after its deformer (`Morpher_CC_Base_Body.Brow_Down_L`), and that full
  name matched all 1374 channels of the facial sample.

## What is checked when a file attaches

Anything that does not line up is reported up front, as a warning card, and in
the file's binding report.

| Check | Outcome |
|---|---|
| Different units or up axis | Warn, then convert, so it plays the way an engine importing both files would |
| Bone proportions differ (matched bone lengths) | Warn, and play the keys as authored, which is what the engine would show |
| A matched mesh's vertex count differs | Warn: the file was probably exported from an older version of the mesh |
| An animation-file **bone** with no match | Warn: most likely a renamed or missing bone |
| Unmatched non-bone nodes (control rigs) | Listed, no warning |
| Rig bones / channels the file does not animate | Listed, no warning: a face-only file is meant to leave the body alone |
| Different frame rate | Nothing: each clip plays at its own rate |

## Interface

- **Outliner, scene tab:** each file is a top-level group. An animation file's
  group shows its node tree with matched / unmatched marks; clicking a matched
  node selects the rig node it drives. Selecting the file's own row shows its
  binding report in the Inspector.
- **Binding report:** coverage per half (skeleton: rig bones animated / total;
  blend shapes: channels animated / total, over matched meshes), then the
  unmatched lists both ways and any mismatch warnings.
- **Animations tab:** clips grouped by the file they came from, each group
  showing its file's frame rate. Search filters across groups.

## Measured on the samples

| | `SK_Eve_Rig` | `AN_Eve_Animations` | `AN_VO_Eve_DynamicAnimations` |
|---|---|---|---|
| Source | Maya 2024 | FBX SDK 2020.1 (MotionBuilder HumanIK) | Maya 2022 |
| Units, axes, rate | cm, Y-up, 24 fps | cm, Y-up, 30 fps | cm, Y-up, 30 fps |
| Geometry | 14 meshes | none | the rig's 6 `CC_Base_*` head meshes |
| Bones | 101 | the same 101, same parents, lengths within 0.0003 cm | none (skins with no clusters) |
| Other nodes | - | 124 `Eve_Ctrl_*` control-rig nodes, 4 hair-LOD transforms | - |
| Blend channels | 2460 | none | 1374, all matching the rig |
| Clips | 0 | 9, 36 to 1140 frames, every node keyed in T/R/S | 69, 2-4 s each, cut from one 243 s timeline |

The facial file's clips are keyed on every frame: about 7 million weight
samples, roughly 110 MB in today's `Key<f32>` form. That is already true when the
file is opened alone, but a session of several VO files adds them up, so a more
compact key store is worth doing later.

## Build order

1. **Animation-only files and skeleton-only mode.** *Done.* Accept an FBX
   with no geometry but a skeleton; a frame rate per clip
   (`AnimationClip::frame_rate`, read by every frame-numbering method); deform
   tables owned by the model slot rather than the mesh, so a skeleton can be
   posed with no mesh; bounds and clip envelopes from the joints when there is
   no mesh; skeleton overlay on by default for such a model. Tested by
   `import/tests/animation_only.rs` (an ASCII FBX written at test time) and
   `a_skeleton_with_no_mesh_is_measured_on_its_joints`. Known wrinkle: opening
   a mesh model next leaves the skeleton overlay on.
2. **Sessions, and the core of binding.** *Done.* A multi-file drop is one
   request (gathered in `about_to_wait`); Open takes several files; File > Add
   Animations (also on the macOS menu bar, which could not be built here);
   Open Recent behaves as a drop; several paths on the command line open as one
   session. Requests carry an intent and are parsed one at a time, each request's
   files in parallel, then classified by the pure `loading/classify.rs`. Binding
   had to come forward from step 3, since "does it bind" is what decides add or
   replace: `review_model::attach` matches by name and re-points clips in place,
   and the Animations tab groups clips by file. Beyond the plan: clip envelopes
   are measured on every core in 128-frame pieces (9 body clips on the Eve rig,
   9.8 s to 0.9 s), and a rigid rig with attached clips builds its deform path
   when the first pose arrives.
3. **The checks and the report data.** *Done.* `review_model::attach::report`
   measures a `BindingReport` as a file attaches: coverage (model bones animated
   of all; channels animated of those on the meshes the file carries), the
   unmatched lists both ways, and the warnings — units, axes, bone offsets,
   copied meshes' vertex counts, unmatched bones — which `ui::binding_warnings`
   turns into one warning card per file. `attach::convert` scales translation
   keys into the model's unit and turns top-level nodes' keys into its axes
   (import now captures each file's declared axes, `ModelStats::source_axes`).
   A bone offset is compared only for a bone under a bone whose keyed
   translation holds still, so root motion is never read as a proportion.
   Tested end to end by `import/tests/attach_checks.rs`: a metre Z-up file on a
   centimetre Y-up rig lifts the root *up* and bends the tip to -X. The Eve pair
   reports nothing, as measured by hand: the body file animates all 101 bones,
   the face file 1374 of the 2460 channels on its six meshes. Alongside, the
   animation curves got an options window (shape, width, opacity, dot size).
4. **Interface.** File groups in the Outliner with marks, the report in the
   Inspector, the Animations tab grouped by file.
5. **Later:** layering a face clip over a body clip; a compact key store.
