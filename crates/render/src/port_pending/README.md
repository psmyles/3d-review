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
| `tex_d3d.rs` | `src/tex_d3d.rs` — the Tex viewport's image draw | step 4, first stage |
| `scene_pipelines.rs` | `src/scene/pipelines.rs` — `ScenePipelineSet` | step 4 |
| `scene_resources.rs` | `src/scene/resources.rs` — `ModelSlot` / `DerivedViews` / every `sync_*` | step 4 |
| `scene_d3d.rs` | `src/scene/d3d.rs` — `SceneGpu`, the passes, the uniform encoders | step 4 |
| `material_d3d.rs` | `src/material/d3d.rs` — the material table + texture cache | step 4 |
| `ibl.rs` | `src/ibl.rs` — the baked-map upload *and* the offline precompute | step 4 (upload), step 6 (bake) |
| `scene_opt.rs` | `src/scene/opt.rs` — the Opt split + ghost overlay | step 4, last stages |
| `rhi_target.rs` | `src/rhi/target.rs` — the offscreen MSAA colour + depth targets and their resolve | step 4, with the scene pass |
| `gpu_profiler_zones.rs` | `src/rhi/gpu_profiler.rs` — the timestamp-query `Zone` machinery | step 6 |
| `bake.rs` | `src/rhi/bake.rs` — the headless bake device + readback | step 6 |
| `bake_ibl_main.rs` | `src/bin/bake_ibl.rs` — the offline bake binary | step 6 |

`rhi/gpu_profiler.rs` still exists and still carries the `--tracy` arming flag,
`should_enable` and `note`; only the D3D11 query machinery moved here. The other
`rhi` modules — `pipeline`, `buffer`, `texture`, `sampler`, `bindings`, `shader` —
were **rewritten in place** on sokol_gfx rather than parked, and grow a knob at a
time as step 4's stages need them; `target.rs` is here because nothing in step 3
renders offscreen at all.

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
