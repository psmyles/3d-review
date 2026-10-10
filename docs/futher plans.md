# Gap analysis: what a commercial FBX audit / optimization tool still needs

## Context

The viewer, the four workspaces, the material/texture system, skinning + animation,
the Opt stack and the FBX round-trip export are all built and documented
(README.md). The user's stated scope is broad: tech-artist pre-import audit,
outsourcing QA acceptance, an optimization operator, a general previewer, a
**headless batch optimizer driven by saved presets**, targeting Unity + Unreal +
engine-agnostic pipelines, with import _and_ export formats allowed to broaden, as a
**commercial / public release**.

Every item below was checked against the code (not just the README). Legend:
ABSENT = nothing exists, PARTIAL = data or plumbing exists but no user-facing
feature. File paths point at the seam a feature would attach to.

---

## Tier 1 — gaps that block a use case the user named

### 1. Headless batch mode (preset × files → export + report) — ABSENT

`crates/app/src/main.rs:77-88` hand-scans three flags (`--tracy`, `--gate-out`, a
path); nothing runs without a window. Everything underneath is already pure:
`review_import::load_model`, `review_optimize::process` + `preset` (JSON) +
`export`, and the AO bake is CPU raycasts. What's missing is the driver:

- A `review-cli` binary (or `3d-review batch` subcommand) with a real arg parser:
  `--preset p.json --out <dir> [--recurse] [--glob] <files...>`, per-file
  progress on stdout, non-zero exit on any failure, `--report out.json`.
- Overwrite policy (code-quality finding F2: export writes over the destination
  with no check) — matters far more once a batch can clobber a whole folder.
- Naming template for outputs (`{stem}_LOD{n}.fbx` etc.).
- Parallelism across files (each run already uses `std::thread::scope` inside).

### 2. Automated validation / audit report — DONE in the Aud workspace, except materials / textures

Shipped as the **Aud** workspace (`crates/audit`): 43 checks over mesh, transforms,
UVs, skin, density, naming and hierarchy, Unity / Unreal / Generic profiles in a
versioned JSON envelope, an Issues list, and a JSON report whose schema the batch
mode can reuse. Still open: the materials / textures rules below, and a pass/fail
verdict over the report. The original gap analysis follows.

The only user-visible warning today is one toast (`crates/app/src/loading.rs:394`).
`crates/model/src/skin.rs:140` / `geometry.rs:111` reject malformed data at load;
nothing _reports_ on a well-formed but bad asset. QA acceptance and outsourcing
review need a rule set with pass/fail:

- **Mesh**: degenerate / zero-area triangles, non-manifold edges, isolated verts,
  unwelded duplicates, n-gon count, missing normals/tangents, inverted normals
  (winding vs. normal), hard-edge ratio, triangle and draw-call budgets.
- **Transforms**: non-identity or negative scale, unfrozen transforms, pivot away
  from origin, unit ≠ target, up-axis (imported into `extras.rs:689` but never
  shown).
- **UVs**: outside 0–1, overlapping islands, flipped islands, missing set,
  lightmap set (UV2) present + non-overlapping + padded — Unity and Unreal both
  need this.
- **Skin**: > 4 / 8 influences per vertex, unnormalized weights
  (`skin.rs:40` notes they are not guaranteed to sum to 1), bones per mesh over
  the engine limit, unused bones, rest ≠ bind mismatch.
- **Materials / textures**: missing files, NPOT, oversize, unused or duplicate
  materials, texture memory estimate.
- **Naming**: regex rules, `_LOD<n>` suffix consistency, socket/collision
  prefixes (`UCX_`, `SM_`).
- Rules live in a versioned JSON spec like the Opt preset so a studio can share
  one per project; results show in an **Issues panel** (click → select + frame
  the offender) and go out as JSON/HTML from the CLI.

### 3. Import formats beyond FBX — ABSENT (glTF/GLB, OBJ; USD is a separate question)

The user opened this up. glTF/GLB is the common outsourcing delivery and the
Godot/agnostic native. Constraints from the invariants: each format is its own
funnel producing an identical `ModelData` (invariant 7 applies to the ufbx core,
not to adding a sibling), and `SourceExtras` is FBX-shaped — a glTF source has no
extras, so the export of a non-FBX source is lossy by construction and the report
must say so. USD needs a C++ dependency (OpenUSD) far heavier than anything
vendored so far; recommend deferring it and listing it separately.

### 4. Export formats beyond FBX — ABSENT

glTF/GLB export from the Opt chain is the natural pair to item 3 (and what
Unity/Unreal both import). The processed `ModelData` + `LevelCarry` already hold
everything a glTF needs; skins/morphs/clips map cleanly.

### 5. Embedded FBX textures for display — PARTIAL

Bytes are captured (`crates/model/src/extras.rs:413`) and written back on export,
but never decoded for the viewport; `crates/app/src/texture_manager.rs:8` lists it
as an unbuilt phase. Outsourced deliveries frequently embed. Needs a decode path
from bytes into the same pool as file textures.

---

## Tier 2 — audit depth a tech artist expects (Unity / Unreal)

7. **Texel density** heat map + per-mesh px/cm figure for a chosen texture size —
   DONE (Aud: per-face heat map in px/m and the `density.texel` check). UV checker exists; density does not.
8. **UV diagnostic modes** in the UV workspace: overlap, flipped, out-of-range,
   lightmap-UV padding — DONE as Aud findings, shown in a 3D | UV split; not yet
   as modes of the UV workspace itself.
9. **Overdraw / quad-overdraw visual mode** — DONE (Aud's Overdraw and Quad
   overdraw views). The reorder op can't be judged visually today.
10. **Skin audit stats** in the Inspector/stats card: max influences, bones per
    mesh, weight-sum deviation, unused bones — DONE as Aud checks; not on the
    stats card.
11. **Up-axis + unit in stats/Inspector, and a target-engine preview** (cm for
    Unreal, Y-up for Unity) — PARTIAL (unit shown, axis not).
12. **Lights and cameras drawn as gizmos; adopt an FBX camera as the view** —
    PARTIAL (listed in the Outliner, no geometry).
13. **Normal-map convention** per texture (DirectX vs OpenGL green flip) with a
    heuristic auto-detect — ABSENT (`review.glsl:547` samples raw).
14. **Directional key light with a shadow map** — ABSENT. IBL-only hides
    silhouette and normal-map errors that a hard light exposes. Optional but
    common in every peer tool.
15. **LOD export with screen-size / distance thresholds** as a real `LodGroup` —
    PARTIAL. Chains go out as `_LOD<n>` siblings only; ufbx-write patch P4 already
    supports LOD groups and the probe test writes thresholds. Wire to generated
    chains, and add a distance-driven **LOD transition preview** in the viewport.
16. **A/B of two arbitrary files** (v1 vs v2 of the same asset) — PARTIAL. The
    renderer already has the two-slot design for Opt; generalize to a second
    loaded file. Directly serves outsourcing revision review.
17. **Measurement tool** (point-to-point ruler) and **camera bookmarks** — ABSENT.
    Smaller, but expected in a review tool.
18. **Texture reporting**: NPOT, VRAM estimate, mip count, oversize — PARTIAL
    (format/size/bit depth only).

---

## Tier 3 — commercial-release plumbing

19. **Crash handling**: no `panic::set_hook`; a panic in the windowless binary is
    invisible (`main.rs:798` comment). Need a hook → dialog + log file in the
    config dir + "copy diagnostics" (GPU, driver, OS, file being loaded).
    Structured file logging generally.
20. **Update check / auto-update**, Windows code signing (macOS is signed and
    notarized; Windows installer exists but signing is not mentioned).
21. **Preferences window + persisted settings** — ABSENT. `window_state.rs`
    hand-rolls x/y/w/h/maximized only. Default environment, AA, units, mouse
    scheme, key remapping, theme, default preset, texture watch on/off.
22. **Recent files / MRU, a Windows menu bar, session restore** — ABSENT (menu bar
    exists only on macOS).
23. **In-app help**: shortcut cheat sheet, tooltip coverage (~33 tooltips across
    17 panels), first-run hint, docs link — ABSENT/PARTIAL.
24. **Screenshot / turntable capture** — ABSENT. Readback exists only in the
    bake-gated leaf (`read_image_subresource`). PNG capture with/without chrome
    and a turntable image sequence are how reviewers attach evidence to tickets.
25. **Code-quality findings F1–F8** (`docs/CODE_QUALITY_ANALYSIS.md`) are
    hardening items a public release needs: F2 (silent overwrite) and F7
    (superseded background work accumulating) especially.
26. **Opt-in telemetry / crash upload** — a product decision, listed for
    completeness.
