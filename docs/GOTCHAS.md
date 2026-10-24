# Gotchas

Things that have already cost someone a day. Each one is a trap that looks like
working code, or a rule whose reason is not visible from the call site.
[ARCHITECTURE.md](ARCHITECTURE.md) has the crate map and the invariants these sit
under.

---

## Contents

- [Testing and CI](#testing-and-ci)
- [Skinning, animation and deformation](#skinning-animation-and-deformation)
- [GPU struct layout and the render passes](#gpu-struct-layout-and-the-render-passes)
- [Import and the FBX libraries](#import-and-the-fbx-libraries)
- [The Opt workspace](#the-opt-workspace)
- [Remesh](#remesh)
- [The Aud workspace](#the-aud-workspace)
- [The chrome](#the-chrome)
- [Loading, undo and the Opt run](#loading-undo-and-the-opt-run)
- [Tools and builds](#tools-and-builds)
- [Baked assets](#baked-assets)
- [The Explorer shell extension](#the-explorer-shell-extension)

---

## Testing and CI

**There is no CI; the gate is `scripts/check.ps1` (or `.sh`).** It runs fmt,
clippy (the workspace, and `review-render` with its `bake` feature, which nothing
else compiles), the tests with `REVIEW_REQUIRE_FIXTURES=1` — so a suite that
cannot find its fixture or vendored tree *fails* rather than printing "skipping"
— and the shader bytecode check. **A real GPU is required for render checks**,
which stay manual.

**`crates/optimize` carries two integration suites over the real fixtures** in
`assets/test_models`, both of which have already caught bugs the synthetic demo
cube could not: `tests/real_models.rs` (welds, LOD targets, per-triangle tag
survival, exclusions) and `tests/export_round_trip.rs`, which checks every written
FBX by **reading it back** through the vendored ufbx reader — the writer's own
return value only proves it did not error. Each test skips itself when its fixture
or a vendored tree is absent. `tests/remesh.rs` and `tests/remesh_fidelity.rs`
are the Remesh pair — see [Remesh](#remesh) for why validity alone is not enough.

**A fixture that leaves the repo takes its tests' meaning with it.** The
proprietary models went in a9c6db3, and every suite that loaded them skipped
quietly — two of the audit's determinism tests ran on nothing at all. Each suite
keeps an `every_fixture_the_suite_loads_exists` list, which
`REVIEW_REQUIRE_FIXTURES=1` turns into a failure. When a fixture goes, re-point
its tests at one that still exercises the same property; a test no remaining
fixture can exercise (a skinned character, today) stays, `#[ignore]`d with the
reason, rather than passing by skipping.

**Render checks can be scripted, headlessly.** `review-snapshot` (see
[ARCHITECTURE](ARCHITECTURE.md#snapshot--review-snapshot)) draws through the
real renderer on a windowless GPU and writes PNGs; `--compare` turns "the
shading looks different" into a number — the per-pixel angle between two
meshes' shading normals from the same cameras — and `--inspect` says what a
simplifier did to the vertices. It still needs a real GPU, so it is a tool to
run by hand, not a test.

**The shaders are validated by the build, not by a test.** `fxc /WX` (or `xcrun
metal`) in `render`'s `build.rs` rejects a broken shader.

---

## Skinning, animation and deformation

### Rest pose ≠ bind pose

`ModelData::vertices` are world-baked in the **bind** pose, but a skinned model is
*displayed* in the file's **default** pose: the palette at rest is
`bone_rest_world × world_to_bone_bind` per cluster, not identity.

Anything that reads the vertex buffer on the CPU — the dimension-label BVH, the AO
bake, `recompute_bounds` — therefore sees the bind pose. `model.bounds` is measured
through `review_model::anim` at import, and the Opt workspace (which shows the bind
pose on purpose) passes no pose.

Every *box* is measured in the pose on screen, never from the buffer: for a model
that deforms, `ModelData::visible_bounds` (Visible Only) goes through
`anim::visible_rest_bounds` and `selection_bounds` (Only Selection, and `F` on a
selection) through `anim::triangle_rest_bounds`. Reading a skinned model's buffer
made the box jump to the bind pose the moment a mesh was hidden or selected. A
selected clip's *envelope* under Visible Only with something hidden is
`anim::visible_clip_bounds_over`, which costs what `clip_bounds` costs and belongs
on a worker; nothing measures it yet, so that one box shows the visible meshes at
rest.

### Every mesh-derived overlay must copy its source corner's `deform` lane

The wireframe, normal lines, heat map and selection highlight deform only because
their builders take the slot's lanes (`geometry::deform::corner_deform`). A
builder that pushes `NO_DEFORM` for mesh geometry silently draws the bind pose
over the skinned mesh.

Static geometry (grid, bounding box, pivot, UV) is `NO_DEFORM`; the skeleton
overlay uses `node_deform`.

### A clip's bounds cost the *animated* part of the scene, not the mesh

`anim::clip_bounds` re-measures only the corners the clip can move — a node it
tracks or a descendant of one, a bone reached through the skin, a vertex a tracked
blend shape has an offset for — and measures everything else once, at rest, which
is the pose it holds in every frame.

Walking the whole mesh per frame instead is what a game FBX punishes hardest. One
2.8M-triangle exterior carries a 2401-frame `Take 001` whose tracks reach 1.5% of
its triangles, and the naive walk spent **97 s of a 102 s load** re-deriving bounds
it already had — the parse itself is 1.6 s. Note that `needs_deform()` is true for
*any* clip, so a static scene with one leftover take takes this path too.

Keep the two halves in step: `corner_moves` mirrors `deform_corner`'s three
sources, and a new deform source added to one must be added to the other, or its
vertices will be measured at rest and the clip's envelope will come out too small.

A skin row whose weights are all zero is not skinned: `deform_corner` has nothing
to normalise by and rides the corner on its node, so `corner_moves` asks whether
the row's summed weight is positive, not whether it has influences — and so does
the GPU lane builder (`geometry::deform`), or the shader would hold that corner at
its bind position. And a corner map that is *present* is not one that *covers*:
`AnimContext` builds its one-corner-per-logical-vertex table only when
`corner_to_logical.len() == vertices.len()`, because an Opt level with a clip but
no skin or morph carries no map yet keeps its logical count, and a table of nothing
but `u32::MAX` measured no mesh at all — the level lost its box.

### A golden-ratio palette keyed on the node table gives a chain one colour

Stepping hues by the golden-ratio conjugate spreads *consecutive* indices well,
and the worst case is the Fibonacci distances: 5, 8 or 13 apart lands within a
few percent of the wheel. ufbx lays a rig out breadth-first, so a chain's bones
sit as many entries apart as there are chains beside it — on the dog fixture the
front-left knee, ankle and foot were 5 and 8 apart and came out the same
yellow-green. The animation curves therefore key each node's colour on its
depth-first rank in the hierarchy (`geometry/trajectory.rs`'s `hierarchy_ranks`),
which puts a bone next to its parent and sends it to the far side of the wheel.
`a_chain_laid_out_breadth_first_gets_distinct_hues` pins it. Any other per-node
palette should do the same.

### `ufbx_bake_anim` facts

- Linear keys are kept as authored; only cubic segments resample.
- Stepped keys become 1 ms pairs.
- `key_time_min` / `key_time_max` can extend **past** the stack range — so frame
  counts derive from the stack range × fps, and the evaluator holds the end values
  outside the keys.
- Blend-channel weights arrive as the `DeformPercent` element property (percent ÷
  100), and ufbx folds the channel weight into per-keyframe *effective* weights
  (`channel_effective_weights` is a verbatim port). Do not multiply the channel
  weight in again.

### "No vertices" does not mean "no model"

An animation file saved apart from its character is a skeleton and its clips, with
no mesh, and it imports as exactly that. So an empty vertex buffer is no longer
the empty start state: ask `ModelData::is_empty()` for that (no geometry *and* no
nodes), and `has_geometry()` for "is there a triangle to draw". Two places tested
`vertices.is_empty()` before this — the double-click-to-open and the fly-in on
replacing a model — and would have opened a file dialog on a click at the
skeleton.

The same file is why the GPU deform tables live on `ModelSlot` beside
`deform_layout`, not inside `MeshBuffers`: a meshless skeleton still poses its
overlay through the palette, and with the tables hung off the mesh it drew frozen
at rest. And it is why `posed_bounds` / `clip_bounds` / `recompute_bounds` fall
back to the bone joints when there is no mesh — they would otherwise measure
nothing and leave the model unframed.

### A clip is timed at its own frame rate, never the model's

`AnimationClip::frame_rate` is the rate the clip was baked at, and every
frame-numbering method reads it; none takes a rate from the caller. A clip can be
played on a model from another file, and the two disagree in practice — the Eve
rig is saved at 24 fps and its animations at 30 — so a caller-supplied rate was a
standing invitation to number one file's clip by the other's. `ModelData::
frame_rate` is still there, but only as the scene setting a re-export declares.

### A drop of several files is several events

winit delivers a multi-file drop as one `DroppedFile` per file, back to back.
Opening each as it arrived made every file supersede the one before it, so a
rig and its animations dropped together loaded only the last of them. The paths
are gathered in `LoadQueue::dropped` and opened as one request from
`about_to_wait`, by which point the whole drop has been delivered.

### Name matching takes exact names first, across the whole file

`Binding` lets each model node be driven by one source node at most, so the
order the source is walked in decides who gets a contested node. Matching in
file order with exact-then-stripped preference *per node* let `Eve:Hip`, earlier
in the file, claim the rig's `Hip` before the exact `Hip` was reached. The exact
pass therefore runs over the whole file before the namespace-stripped one
(`a_model_node_is_driven_by_one_source_node_at_most` pins it).

### A rigid model gets its deform path only once a pose arrives

A model with no skin, no blend shapes and no clips of its own builds no deform
layout — its per-corner lanes would be memory for nothing. Clips attached from
another file can still move it, so `app` evaluates a pose whenever *any* clip is
on offer, and `sync_mesh` builds the layout the first frame a pose is handed over
for such a model, then keeps it for the model's lifetime so a workspace switch
does not rebuild the mesh. The derived views copied the old, empty lanes, so the
same rebuild resets `views_revision` to make them follow.

### A file's unit lives in its root, not in its keys

Import normalizes every file to meters with ufbx's default space conversion,
which puts the scale on the scene **root**'s transform. Every other node's local
translation — rest and keys alike — stays in its own file's unit: the Eve rig's
spine sits `7.37` from its hip, in centimeters. Played on its own that is
invisible, since the root scales it back. Attached to a model in another unit it
is not: the model's root applies the model's factor to the file's numbers, and a
meter file's clip moves a hundredth as far on a centimeter rig. So
`attach::convert` scales every translation key by the file's unit over the
model's, and the bone-offset check reads both sides through their own unit
before comparing. Import does not convert axes at all, so a Z-up file's
top-level nodes are written in Z-up coordinates and their keys are turned into
the model's axes; nodes below them are written in their parent's frame and are
left alone. `crates/import/tests/attach_checks.rs` pins both through the real
importer, with a meter Z-up file played on a centimeter Y-up rig.

---

## GPU struct layout and the render passes

### Field order must match the shader

GPU struct field order must match the shader's uniform-block and vertex-input
layouts (invariant 11). The one shader source is
`crates/render/src/shaders/review.glsl`; update it in lockstep with the
`#[repr(C)]` `SceneUniforms` / `SceneVertex` structs in `scene/gpu_types.rs`, and
mind std140's 16-byte block packing.

Two things catch a mistake: `fxc /WX` in `build.rs` rejects a broken shader, and
each struct's `size_of` assertion against **shdc's generated struct** rejects a
layout that drifted. Re-run `scripts/gen-shaders.ps1` (or `.sh`) after editing the
GLSL, and commit what it writes.

### Storage-buffer structs are invariant-11 territory too

`InfluenceEntry`, `PaletteEntry` and `MorphEntry` in `gpu_types.rs` mirror
`review.glsl`'s storage-buffer entries at bindings 12-15 byte for byte, and the
morph weights upload as bare `f32`s against a one-scalar entry. The palette is
three `vec4` rows, never a `mat3x4`, because matrix packing differs between
uniform blocks (std140) and storage buffers (std430); `MorphEntry` spells its
vectors out as scalars for the same reason, since a `vec3` member would pad the
std430 entry to 48 bytes. Each has size and field-offset assertions against
shdc's reflection.

`scene_vertex_size()` deliberately reports the 64-byte engine-equivalent vertex,
not `SceneVertex`'s 80 bytes — the deform lane is viewer-internal and must not
move the Opt overfetch figure.

### The scene pass is 2-MRT; keep three things in lockstep

The scene geometry pass has **two** colour targets: the scene fragment shaders
write location 0 (linear scene radiance) and location 1 (AO-eligible diffuse
ambient radiance). So every scene pipeline (mesh, line, uv-fill, skybox) declares
*two* colour formats and the offscreen pass binds *two* attachments plus their
MSAA resolves. Keep the three in step.

Both locations alpha-blend. Overlays (zero-normal verts) write 0 to location 1 so
they are not AO-darkened.

GTAO does **not** read location 1: it has its own single-sample mesh-only pass
(`fs_gtao_gbuffer`) into a separate G-buffer target, which avoids MSAA edge
averaging. That pass is itself 2-MRT — location 0 the view normal `xyz` plus view
Z `w`, location 1 the same depth again as a positive distance for the prefilter
chain — so it is a third thing to keep in step. The occlusion is horizon-based
(Intel's XeGTAO), outputs a single scalar (`R16F`), and `post` darkens the diffuse
ambient by it — an *additive correction* over location 0, so MSAA stays correct
and direct and emissive light are never darkened.

The G-buffer pass draws the surface the scene pass drew: the same culling
(`gtao_gbuffer_pipeline` culls back faces, `gtao_gbuffer_double_sided` follows
Backface Rendering) and `fs_main`'s alpha clip (`@block material_alpha`,
`surface_cut_out`), one draw per material range. The 2×+ line pass's depth prepass
runs through the same program with colour writes off. Change the cut-out in one
place and all three follow; draw one of them without it and an alpha-cut leaf
occludes as a solid quad, ringing every leaf with a dark halo. `fs_gtao` takes its
centre depth from `depth_mip0` (`R32F`) like every tap, not from the G-buffer's
half-float `w`, which is coarse enough to move a horizon test (see below).

sokol checks bindings against the shader's *declared* slots: leaving a declared
slot empty fails validation, a bound slot the shader does not declare is ignored,
and applying a uniform block it does not declare fails. That is why the clip
passes can bind the whole material set where the program reads only its opacity.

### A GTAO depth chain cannot live in one image's mips

sokol refuses to bind an image as a texture in the same pass that attaches it
(`VALIDATE_ABND_TEXTURE_BINDING_VS_COLOR_ATTACHMENT`), and it compares **image
ids**, not subresources. So a prefilter chain built a level at a time — pass *k*
reading mip *k-1* and writing mip *k* of one image — fails validation in a debug
build and silently unbinds the source in a release one, which reads as black.

The chain is therefore five separate single-mip `R32F` targets
(`TargetSet::gtao_depth_mips`), and `fs_gtao` binds all five and picks one per tap
with an integer compare. The bake's `CubeTarget` looks like a counter-example but
is not: each IBL convolution pass reads a *different* image from the one it writes.

`R32F` rather than `R16F` because the chain holds view depth in metres, where a
half-float's steps are ~8 mm at 10 m — enough to move a horizon test on a contact
crease. The occlusion buffers it feeds are `R16F`, which is ample for a 0..1 term.

### The AO radius comes from the view, never from the model's size

Ambient occlusion shades a crease between two surfaces or the contact under a chair
leg. That is a *local* effect, so its radius belongs to the scale of detail being
looked at — which the model's bounding sphere does not describe. Keying it there
meant a room interior asked for a four-metre radius: the fixed step count spread
across half the screen, the near geometry that makes contact shadows was never
sampled, and the effect all but vanished exactly on the scenes that need it.

`GtaoSettings::effective_radius` takes a fraction of `OrbitCamera::view_extent`
instead, capped at half the bounding sphere so a pulled-back camera cannot ask for
more than the object. Two consequences worth knowing:

- The radius in **pixels** is then the view fraction times the viewport height, at
  any scene scale and any zoom. The march always spans the same screen distance, so
  its step count always resolves it. Anything else scale-dependent should be
  expressed in pixels or in `view_pixel_size` for the same reason.
- The occlusion changes as the camera dollies, because the world radius does. That
  is deliberate: it is what holds the *on-screen* size of the shading constant, which
  is what the eye judges. The accumulation resets on any camera move anyway, so there
  is no stale-history artifact.

`OrbitCamera::view_extent` is one expression for both projections, because the
orthographic half-height is defined as the same `distance * tan(fov/2)`.

### Ambient occlusion converges while the view is still, and the term that drives
### it must be read after the render

GTAO's single-frame estimate is noisy at any affordable sample count, so whenever
nothing that affects it changes, each frame is folded into a running mean over 24
frames and the passes then stop entirely (`scene/ao_accum.rs`). `Renderer::
is_ao_converging` is what keeps `app` pacing those frames.

It is deliberately **not** in the continuous-redraw disjunction in
`app/src/frame.rs`, which is evaluated *before* the render: there it would read the
state from before this frame reset the average, and a one-off redraw — the frame
after a slider tick, say — would never schedule the follow-up that converges it.
It sets `redraw.requested` after `render_scene` instead, which is the same
coalescing path input events take and is paced identically.

What is in the reset key matters as much as what is not. The selection highlight
colour never touches the G-buffer, so including it would restart the average for
nothing; the MSAA level does not either, since the
GTAO targets are single-sample by design and an AA change does not recreate them.
The hidden-mesh set reaches the key as `ModelSlot::visibility_generation` — a
counter bumped when the visibility list is rebuilt — because the key is compared
every frame and must not walk a `Vec`.

**A view that runs no AO must say so.** Every render entry point
(`SceneGpu::render`, `render_opt`, `render_uv`, `render_aud_split`,
`Renderer::render_texture`) first idles both target sets' averages
(`SceneGpu::idle_ao_history`); only `sync_ao_history` re-activates a set, and only
on a frame that runs GTAO into it. Without that the UV and Tex viewports, which
never advance the average, left `is_ao_converging` answering for the 3D view they
replaced, and the viewer redrew at the refresh rate until the user went back. A
resized `TargetSet` is recreated whole, accumulator included, and the overdraw
views are outside `gtao_active`: they skip the passes, and counting their frames
as samples "converged" the average on frames that wrote nothing.

Tone mapping and sRGB encoding happen once in `post`, not in the scene shader.

### fxc notes

- Use `SampleLevel` (not `Sample`) for any texture read inside a loop or branch —
  non-uniform control flow.
- Guard a possibly-negative `pow` base with `max(x, 0.0)` so `/WX` does not reject
  it.

### Derived views: follow the ensure/free pattern

CPU-side vertex generation (grid, wireframe, face and vertex normal lines) lives
in `crates/render/src/geometry/`, one file per category; `scene/slot.rs`
(`ModelSlot` / `DerivedViews`) owns the per-model GPU state and its bake keys.

Derived line views are built-on-demand and freed-on-off by
`SceneGpu::sync_line_views` in `scene/line_views.rs` (invariant 3): a view's
buffer exists only while its toggle is on, and is rebuilt live when its baked
length or colour drifts. Add new debug views by following that pattern, and give
every one a free arm on *each* path that leaves its view — including a switch to
another workspace, which runs a different `render_*` entry point that never
calls the view's `sync_*` at all. `SceneGpu::release_opt_views` and
`release_uv_views` are those arms for the Opt-only and UV-only resources. The Aud
split draws through the second target set itself, so it calls only the
mesh-derived half, `release_opt_derived`.

A `sync_*` must compare against the **borrowed** frame inputs and only `to_vec()`
its bake key on the rebuild path, so a steady-state frame allocates nothing.

Geometry builders that filter hidden nodes go through `geometry::HiddenFilter`
rather than rebuilding the set-plus-length-guard by hand — the guard is what keeps
an out-of-range index out of `model.triangles.node`.

### Keep `model` and the camera/debug math host-agnostic

So that a future renderer swap only touches `render`.

### Framing fits what the user can see, not the image the camera draws

A single view is drawn across the whole window with the chrome painted over it,
so a framed model has to land in the window *less* the toolbar, status bar,
Outliner and Inspector — an area that need not be centred, since the two side
panels are sized independently. The Opt and Aud splits are the opposite case:
each view is an image of its own and nothing covers it, but it is half as wide,
while `app` keeps the main camera's aspect at the window's and the renderer
corrects it only at draw time. Fitting at the window's shape with a height-only
"safe area" — what `F` used to do — put a wide model's sides under the side
panels in 3D and cut a split view down to its middle.

`app`'s `sync_framing_area` hands the renderer a `FramingArea` (the image's
shape and the visible fraction of it) **every frame**, after the egui pass has
measured the chrome, because panels are resized and closed at any time.
`OrbitCamera::framed_to_bounds` fits the box's corners into it exactly: a
closed-form distance per axis and a bisected centring pan. It replaced a
six-pass iteration whose last pan move came after its last distance fit, so the
pair it returned had never been checked together.
`a_framed_box_lands_inside_the_visible_area` pins it over 3,600 framings.

Centring in an off-centre area pans the pivot off the model's centre, so an
orbit after `F` swings the model a little. With both panels at their default
width the area is centred and nothing changes. Keeping the pivot on the model
would take an off-axis projection, which GTAO's view-space reconstruction, the
pick ray and the label projection would all have to learn.

The fit is per projection. Orthographic sees every corner at its size at the
pivot, and perspective binds on the near ones, so a perspective fit seen
orthographically overflowed by up to a third. Holding both at once costs
perspective up to a quarter of its fill, so each is fit for itself, and a
projection toggle may want a second `F`.

Orthographic's view size is tied to the distance, so a zoom walks the orbit eye
towards the pivot and, on anything deeper than the view is tall, into the model —
it used to cut a facade open when the user zoomed in on a window. The orthographic
picture does not depend on where its eye sits along the axis, so it is drawn from
its own eye (`OrbitCamera::eye_for`), backed off until the framed content's sphere
(`scene_center`, `scene_radius` widened to the grid) starts past the near plane,
with the far plane stretched to its back (`near_far_for`): no zoom, pan, orbit or
flight clips it (`an_orthographic_view_never_clips_the_model`). **Anything that
works in orthographic view space** — the scene uniforms' eye and view matrix, the
pick span, the dimension labels' occlusion ray — **must use `eye_for` /
`view_matrix_for` / `near_far_for`**, not `eye_position` / `view_matrix` /
`near_far`, which are the orbit eye's and perspective's. Framing still holds the
distance where no orbit can swing the model through the orbit eye's near plane,
because a projection toggle keeps the distance and hands that eye to perspective;
that clearance is measured round the *pivot*, which the centring pan has moved
(`orthographic_clearance`, pinned by `an_orthographic_framing_survives_an_orbit`).

### A texture past the device's 2D limit is uploaded from its first mip that fits

The decoders accept 30000 px, but D3D11 and Metal stop at 16384. A refused
upload used to fail every frame's material sync — the 3D view stopped drawing —
and rebuild the whole CPU mip chain on each redraw. `Texture::rgba8_mipped_or_white`
drops the levels above `max_image_size_2d` and makes the first that fits the
base, and a refused upload still caches its fallback against the decoded image
so it is not retried every frame.

---

## Import and the FBX libraries

### `ufbx_write`'s `ufbxw_view_*` buffers are borrowed, not copied

The pointer must stay valid until the save completes. `export_bridge.c` uses
`ufbxw_copy_*` throughout so nothing is borrowed past the call; reusing one
scratch buffer across meshes with a `view` was an access violation.

### FBX declares its unit rather than fixing one

Import normalizes every file to meters, so the writer declares the **source
file's own** unit (`UnitScale`: 1 for a centimeter file, 2.54 for inches, 100 for
meters) and expresses the coordinates in it. Declare one without scaling the
coordinates, or the reverse, and the geometry reads back off by their ratio,
which no error reports.

The matching coordinate scale is applied **once, at the top of the hierarchy**:
`place_node` builds the chain against a root frame of `1/per_meter` instead of
the identity. Applying it per *local* value double-counts, because import parks
the unit normalization in the node transforms themselves — so the node inverse
`build_mesh` runs the geometry through already returns the source's unit. That
double-count cancelled in the positions (a 100× subtree under a 100× too-small
node scale) and passed every bounds round trip, but it left the reimported
`geometry_to_world` at 1e-4 and the cofactor matrix ufbx derives normals from at
1e-8 — under the reader's epsilon, so every normal fell back to a constant
`(0, 1, 0)` and the exported mesh came back lit but featureless.

### A normal transposes the forward matrix, not the inverse

`build_mesh` sends positions through `node.transform.inverse()`, so the normal
matrix is the transpose of `node.transform` itself. `inverse().transpose()` is
the map for the opposite direction and rotates every normal the wrong way round.
The two agree for a pure uniform scale, so it hides until a rotated node is
exported.

### ufbx_write seeds every scene with a "Take 001" animation stack

`ufbxw_create_scene` creates a default `AnimationStack` named `Take 001` and its
`BaseLayer` unless `no_default_anim_stack` / `no_default_anim_layer` are set — so
a static mesh came back out of the exporter carrying an empty animation, listed
in the Outliner's Animations tab and shown as a take by every DCC.

### A processed level keeps its deform rows only because the weld key carries a row id

`Submesh` holds the skin / extra-skin / dual-quaternion / morph rows per vertex
(`VertexRows`, a CSR), and a vertex remap *gathers* them from a representative
old vertex — a scatter would let the last writer win. That is exact only if every
vertex merged into a slot carried identical rows, so `ops::weld` (and the index
pass) folds one `u32` row id — the dedup of the concatenated row bytes — into the
exact key and into the tolerance predicate, and `filter_triangles` feeds it as a
second stream to `meshopt_filterIndexBufferMulti`. Drop the id and a 50 % LOD
silently re-skins vertices to a neighbour's bones.

### Polygons ride the pipeline as a carry, and each operation class treats it one way

`PolygonCarry` maps faces and edges onto local vertices. A vertex remap rewrites
its corners; an operation that keeps triangles whole (filter, prune, the
cache/overdraw reorders) reconciles it by canonical triangle content, dropping
faces that lost a triangle; a simplify clears it, and the export writes that
level as triangles — with a note naming the operation only when the source part
actually had quads or n-gons, since most game assets are triangles already and
lose nothing. The processed `ModelData`
still carries no `faces` / `to_face` — the renderer assumes the corner-split
layout — so the polygons live in `ProcessedLod::carry`, which only the exporter
reads. `face_group` there is the group *id* (`face_groups[i].id`), not ufbx's
table index, which is what `ufbx_face.face_group` holds.

### A skin cluster's `Transform` is `mesh_node_to_bone` verbatim; only `TransformLink` is in scene units

ufbx reads `Transform` as-is and root-transforms `TransformLink`
(`bind_to_world`, so import holds it in meters), which is why the export scales
`bind_to_world`'s translation by `per_meter` and leaves `Transform` alone. A bind
pose's `bone_to_world` rows scale the same way. Getting either wrong passes every
bounds round trip — the skinned pose is what breaks — so
`skin_survives_with_its_authored_cluster_matrices` compares the reimported
matrices numerically.

### Blend-shape normal deltas ride the scale-normalized normal matrix, and go out unnormalized

The base normal is renormalized after its transform and the delta is then added
to it, so a delta must ride `L⁻ᵀ` divided by the base normal's scale. The bridge
uses `|det L|^(1/3) L⁻ᵀ` (`review_import_normal_delta_matrix`), exact for a
uniform scale. `ufbx_matrix_for_normals` alone is `|det L| L⁻ᵀ`, which scales a
delta by `s²`: since import parks a centimeter file's 0.01 in the node
transforms, every blend shape of a Maya-default rig moved its vertices but never
changed its shading. The export inverts it with `Lᵀ / |det L|^(1/3)` and must not
normalize the result — a delta is a difference of normals, and its length is the
shape's.

### Tangents ride the positions' map, and a mirroring node flips their handedness

A tangent lies in the surface, so it transforms like a position difference —
through `L`, not the normal matrix, which turns it off the UV direction under a
non-uniform scale and does not round-trip. Handedness lives between spaces too:
under a node whose `det` is negative, `w` flips between world and node space, so
import gives a tangent layer without binormals `w = sign(det)` and the export
writes `w × sign(det)`.

### A captured material is written with exactly its authored properties

`rvo_export_material::authored` says the material came from the capture, and then
the bridge writes the properties it declared and nothing else — never the
viewer's own figures, which Rust zeroes for a captured material. Falling back to
them wrote a material with no explicit properties as black with a
`ShininessExponent` of 0 (fully rough), and on a custom shading model, whose
template has no `DiffuseColor`, failed the whole export. A single-level export
(one file per LOD) likewise puts its level on the source node itself, under its
own name; level copies exist only in a chain.

### Every property is written before any curve binds to it, and a curve node gets only the curves the source had

ufbx_write's `animate_prop` needs the target property to exist, so the bridge
writes nodes / attributes / materials / textures / channels / layers first and
the animation last (`RVO_TARGET_*` resolves each `ElementRef` to an export id).
Its `d|X/Y/Z` defaults are always written, but a component the source keyed
nothing on gets no `AnimationCurve` (patch P5, `ufbxw_animate_prop_masked`): a
reader treats *any* curve, even an empty one, as a non-constant value — ufbx then
adds a scale helper under every bone whose `Lcl Scaling` Maya wrote as a
default-only curve node, and the dog fixture came back with 42 nodes the source
did not have. Deleting the unwanted curves after the fact is not an option:
`ufbxw_delete_element` leaves the curve node's connection list pointing at the
freed slot, which the next element reuses.

### Keys at the same ktime are not merged, and the first key's left tangent cannot round-trip

ufbx_write sorts out-of-order keys but keeps equal ktimes
(`round(t × KTIME_SECOND)`), so `curve_data` skips a key equal to the previous
one. Tangents convert as `slope = dy/dx`, `weight = dx / interval` (left: the
previous interval, right: the next) — the exact inverse of ufbx's
`dx = weight × interval`, `dy = dx × slope` — but a first key has no previous
interval, so its left handle is dropped by the reader; the round-trip test skips
it, and nothing evaluates it.

### ufbx_write elides a property equal to its template default, and that loses authored zeros

A source `Lcl Translation` of `(0, 0, 0)` or a `ReflectionFactor` of `0` is
authored data the reader would otherwise fall back to the template for. Patch
P1's `UFBXW_PROP_FLAG_EXPLICIT`, set on every property the bridge writes, forces
it out. The elision is upstream's design — don't remove the flag to "clean up"
the file.

### `ReflectionFactor` is not metalness

Import takes metalness from `pbr.metalness` alone. A classic Lambert or Phong
material declares none, and its `ReflectionFactor` is Phong reflectivity — a slot
DCCs fill with a non-zero default nobody authored (Maya 0.5, the FBX SDK 1.0).
Reading it as metalness made nearly every real game asset arrive half or fully
metal, and in the IBL path a metal has no diffuse term, so stone, wood and skin
became mirrors of the environment. ufbx *does* map that slot onto metalness for
`UFBX_SHADER_BLENDER_PHONG`, where Blender genuinely writes it there, and that
arrives through `pbr.metalness` anyway. The export's Phong fallback still writes
the viewer's metallic to `ReflectionFactor` (the closest slot Phong has); that is
deliberately **one-way**, so do not "restore symmetry" by reading it back.
`a_classic_material_imports_as_a_dielectric` pins it.

### A classic material's shininess texture is a gloss map

ufbx reads a Lambert or Phong material's `ShininessExponent` into `pbr.roughness`,
converting the *value* from a specular exponent. The *texture* connected there is
what Maya's cosine power and 3ds Max's Glossiness map export as — brighter is
shinier — so binding it as a roughness map inverted it and turned every such
asset into polished plastic (measured on a Max character: every gloss map read as
mirror-smooth). The bridge records that texture as `MaterialMap::Glossiness`,
which binds into the roughness slot with the material switched to the smoothness
workflow. **Blender is the exception**: its exporter writes the *roughness*
texture into that same slot (its own source says "will be inverted in fact"), and
ufbx reports which tool wrote the file, so a Blender file keeps it as roughness.

### A connected texture replaces the color it is connected to

FBX stores a color property's value even when a texture drives it, and the value
is whatever the color was before the connection: Maya's 0.5 grey, often black for
emission. Every DCC shows the texture in its place, so binding an imported base
color or emission map sets the factor it is multiplied by to white (emission: the
material's emission *strength*, `MaterialImportDefaults::emission_factor`,
carried for exactly this). Multiplying by the stored value showed every textured
asset half as bright as its DCC did. A texture assigned by hand in the viewer
keeps the color the user set — that one *is* an edit.

### UV and vertex-colour layers go out `ByPolygonVertex` + `IndexToDirect`

ufbx_write's plain `ufbxw_mesh_set_uvs` / `set_colors` ask it to generate
indices, which dedups the values and emits an index array *as long as the value
array* — so vertex-mapped values become `ByVertice` + `IndexToDirect` with a
per-control-point index. That is legal FBX that no reader assuming an indexed
UV or colour layer is per polygon vertex can load: Unity rejects the mesh with
"has invalid UV coordinates" / "invalid vertex Colors" and blames the exporting
tool. `export_bridge.c` therefore uses the `*_indexed` setters with the mesh's own
index buffer. Normals are unaffected — their layer forbids indices, so they stay
vertex-mapped and `Direct`.

### Recovering from a malformed index drops the whole triangle, never one corner

An index buffer is a flat corner stream, so `continue`-ing past a single bad
corner leaves a length that is no longer a multiple of three and shifts every
later triangle by one — plausible-looking garbage rather than a visible failure,
and trimming the tail afterwards does not undo it.

Both `submesh::partition` and `export::build_mesh` check all three corners first
and skip the triangle as a unit. `export` additionally consults the same check
from its per-face material loop, since skipping in one loop and not the other
mis-assigns every subsequent material.

### One bad key, shape or binding must not cost the artist the model

The bridge copies animation keys, blend-shape values, bind matrices and stack
ranges as the file has them, and `validate_deform` — the funnel's last guard —
rejects any non-finite value or reversed range. Left at that, one NaN key failed
the whole load. `import::repair` runs just before the guard and drops only the bad
piece (a key, a channel, a shape's offsets, a cluster) or plays an unusable clip
range over its keys, renumbering nothing, and says what it did:
`PendingExtras::warnings`, which the viewer shows as one card per file and the
CLI as warnings. An optional capture layer that cannot be read (a second colour
set, a vertex crease) is skipped the same way rather than dropping the whole
capture, and each embedded image is stored once however many textures share it.

---

## The Opt workspace

### Every run begins with a lossless index pass, and must

Import splits each face corner into its own vertex, so a mesh reaches `optimize`
with *no* shared vertices at all — a real 10,006-triangle asset arrives as 30,018
vertices. Every meshoptimizer operation works through the index buffer, so on that
mesh they are all no-ops: nothing for the vertex cache to reuse, no edge a collapse
may cross, a LOD chain that removes nothing.

`process::index_mesh` therefore merges vertices identical in *every* attribute
before any stack operation runs. They are byte-for-byte survivors, so nothing
visible changes — 30,018 → 5,284 on that asset, after which a 50% LOD target is hit
exactly. It is deliberately **not** a stack operation; skipping it is never useful.
The `Weld` operation is for the *lossy* merges (dropping normals or UVs from the
comparison, or a tolerance).

"Every attribute" means every one a vertex carries (`ops::index_losslessly`):
every UV and colour set, the crease, the deform row, and an *authored* tangent
including `w`, which is what keeps a mirror seam apart — merged, one side exports
the other's basis. A tangent the import synthesized is compared by the sign of `w`
alone: it is built on the corner-split buffer, so its direction is faceted per
polygon, and comparing that indexes nothing (30,018 of 30,018 kept on
`SM_column04`). The sign is not faceted — it flips only across a mirror seam — and
an engine splits there either way. Excluded objects are indexed too; left
corner-split they were counted per face corner and exported as polygon soup.

**The run's baseline (`ProcessedResult::source` / `source_metrics`) is measured
after that pass.** Quoting changes against the corner-split buffer credited the
user's operations with an "-82%" any engine cooker gets for free, and made the
baseline ACMR a meaningless 3.0. The indexed baseline also equals the stats panel's
`GPU Verts`, and a `real_models` test pins the two figures equal — which is why
`ModelData::mesh_group_stats` keys on the same tangent sign the pass does.

### A simplify that barely removes anything is usually attribute seams, not a bug

Import splits every face corner into its own vertex, so a mesh whose normals or
UVs differ at every corner — a scan with generated per-face normals — presents
*every* edge as a discontinuity, and a topology-preserving collapse cannot cross
one.

Measured: a real game asset welds 369k → 108k vertices and hits its LOD targets
with the default settings, while such a scan stalls at 249,882 → 249,880 triangles
until either a position weld (normals excluded) or the permissive flag unblocks
it. `process` detects the stall and says so.

The default weld comparing normals is *correct* for the tool's stated target of
static game meshes, so do not "fix" it by loosening the defaults.

### A simplifier's survivors must be refit, or a flat face shades in blotches

A collapse-based simplify keeps some of the source's vertices and drops the
rest — and a kept vertex's normal describes the few small faces it had at full
detail. Game assets soften their edges, so the vertices along a bevel lean into
it. Collapse the middle of a flat face away and the triangles left stretch from
bevel to bevel; with their old normals the lean is interpolated across the whole
face, and it reads as dark and light patches. That was the Standard simplifier,
the default until it was measured against a commercial tool's output on the
same budgets: on an arch whose stone faces are bevelled panels, 11.7 degrees of
mean shading error at a quarter of the triangles against the other tool's 1.4.
`--inspect` showed what the other tool does differently — it *moves* about half
its vertices off the source positions, and re-derives the normals even where it
keeps a position.

`WithAttributes` now does both through `meshopt_simplifyWithUpdate`, and it is
the default. Measured with `review-snapshot --compare` over ten assets, eight
views, three levels each, at equal triangle counts: survivors kept as they were,
11.5 degrees; attributes weighed but nothing moved, 6.0; weighed and refit, 4.7,
with the silhouette tighter too. **Refitting is most of it** — refit alone, with
normals weighed at almost nothing, already gives 5.9. An attribute left at weight
zero is therefore still refit (`REFIT_WEIGHT`): meshoptimizer neither weighs nor
rewrites a zero-weighted attribute, and a vertex that moves without its UVs drags
the texture with it. The refit value does not depend on the weight, so a tiny one
is exact. The default normal weight (0.25) is where the measured trade between
shading and silhouette sits best; weighing normals harder (1.0) shades slightly
better and loses silhouette. **Do not "simplify" back to keeping survivors as
they were** — it is the bug.

Only a vertex inside a smooth surface or at a junction of seams moves; one on an
open border or a two-sided seam stays put, and a vertex that moves takes every
vertex at its position with it, bit for bit. That is what keeps UV seams, hard
edges and material borders closed.

### A simplify budget is per object, never per material

`Submesh` is a (node, material) piece, and the simplifier used to be handed one
piece at a time with the ratio applied to each. That spends triangles by
material rather than by shape: on the arch, 91 % of the triangles are a dense
decorative trim and 9 % the stone faces, and a quarter of each left the trim
with detail nobody could see and cut each stone's curved outline to a chord — the
silhouette visibly faceted where the other tool's stayed round. It also opened
the material borders: each side of a border collapsed its shared edge on its
own, leaving T-junctions (`a_material_border_does_not_open` fails four edges
open on the old path). `ops::simplify_object` simplifies a node's pieces as one
mesh, so the border is an attribute seam both sides keep and the budget goes
wherever the error is lowest across the object. The ratio is still per *object*
— a scene's objects do not trade triangles with each other, so none vanishes to
pay for another. The sloppy simplifier stays per piece, since it clusters
vertices without regard to which piece they came from.

**Simplifying across seams moves corners between materials.** Under the
permissive flag a material junction is a complex vertex, and meshoptimizer
remaps its wedges onto another piece's. `ops::deal_triangles` gives each output
triangle to the piece most of its corners belong to, matches a stray corner to
that piece's own vertex by its position *before* the simplify (an unreferenced
twin keeps its old position), and borrows a copy where the piece has no vertex
there at all. Dealing by first corner dropped 47 of 384 triangles on a
three-material patchwork. Without "Collapse across seams" meshoptimizer locks
every vertex where materials meet, so this never arises.

### The error limit bounds the shape; attribute error must not spend it

Inside meshoptimizer the attribute error rides in the same figure as the
position error, so a single attribute-aware run stops as soon as the *shading*
has drifted the limit's worth, however little the surface moved. On the
fixtures that held most levels far short of their targets — a column stayed at
8430 triangles for every level of a chain asking for 5000, 2500 and 1250. The
limit is documented as a limit on how far the surface moves, so
`ops::simplify_appearance` runs a position-only pass first to find how far the
limit allows, then the attribute-aware pass to exactly that count with no limit
of its own, and reports the position-only pass's error. With pruning on, the
loose pieces that first pass removed are dropped before the second, which then
prunes nothing — a second pass with no limit would otherwise be free to prune
anything to reach its count.

### Re-keying operation ids is a two-pass job

`OptStack::reassign_ids` builds the whole old→new map before rewriting a single
per-node override. Rewriting them as the walk consumes the id space lets an
already-rewritten override collide with a later operation's *old* id and be
rewritten twice, silently reattaching the user's per-object settings to the wrong
operation — reachable with one reorder plus a preset round-trip.

### meshoptimizer's `assert`s abort the process in every build profile

Nothing defines `NDEBUG` for the vendored tree, so upstream's C `assert`s are
compiled into release builds too, and a failed one aborts — it is not a Rust
panic and nothing catches it. Every wrapper in `crates/optimize/src/meshopt/`
therefore rejects in Rust whatever upstream asserts on before the call: whole
triangles and in-range indices, exact stream lengths, a voxel resolution in
`4..=256`, a crease angle in `[0, π]`, non-negative smoothing, and only the
documented option bits (`meshopt_remesh` keeps a private debug bit at `1 << 30`).
Each `// SAFETY:` comment names the asserts it satisfies. Do the same for any new
binding, and extend `MESHOPT_BOUND` in `build.rs`, which is what turns a changed
upstream signature into a build error rather than an ABI mismatch.

### Normals and tangents are per corner; the split happens on the pieces

meshoptimizer's generators answer per index-buffer *corner*, and two corners of
one vertex can legitimately disagree (either side of a hard edge, either side of
a mirrored-UV seam). `Submesh::split_by_corner` is the one place that turns that
into vertices: it copies every per-vertex array and deform row, makes a carried
polygon's corners at one vertex agree first (so a quad never needs two copies of
a vertex), and fans each carried edge out to the copies its faces now use. It
runs on the submeshes, before `assemble`, because splitting the assembled model
would break the corner-run layout and every carry that indexes its vertices.

Two consequences:

- **The tangent rebuild must run last.** `ops::weld` never compares tangents, so
  a weld after `shading::finish_bases` would merge the mirror-seam copies back.
- **Those copies are real vertices** and the stats count them — an engine needs
  them too. On a real character they add about 0.2%. A weld that ignores UVs
  leaves each survivor with an arbitrary UV, so the layout can fold and the split
  count grows; tests about such a weld count distinct *positions*.

### An overlay projects into the rect its camera draws into, and the split has two

`ui`'s `dimensions.rs` maps clip space onto a view's `image` rect and clamps a
label to its `clamp` rect. For a single view those are the whole window (the
scene is drawn over the full backbuffer and the chrome paints on top) and the
chrome-free viewport. The Opt split is two views over *halves* of the chrome-free
viewport, each with its own mesh, camera and measured box — and each camera's
aspect ratio has to be re-derived from its half, because `app` sets it from the
whole window and `render_opt` overrides it on its side; project through the
uncorrected one and every label drifts sideways from its box.
`ui::overlay::split_halves` is the single source of that geometry, shared with
the divider, the picking and the pointer routing. The split lays out both halves
*before* any run lands (the renderer draws the source into both), so the second
view exists whenever the workspace does; keying it on "a processed level exists"
mis-projects the common empty-stack case.

### A target either half of the split samples later belongs to its `TargetSet`

Each half's composite — and the overdraw views' ramp — is a deferred
`SwapchainJob`, so both halves' offscreen passes have been recorded before
either job runs. Anything a job samples must therefore exist once per view. The
overdraw count used to sit beside the count pipelines, one for the renderer;
the split then counted the source into it, counted the processed mesh over it,
and both ramps showed the second count. It is now `TargetSet::overdraw_count`
(built by `sync_overdraw` for the primary set and `sync_split_targets` for the
second), and the triangle list it is drawn from is per slot (`DerivedViews::
overdraw_indices`), so alternating the slots within a frame rebuilds nothing.
The pipelines stay shared: a draw issued is a draw done.

### A processed level's heat map blends from face to face

`heat_vertices` writes a colour per render vertex. Import gives every face
corner its own vertex, so a face is one flat colour; an Opt level is indexed
and shares a vertex between faces, so it cannot be. Each vertex takes the mean
of its measured faces' ramp positions — the same value as before on an imported
mesh, where a corner has one face — rather than whichever triangle was walked
last, which coloured a shared vertex by an arbitrary neighbour.

### The UV-seam view finds a shared edge two ways, and both are exact

A seam is an edge whose faces disagree about the UVs, so the question is which
render corners are the same point. An imported mesh answers from
`corner_to_logical` — free, and the only workable answer, since import
corner-splits every face corner and comparing vertex indices would find no shared
edges at all. An Opt-processed level carries no such map, so
`review_model::topology::welded_logical_ids` welds by **(owning node, exact position
bits)** instead. Exact is correct, not a shortcut: outside Remesh and Shrinkwrap,
which rebuild whole objects, the only operation that synthesizes a position is
the attribute-aware simplify, and it moves a vertex together with every vertex
at the same position, writing the same bits to each — every other operation
drops vertices or copies whole ones — so the copies of a processed point still
share their position bit for bit. The node is in the key because reassembly
concatenates per-object buffers, and a bare position weld would fuse two objects
that merely touch and hide the border each really has.

### A voxel shell is closed, but not always manifold

Shrinkwrap's Voxel method (`meshopt_remesh`) keeps a thin sheet as two
coincident, opposite-facing surfaces over the same welded vertices, so one edge
can be used twice in each direction. Its closedness is *balanced* directed edges
(each matched by as many reversed ones), not one edge one twin — the distance
field's test would reject a correct result. The same sheets z-fight when
back-face rendering is on, and are why the method defaults to generated normals:
projection would read one side's shading onto both.

## Remesh

`crates/optimize/src/remesh/` partitions the surface into even regions and
collapses each to one vertex (see [ARCHITECTURE.md](ARCHITECTURE.md#settled-decisions)
for why it replaced the vendored engines). Most of what follows was found by
measuring, and the numbers are what the change measured at the time it was made.
`tests/remesh_fidelity.rs` re-measures them; run it with `--ignored --nocapture`
in release before and after any change to a stage. Its `how_each_setting_holds_up`
prints area, folds, self-crossing, spread, budget and crease length per setting
over the fixtures, which is the table to compare. (Its crease length sums over a
`HashMap`, so the last digit wobbles between runs.)

### A rebuild can be valid and the wrong shape, and validity is all most tests check

Closed where the source was closed, wound one way, within its face budget — a
strip of bark narrowed until a gap opens against the trunk passes all of it.
`tests/remesh_fidelity.rs` is the other half: per object, the two-sided distance
between source and rebuild in units of the target face size `h`, the same
restricted to *rim* vertices, and the share of surface area kept. Area is the
bluntest measure and the hardest to argue with. Rim distance is the most
sensitive, because a silhouette is a few hundred vertices out of fifteen thousand
and can move a long way before a percentile over all of them notices.

**A fold inflates surface area**, so an area ratio scores a self-intersecting
rebuild *above* 100% and reads the damage as better than perfect.
`a_rebuild_does_not_fold_through_itself` is what catches that, and it asserts the
fixture's own source is clean first: most real game assets self-intersect as
authored (a crystal cluster 11.4%, an ammo crate 15.7%, a tree branch 9.2% of
triangles), and a rebuild carries that through rather than causing it.

### A rebuilt mesh is even in density and unstructured in layout — not a bug

Vertices land at an even spacing and are triangulated as that gives; a field
extraction laid them on a lattice. An unstructured triangulation of perfectly
spaced points has real area variance in it — 2.8x P90/P10 on edge length against
the old engines' 1.4x, measured at the switch — and it is *not* a convergence
problem: twenty-four relaxation passes plateau and forty Lloyd iterations change
nothing. Aligning the layout to a cross field is the fix on paper and **does not
work on triangles**: a cross describes a square lattice and a triangulation wants
a hexagonal one. Four ways of spending the field on the triangle output each
measured neutral or worse (`remesh/cross_field.rs`'s module doc has all four). It
is spent on the quad merge instead, where the output genuinely is four-valent.

### A collapse cannot add faces

Asking for more faces than the input holds returns about the input, with a
warning (`a_budget_above_the_input_says_so`). The engines could subdivide; this
cannot, and that is the trade for never tearing a mesh.

### The rim of a thin shell is a feature, whatever **Keep sharp edges** says

A leaf, a strip of bark, a sheet of cloth is two sheets meeting at a fold, and
that fold is the silhouette. `features::is_feature` treats a dihedral past
`FOLD_COSINE` (a hair past a right angle) as a feature regardless of the crease
setting. The threshold was measured, not chosen: every value from 30 to 100
degrees held the bark's area within a point or two and it fell away past 110
(93.9%). The margin past square is there so a box modelled at exactly 90 degrees
still rebuilds smooth. Without the rule the bark kept 88% of its area; with it,
97%.

**A rim is usually bevelled**, so a single edge's dihedral will not find it — but
a chain of them will. On the palm fixture only 0.5% of edges turn past 120
degrees; the leaves turn their full 180 over two or three rings at about 60 each.
That is why the threshold sits near a right angle rather than near a fold-back,
and why `how_sharply_the_source_folds` exists: run it before assuming a per-edge
test can see a feature at all.

### A sliver's normal is rounding error and will invent features

A cross product of two nearly parallel edges points anywhere, and a pair of them
reads as folded. A feature costs a seed and the seeds *are* the vertex budget, so
a coarse sphere whose poles each read as a ring of folds came back 42% denser
than asked. `features::is_sliver` refuses a normal from a face whose area is
below `SLIVER_QUALITY` times its longest edge squared. (The sphere's case comes
from `sin(PI)` being 8.7e-8 rather than 0; real exporters leave worse.)

### A corner in the middle of a chain is invisible to a degree count

A leaf's tip sits on a border that runs unbroken all the way round, so it has two
feature partners like every other vertex and is never a junction; stations
spaced by arc length land either side and the tip comes back as a chord.
`features::mark_corners` measures the turn across a window of half a target edge
either side — not between one edge and the next, which on a dense input is a
degree of noise — and flags only the sharpest vertex of each bend, because two
adjacent junctions cannot collapse into one another and sit there spending
budget. **Both walks step outward and stop**, so the cost is the number of input
vertices one new edge spans; scanning the chain per vertex instead is quadratic
and ruinous on a ten-million-triangle rim.

### Seeding must hand out stations that land on a full patch again

A station is a point on the surface and a seed has to be an input vertex; where
the input is barely denser than the target, a patch fills up and every later
station landing there used to be spent for nothing — 18% under budget at ratio 1,
hidden on every fixture because regions that will not collapse added about as
many faces back. `seeds::place_interior` hands those stations out again over the
surface that still has room (`REFILL_ROUNDS`, weights recomputed with the full
faces left out), which put every fixture within 5% of its budget. Taking the seed
from wherever there happened to be a free vertex meets the count too, but spreads
face sizes 4.9x against 2.8x — do not.

### The survivor of a collapse has to be allowed to move

Pinning it to its own position makes every edge length static, which is tempting
because the collapse order can then be one sort. It also means the last merges
in a region drag a vertex across the whole of it and flip every sliver on the
way: 1489 collapses refused for normal flips against 102 for every other reason,
and a fifth more faces than asked for.

### A collapse can pinch an outline shut if it is allowed to

Two vertices far apart on one border loop share a feature chain and pass the link
condition, and merging them leaves a vertex with four border edges — a figure of
eight. `collapse::survivor_of` therefore requires the edge between two border
vertices to have exactly one face, i.e. the outline genuinely runs between them.
The palm's bark is the fixture: a sheet curled into a tube, whose border passes
close to itself.

### The final placement needs the flip guard the rest of the collapse has

Every placement in `collapse` refuses a move that turns an incident face over —
except, for a long time, `place_survivor`, the move of a region's last vertex to
its quadric's optimum, which was bounded only by how *far* it went
(`OPTIMUM_REACH`), not which *way*. It is the largest and least constrained move
in the rebuild, because by then the collapses have pulled the fan tight around
the vertex. Unguarded it put 3.3% of a clean fixture's triangles through each
other at ratio 0.25 and 5.4% at 0.07. Refusing the move outright costs the area
the optimum was keeping, so `OPTIMUM_BACKOFF` offers the same move at a half, a
quarter and an eighth first.

### The relaxation's guard has to be stated as a fold, not as a flip

This is the opposite call from the collapse's. Out of the collapse, 5.01% of the
palm's edges fold past a right angle, and two rounds of `cleanup`'s relaxation
bring that to 3.56%. A guard that refuses any move turning an incident face over
makes it *worse* (4.54%), because it blocks exactly the recovery moves: a face
always agrees with where it just was, so that test only measures "did this vertex
move far". A fold is a disagreement between two faces sharing an edge, and
`cleanup::fold_count` is the one-ring reading of that; a move is taken when it
does not *increase* it, which permits the recovery and makes the pass unable to
make the mesh worse.

It also has to be read from the mesh *as already moved*, over every edge the move
can tilt. `fold_count` used to count only the edges at the vertex, against its
neighbours' start-of-pass positions, while every accepted move was applied at
once — so two neighbours that each passed alone could fold the face between them,
and an edge opposite the vertex could fold with no count noticing. It now counts
every edge of every face around the vertex and judges each move against the
moves already made. On the palm at 0.25: folded edges 12.24% untidied, 11.84%
after two rounds (was 12.89%) and 11.78% after four (was 13.16%); area kept 95.6%
to 95.9%.

### A junction lies on every chain that ends at it

`Features::polyline_of` stores one chain per vertex — the lowest-numbered one —
so a vertex beside a corner where three creases meet could merge into it only on
that one chain, leaving a stubborn vertex beside every multi-crease corner.
`Features::lies_on` answers for every chain through a junction, and `survivor_of`
asks it. The flip pass, likewise, never flips an edge between two held vertices,
or it rotates a 30-90 degree crease into a zig-zag. Together: the creased column
at 0.5 came in 70% over budget and is now 48% over; its crease length went from
115% of the source's to 101%. The remaining overrun is stubborn regions, not this.

### With Align to boundaries off, the pinch guard reads the survivor's own border

A border vertex can collapse into an interior survivor when borders are not
aligned, and the survivor is then on the border although the input's flag says
not — so the figure-of-eight guard, reading the input, let a later collapse pinch
the loop. `Collapsed::border` travels with the survivor instead.

### A rebuilt face is projected however far it is from the source

The projection that reads attributes back from the source used to stop at a
quarter of the object's radius, and a Shrinkwrap offset (±0.5 m whatever the
object's size) put a small prop's whole shell past it: every face came back with
nothing to read, and the object was replaced by an empty piece without a word.
`ProjectionSource` now searches unbounded, and a face that still cannot be placed
leaves the object as it was, with a warning.

### A distance-field wrap closes an opening because every block has a value

Blocks used to be allocated only near triangles, so the winding-number surface
spanning a wide opening — mid-air, by definition — fell in cells with
unallocated corners, which the extraction skips: an open bin came back with a
ragged ring instead of a cap. The sampler now reads the inside test at block
corners, allocates every block whose corners disagree, and stores every other
block as one value (the band, signed by that test). Every cell then has eight
values and the shell is closed by construction; a closed object allocates
exactly what it did before.

### Tangential relaxation is only as good as the vertex normal

The pass drops the part of its move along the vertex normal, which is what keeps
it on the surface. At the narrow end of a leaf the sheet above and the sheet
below are in the same fan and point opposite ways, so the area-weighted sum
nearly cancels, and subtracting the rounding error that is left walks the vertex
through the shell. `cleanup::vertex_normal` reports how much the fan agrees (the
weighted sum's length over the total area), and the pass declines below
`NORMAL_AGREEMENT`. Worth ten points of a thin object's area.

### A black patch is an inside-out face, and the hole test cannot see one

`welded_edge_uses` counts edges *undirected*, and an inverted triangle uses each
of its edges exactly once like any other — so a mesh can pass every no-holes
check and still show dark patches, because a surface lit from behind is black.
The cleanup's edge flip produced them by rewriting the two triangles either side
of an edge to a winding it *assumed*: the two faces of an oriented surface
traverse their shared edge in opposite directions, which is a property of the
mesh, so guessing turned both over every time the guess was wrong. Its
normal-flip guard compared fabricated before-normals against fabricated
after-normals under the same assumption and so never noticed.
`cleanup::Quad::read` now reads the winding from the corner order, and
`a_rebuild_is_wound_the_same_way_all_over` pins it on directed edges.

### Snapping the rebuild back onto the source surface makes it worse

It is the obvious last pass, and what a remesher that *synthesizes* a surface
ends with. Measured over every vertex: a plant 96.8% of its area to 96.1%, a
column 86.1% to 84.0%, stones 99.7% to 98.0%; narrowed to only the vertices the
relaxation had just moved, the plant 89.0% to 87.3%. This rebuild synthesizes
nothing: its vertices are source vertices or the optimum of a quadric over the
source's own planes, and on a convex patch that optimum sits slightly *outside*
the surface — which is where a coarse mesh has to be to keep the area of the fine
one. The nearest point on the source is by definition not outside it, so
snapping inscribes what was correctly circumscribed. Do not re-add it.

### Two changes that look right and measure worse

Do not retry these without a fixture that shows otherwise.

- **Spacing chain stations by cost** (per-vertex `h`) instead of one averaged
  size per chain: correct in principle, and 1% worse on the plant at every
  smoothing setting, because no fixture here has a rim whose density varies.
- **Widening the size field's dilation** from 2 passes to a radius in `h`: at
  these densities it resolves to the same 2 passes, and where it does bite it is
  a trade — 4 passes buys evenness (5.64x edge spread to 4.96x) and costs area
  (97.3% to 96.8%).

### A quad is two triangles with the edge between them rubbed out

`remesh::quads` runs after the cleanup and merges a pair only when the four
corners are near enough coplanar — `MERGE_WARP`, a corner's distance out of the
other three's plane over the mean edge, **not** a dihedral angle, which reads 160
degrees across the short diagonal of two long thin triangles whose corners are
all but in one plane. Nothing moves: the merged quad keeps its own diagonal in
its corner order, `layout::canonicalize` rotates a quad only by an even number of
places so that survives, and `remesh::fan` splits on corners `0..2`, so the
triangles drawn are the triangles that were there before the merge. Do not
"improve" `fan` to pick the shorter diagonal: that silently flips the
triangulation of every quad whose merge diagonal was the longer one.

**Greedy matching strands more triangles than it looks like it should**, because
a pair taken early can be the only partner two other faces had. `quads::augment`
finds the length-three augmenting paths (a stranded triangle, a quad, another
stranded triangle) and is worth about ten points of quad share. What is left is a
property of the model: at half density, 81% on a plant and 47% on a creased stone
column, where nearly every edge is a genuine fold and a quad across one would be
a lie.

### Measuring a tear on the assembled buffer counts material seams

A node is split into one piece per material with its own vertices, so a closed
object reads as several open ones. Weld by position first — `welded_edge_uses` in
`tests/remesh.rs` — which is the same weld `remesh::proxy` works on.

---

## The Aud workspace

- **Inverted normals are judged against the mirror-corrected winding.** Import
  copies a face's winding as authored but sign-corrects the normals on a mirrored
  node, so `geometry.inverted_normals` multiplies the winding by
  `sign(det(node × geometry_to_node))`; without it every mirrored half of a model
  reads as inside out. And a face is inverted only when *every* corner normal
  opposes the winding by more than `INVERTED_COS` — averaged normals on a smooth
  surface routinely lean across a small face, and one disagreeing corner is
  shading, not a flipped face. Both were false positives on real fixtures.
- **`SceneFrame::with_model` resets `audit` to `None`.** The highlight's lists
  index the *source* model's triangles and corners. The Opt workspace hands the
  renderer its processed mesh through `with_model`, and an inherited overlay
  would tint arbitrary faces of a different mesh.
- **Clay is keyed on grouping, not on the material mode.** Focusing a finding
  switches Source to Standard; the mesh, selection, hover and visibility bakes
  compare `MaterialMode::groups_by_part()`, equal for the two, so a click in the
  Issues list rebuilds nothing. Keying them on the raw mode rebuilt the mesh on
  every focus change.
- **The highlight is drawn in the Issues view only.** Over a density or overdraw
  view a severity tint mixes into the ramp and the model stops matching the key.
- **The audit waits for the source-property capture.** Several checks read it,
  and it lands after the model is drawn; starting on `ModelLoaded` would run the
  audit twice. A new model's run is not respawned by the old one's result: it
  waits for its own capture (`AuditSubsystem::finish` only respawns for an edit).
- **A file can declare a unit its geometry was not modelled in.** The checks
  measure in the file's declared unit, so a wall modelled in meters but declared
  in centimetres is a 4 cm wall to every density check — `SM_Wall_Break_4x3m`
  reports 228845 px/m. The figure is right; the file is what the finding is
  about.
- **Overdraw counts read the mesh's own vertex buffer.** Both views draw each
  visible triangle as three corners pulled through `VIEW_LINE_VERTICES`, with
  the triangle list in a storage buffer at `VIEW_LINE_INDICES` — the same slots
  the `wire` program uses. The quad view's depth prepass uses `MESH_DEPTH_BIAS`,
  since the prepass (`vs_overdraw`) and the count (`vs_quad_overdraw`) are
  different shaders and nothing guarantees their positions agree bit for bit.
  `fs_quad_overdraw`'s rule has a CPU twin in `scene/overdraw.rs`'s tests; change
  them together.
- **A deferred draw's uniform block is capped.** `SwapchainJob` copies its
  uniforms into a fixed `MAX_JOB_UNIFORM_BYTES` buffer, checked at compile time;
  the overdraw ramp's 112-byte block is why it is 112.

---

## The chrome

### An `egui::Panel` is as wide as its body measured, not as wide as it was

egui 0.36's `Panel` lays its body out in a frame and stores the *frame's*
measured rect as the panel's size for the next pass. A body that does not fill
the width shrinks the panel, and one that overflows it grows it — so the
Inspector came out 262pt in 3D (a one-line "nothing selected"), 196pt in Tex,
300pt in Opt and Aud (whose bodies are `auto_shrink(false)` scroll areas) and
329pt on Opt's export settings, all from the one stored width. `overlay.rs`'s
`pinned_to_panel_width` claims the whole rect up front and lays the body out in
a child `Ui` whose size is never reported back, so the panel is only ever the
width the user dragged it to; `the_inspector_is_one_width_in_every_workspace`
pins it. Long text that would overflow should still be made to fit —
`labeled_combo` truncates its selected text, which was the export settings'
29pt — because what overflows the pinned body is cut off at the panel's edge.

### A view setting the view on screen ignores must never be shown lit

The toolbar used to show two things on while the viewport drew one: UV Checker
lit under a texel-density map, Shaded lit over a Buffers view, the grid lit
under an overdraw count. `state/view_rules.rs` is the table that prevents it,
and it is a *mirror* of the renderer's precedence, not a second opinion — each
rule names the decision it copies (`record_overdraw`, `mesh_draw_list`,
`wireframe_on`, `flat_display`, `draws_skybox` and the skybox's coverage). Add
a precedence to the renderer and the table needs the matching rule, or a lit
tile lies again.
Two tests pin the shape: `one_tile_paints_the_model` (the material tiles and
the diagnostic views are one choice, exactly one lit) and
`every_override_can_be_left` (the controls that leave an overriding mode are
never overridden themselves — greying out Unlit and Shaded in Wireframe Only
would have trapped the user there). `flat_display` and `UiState::flat_view` are
twins: both are false in Wireframe Only, where there is no surface to show
flat, and the renderer used to pass a wireframe's lines through untone-mapped
whenever Buffers or Skin Weights happened to be the active material.

### An `Arc::as_ptr` kept as a number is not an identity

Once the `Arc` is dropped the next allocation can land at the same address and
read as the same object, so a cache keyed on it serves a stale result. Hold a
`Weak` instead and compare `Weak::as_ptr` against `Arc::as_ptr`: a live `Weak`
keeps the allocation itself alive (though not the data), so nothing else can
arrive at that address while it is held, and `strong_count() == 0` says when the
entry can go. `widgets/thumbnail.rs`, the Tex fit and the renderer's material
texture cache are the examples. Also: egui temp data is never collected, so
anything cached there per key lives until exit — bound it.

### `ScrollArea::show_rows` takes its row pitch from the `Ui` it is called on

It places rows at `row_height + item_spacing.y` read from the *outer* ui, so a
list that zeroes `item_spacing.y` only inside the closure is placed on a pitch 3pt
taller than it is drawn on: rows jump as it scrolls and the list runs past its
end. Zero the spacing on a scope around the `ScrollArea` (the Outliner's Scene and
Issues tabs do). And because a virtualized row may not be laid out, scroll to it
with `ui.scroll_to_rect` at its computed position (`top + i * row_height`) rather
than `Response::scroll_to_me`.

---

## Loading, undo and the Opt run

### The undo baseline is taken last

Any reset that bumps a revision the undo snapshot compares — clearing the Opt
overrides on a load, restoring the saved audit profile at launch — must run
before `reset_undo_history`. Otherwise the next frame records the bump as an
edit, and Ctrl+Z restores the previous model's per-object overrides onto whatever
nodes sit at those indices now, or the default audit profile, which the
settled-profile save then writes over the user's on disk. Both load paths go
through `App::replace_scene_model`, which ends with the rebaseline.

### Two load generations, not one

`model_load_generation` cancels parses and is bumped the moment a request is
made. `measure_generation` cancels the on-screen model's measuring and is bumped
only when a model actually replaces it. One counter for both left a model
unmeasured for the rest of the session — no pick BVH, no capture, no audit, no
clip envelopes — whenever an open failed.

### An Opt preview has its own revision

The renderer keys mesh buffers on the revision alone, so a preview reusing its
finished level's number stayed on screen after a cancel or an error while
everything else described the level. A preview carries its own revision and the
level keeps its own, so dropping a preview is a re-upload. A preview has no BVH,
so picks in that half fall back to the source and its labels draw unoccluded.

### An Opt run started before the capture lands is run again when it lands

A load publishes the mesh before the source-property capture is marshaled, and
the Opt workspace starts its run at once, without the capture — so nothing the
export carries from it (polygons, extra skin layers) rides that run.
`OptSubsystem` notes the run was owed the capture (`capture_owed`) and reruns
when it settles; until then an export of that chain says the properties had not
loaded rather than blaming the stack for the triangles.

---

## Tools and builds

### review-cli compares files by identity, not by spelling

`inputs::identity` makes a path absolute, canonicalizes the longest part that
exists and appends the rest, and folds case on Windows and macOS. Comparing
`PathBuf`s let `Rock.FBX` be overwritten by its own `Rock.fbx` output, and let
an `--out` folder created mid-run give two models' outputs different keys, so
the second silently replaced the first.

### A build script's `#[cfg(target_os)]` is the host's

Read `CARGO_CFG_TARGET_OS`, and keep the build dependency unconditional: Cargo
matches a *build* dependency's platform against the host too, so
`shell-macos/build.rs` neither ran nor had its dependency when cross-checked
from Windows.

---

## Baked assets

### IBL HDRs must stay finite

Bright suns in an HDR exceed `f16`'s maximum (65504), so the bake's
`load_equirect_from_file` clamps every channel to `F16_MAX` before the
`Rgba16Float` upload. Otherwise they become `inf`, the unbounded irradiance
integral turns to `NaN`, and the model shows black speckles with a dead spot at
the sun.

The `ibl_*` programs additionally clamp each *sampled* radiance to
`IBL_RADIANCE_CLAMP` in both convolutions (irradiance and prefilter) to kill
fireflies, and the skybox clamps `env * intensity` to f16 max so the intensity
multiply cannot re-overflow the HDR target. These clamps run at bake time, so the
shipped maps are already finite. Do not drop them.

### Embedded assets must exist before `cargo build`

- **HDR thumbnails** (`assets/thumbnails/T_HDR_*.png`) are `include_bytes!`-embedded
  by `crates/ui`. They are committed;
  `packaging/generate-hdr-thumbnails.ps1` (ImageMagick) regenerates them from
  `assets/textures/T_HDR_*.hdr`, and the installer build runs it automatically.
  Re-run it after adding or replacing an HDR.
- **Baked IBL maps** (`assets/ibl_baked/T_IBL_*.bin`) are `include_bytes!`-embedded
  by `crates/render`. Also committed; `packaging/generate-ibl-bake.ps1`
  regenerates them by running the `bake_ibl` tool, which needs a real GPU.

The three HDR cubes (env, irradiance, prefilter) are **BC6H** block-compressed
(`Bc6hRgbUfloat`, 16 bytes per 4×4 block, encoded at bake time with `intel_tex_2`);
the shared BRDF LUT stays raw little-endian f16 (`Rg16Float`). BC6H is core in
Direct3D 11 feature level 11_0 — the renderer's floor — so no explicit feature
request is needed.

The `.bin` byte layout is **mip-major with the six cube faces contiguous per mip**,
each face a row-major grid of BC6H blocks. The bake's `write_cube` /
`compress_bc6h_face` and the runtime `Texture::cube_block_compressed` must stay in
lockstep on it.

### The IBL bake is freshness-gated

The installer build invokes `generate-ibl-bake.ps1`, but it only runs the GPU bake
when a baked `.bin` is missing or older than an input that determines its bytes — a
source HDR, or `ibl.rs` / `review.glsl` / `bake_ibl.rs` / `rhi/bake.rs`, meaning an
IBL precompute constant or the encode path. Otherwise it is a fast no-op, so a
normal build touches no GPU. Pass `-Force` to re-bake regardless.

The shipping binary carries the baked maps, not the raw HDRs: `T_HDR_*.hdr` are
bake-tool inputs only.

Run the bake in `--release`, because the BC6H encoder's highest-quality profile is
slow. But note that sokol's validation layer is compiled *out* of a release build,
so if the bake misbehaves, reproduce it in debug where a bad call says so.

### Shader bytecode staleness is by content, not timestamp

`src/shaders/generated/bytecode.manifest` records the SHA-256 of each generated
source and of the blob compiled from it, and is committed beside them. Commit it
whenever you commit bytecode.

Timestamps cannot answer this once two OSes commit blobs: a shader rebuilt on the
Mac reaches Windows as a new source next to a blob one revision behind, both
stamped by `git checkout` at the same instant.
`packaging/check-shader-bytecode.{ps1,sh}` fails a release build on any mismatch,
and the installer build runs it. `build.rs` additionally warns when `review.glsl`
is newer than everything generated from it — the one thing the manifest cannot see,
namely that you edited the shader and never ran `scripts/gen-shaders`.

---

## The Explorer shell extension

### A preview handler's calls arrive on threads that never pump messages

`prevhost.exe` creates a preview handler in the **neutral apartment**, so each call
Explorer makes runs on whichever RPC thread is free, and none of those threads
runs a message loop. A window created inside `DoPreview` received the messages
*sent* to it while it was being created, and then nothing — no `WM_PAINT`, no
timer, no message the load worker posted — for six seconds, until the next call
into the handler (`Unload`) happened to run on its thread. In-process (a test
host calling it directly) the same code worked, which is what made it look like
the harness was at fault. Every pane window therefore lives on
`preview/ui.rs`'s own thread, and the handler's methods *post* work to it rather
than wait on it: a waiting call could deadlock against Explorer, which is blocked
in that very call while the window being made sends messages to its parent there.
One consequence is settled rather than fixed: keystrokes reach that thread's loop,
not the host's, so `TranslateAccelerator` never sees one, and Explorer's own
shortcuts do not work while the pane has the keyboard.

### A pane window takes its parent's DPI awareness, or lands in the wrong place

A window's DPI awareness is its thread's at the moment it is created, and a child
whose awareness differs from its parent's is placed through the system's
mixed-mode scaling. Hosted by a DPI-unaware test harness, the pane came out at
*half the host window's screen position* inside it - (20, 20) for a host at
(40, 40), (100, 60) at (200, 120) - at 100% display scaling, with `SetWindow`'s
rectangle and `CREATESTRUCT` both saying (0, 0). `window::create` therefore
switches its thread to the parent's context (`GetWindowDpiAwarenessContext`) for
the creation and back after. Explorer is per-monitor aware and may never have
shown it, but a host that is not would have.

### A dark-mode Explorer gives its preview pane a white background

A handler that takes the colors `IPreviewHandlerVisuals` offers came up white in a
dark-mode Explorer on Windows 11, model and all. The test harness never showed it,
because the harness handed the handler a dark background itself. The handler no
longer implements that interface: the pane is the viewer's own black backdrop
(`preview/pane.rs`'s `BACKGROUND`) in either theme, and the harness no longer sets
colors, so it shows what Explorer shows.

### A thumbnail handler gets bytes with no path, and every way round that was tried

The thumbnail provider is `IInitializeWithStream`, which is what puts it in
Explorer's isolated COM Surrogate rather than in `explorer.exe`. The stream's
`Stat` name is the bare file name, so textures beside the model cannot be found
(embedded ones, and absolute paths that still exist, can). Measured, so as not to
be tried again: the stream answers `E_NOINTERFACE` to `IPersistFile`, `IPersist`,
`IServiceProvider`, `IObjectWithSite`, `IShellItem` and `IPropertyStore`; offered
`IInitializeWithItem` beside the stream, the surrogate takes the stream; offered
the item alone, extraction fails (`0x8004B200`); `DisableProcessIsolation = 1`
passes an item by loading the handler into the *calling* process — `explorer.exe`
for a folder view — and giving the class an AppID with a surrogate does not move
it back out. A GPU device and a C parser inside Explorer is what the isolation is
for. The preview pane, which does get the path, finds every texture.

### The thumbnail cache keeps straight alpha, and reads back top row first

Windows' own PNG thumbnailer, handed a half-transparent red PNG, stores
`R 255, A 128` in the cache — straight alpha — so the provider hands over straight
alpha too (`WTSAT_ARGB`) and the two match on screen. When checking a thumbnail
through `IShellItemImageFactory::GetImage`, note that the bitmap it returns
reports a *positive* height yet stores its top row first: read it bottom-up, as a
positive height says, and every thumbnail looks upside down. A red-over-blue PNG
is the control that settles it in one run.

### The shell DLL must never abort

Explorer's COM Surrogate hosts other vendors' thumbnail providers beside ours, and
a Rust panic that unwinds into a COM vtable shim — an `extern "system"` function —
aborts the process. So every entry point and the window procedure run inside
`guard::guarded`, and the DLL is built with the `release-shell` profile
(`panic = "unwind"`); the workspace's `release` aborts on panic, which would make
every guard a no-op. A panic inside a draw also drops the GPU host, since a frame
abandoned mid-pass leaves nothing on the device worth trusting.

### A registered DLL is locked by the process that loaded it

A surrogate keeps the DLL loaded long after the folder that needed it is closed,
so `cargo build` cannot overwrite a DLL that is registered in place.
`scripts/register-shell.ps1` registers a *copy* under `target/shell-registered/`,
and each run registers a fresh one. The installer relies on Setup's Restart
Manager pass to close a surrogate holding the old copy; Explorer starts a new one
on demand.
