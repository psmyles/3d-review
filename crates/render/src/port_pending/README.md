# Not yet ported to sokol_gfx

These are the Direct3D 11 modules the renderer drew through before the sokol_gfx
port (`mac-port-plan.md` Phase 1). They are **not declared as modules** — nothing
here is compiled, linted or formatted — so the tree builds and the viewer runs while
the renderer is rebuilt underneath them stage by stage.

That is deliberate rather than incidental: the plan's step 3 lands the new `rhi`
core, the frame flow and the egui renderer with `Renderer::render_*` stubbed, and
step 4 revives the scene "in this order, each stage eye-checked against `main` on the
same model". A file leaves this directory when its stage lands, rewritten against the
new `rhi`; it is here rather than only in git history so the port is a diff against
something rather than a rewrite from memory.

| File | Was | Returns in |
| --- | --- | --- |
| `scene_opt.rs` | `src/scene/opt.rs` — the Opt split + ghost overlay | step 4, last stages |
| `ibl_bake.rs` | the offline precompute half of `src/ibl.rs` (its runtime upload half is live again) | step 6 |
| `gpu_profiler_zones.rs` | `src/rhi/gpu_profiler.rs` — the timestamp-query `Zone` machinery | step 6 |
| `bake.rs` | `src/rhi/bake.rs` — the headless bake device + readback | step 6 |
| `bake_ibl_main.rs` | `src/bin/bake_ibl.rs` — the offline bake binary | step 6 |

The files that carried MSAA and GTAO left this directory even though neither stage has
landed: `scene_pipelines.rs`, `scene_resources.rs`, `scene_d3d.rs`, `material_d3d.rs`
and `rhi_target.rs` were rewritten wholesale by step 4's scene stage, and the two
features they *also* carried come back as additions to the sokol versions rather than
as ports of the parked ones — there is nothing left in them worth diffing against.

`rhi/gpu_profiler.rs` still exists and still carries the `--tracy` arming flag,
`should_enable` and `note`; only the D3D11 query machinery moved here. The other
`rhi` modules — `pipeline`, `buffer`, `texture`, `sampler`, `bindings`, `shader` —
were **rewritten in place** on sokol_gfx rather than parked, and grow a knob at a
time as step 4's stages need them; `target.rs` is here because nothing in step 3
renders offscreen at all.

Gone from the table already:

* `tex_d3d.rs` left with step 4's first stage, rewritten as `src/tex/gpu.rs` — two
  deferred `SwapchainJob`s over the generated `tex_image` / `tex_checker` programs,
  with the CPU mip chain (D7) in the new `rhi/mips.rs`.
* `scene_pipelines.rs`, `scene_resources.rs`, `scene_d3d.rs`, `material_d3d.rs`,
  `rhi_target.rs` and `ibl.rs`'s runtime half left with the scene stage, as
  `scene/{pipelines,resources,gpu}.rs`, `material/gpu.rs`, `rhi/target.rs` and
  `ibl.rs`.

## What step 6 must restore to `Cargo.toml`

The `bake` feature and its binary were removed along with the bake path, so a
`--features bake` build cannot silently pass by compiling nothing:

```toml
intel_tex_2 = { version = "0.5", optional = true }

[features]
bake = ["dep:intel_tex_2"]

[[bin]]
name = "bake_ibl"
path = "src/bin/bake_ibl.rs"
required-features = ["bake"]
```

Until then `packaging/generate-ibl-bake.ps1` cannot run. It is freshness-gated, so a
normal packaging build is unaffected — every baked `.bin` is committed and current —
but `-Force`, or an edited IBL precompute constant, fails until step 6.
