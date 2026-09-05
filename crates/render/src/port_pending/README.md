# Nothing is parked here any more

This directory held the Direct3D 11 modules the renderer drew through before the
sokol_gfx port (`mac-port-plan.md` Phase 1). They were **not declared as modules** —
nothing here was compiled, linted or formatted — so the tree kept building and the
viewer kept running while the renderer was rebuilt underneath them, stage by stage.

That is deliberate rather than incidental: step 3 landed the new `rhi` core, the frame
flow and the egui renderer with `Renderer::render_*` stubbed, and step 4 revived the
scene "in this order, each stage eye-checked against `main` on the same model". A file
left this directory when its stage landed, rewritten against the new `rhi`; it lived
here rather than only in git history so the port was a diff against something rather
than a rewrite from memory.

**Every file has now left.** The last four went with step 6:

| File | Was | Left as |
| --- | --- | --- |
| `tex_d3d.rs` | the Tex viewport's draw | `src/tex/gpu.rs` + `src/rhi/mips.rs` (step 4 stage 1) |
| `scene_pipelines.rs`, `scene_resources.rs`, `scene_d3d.rs`, `material_d3d.rs`, `rhi_target.rs`, `ibl.rs`'s runtime half | the 3D + UV scenes | `src/scene/{pipelines,resources,gpu}.rs`, `src/material/gpu.rs`, `src/rhi/target.rs`, `src/ibl.rs` (step 4 stage 2) |
| `scene_opt.rs` | the Opt comparison view | `src/scene/opt.rs` (step 4 stage 5) |
| `gpu_profiler_zones.rs` | the timestamp-query `Zone` machinery | `src/rhi/gpu_profiler.rs` + `backend::GpuTimer` (step 6, D18) |
| `bake.rs` | the headless bake device + readback | `src/rhi/bake.rs` + `backend::read_image_subresource` (step 6, D19) |
| `ibl_bake.rs` | the offline IBL precompute | the `bake`-gated half of `src/ibl.rs` (step 6, D19) |
| `bake_ibl_main.rs` | the bake binary | `src/bin/bake_ibl.rs` (step 6, D19) |

Three of them were never ported at all: `scene_pipelines.rs`, `scene_resources.rs`,
`scene_d3d.rs`, `material_d3d.rs` and `rhi_target.rs` were rewritten wholesale by step
4's scene stage, and the MSAA and GTAO features they *also* carried came back as
additions to the sokol versions rather than as ports of the parked ones.

**This directory and its README go with step 7**, alongside the dead `src/hlsl/` set
and its hand-listed `build.rs` jobs. It is kept until then only so the step-7 cleanup
is one commit that removes both.
