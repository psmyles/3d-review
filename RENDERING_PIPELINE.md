# Rendering Pipeline

Audit date: 2026-06-17

The current renderer is a `wgpu` pipeline embedded into egui through an
`egui_wgpu` paint callback. It renders the 3D or UV scene into offscreen
render-owned textures during callback `prepare`, then composites the resolved
scene into egui's framebuffer during callback `paint`, behind the egui chrome.

## Frame Flow

```text
winit event
  -> app::render()
  -> egui_ctx.run()
  -> review_ui::draw_viewport_scene()
  -> SceneCallback is registered as an egui paint callback
  -> SceneCallback::prepare()
       sync model/resources/options
       render scene into offscreen MRT targets
       optionally run SSAO
       optionally run bloom
       update post uniforms
  -> SceneCallback::paint()
       fullscreen composite into egui framebuffer
  -> egui draws toolbar, panels, stats, gizmo, help/fade overlays
```

The app redraws on demand. Repaint is continuous only while camera animation,
pointer interaction, egui repaint, or startup fade is active, and continuous
frames are paced to the monitor refresh interval.

## Main Renderer Data Flow

```text
ModelData
  -> geometry::model_mesh()
      SceneVertex buffer + u32 index buffer
  -> SceneResources::update_model()
      GPU mesh buffers
  -> SceneCallback::record_scene()
      mesh / grid / debug / skybox draws
  -> SceneTargets
      linear scene color + bloom source + ambient radiance + Reversed-Z depth
  -> SSAO G-buffer pass
  -> SSAO and bloom fullscreen passes
  -> PostPass composite
```

Debug and UV buffers are derived from `ModelData` only when needed. The renderer
tracks baked parameters, so a line view rebuilds only when its toggle, color, or
length changes.

## Offscreen Targets

`SceneTargets` owns the per-size, per-MSAA render targets:

| Target | Format | Purpose |
| --- | --- | --- |
| Color MRT location 0 | `Rgba16Float` | Linear HDR scene radiance. |
| Bloom MRT location 1 | `Rgba16Float` | Linear HDR bloom source; overlays write zero contribution. |
| Ambient MRT location 2 | `Rgba16Float` | Linear AO-eligible ambient radiance. |
| Scene depth | `Depth32Float` | Reversed-Z scene depth for mesh/line/skybox ordering. |
| SSAO G-buffer | `Rgba16Float` | Separate single-sample view-space normal in `xyz`, view-space Z in `w`. |
| SSAO depth | `Depth32Float` | Single-sample Reversed-Z depth for the SSAO G-buffer pass. |

When scene MSAA is enabled, render views are multisampled and resolved into
single-sample views for post-processing. At 1x, the render views are sampled
directly. Resolve and post-processing views are initialized with clear passes to
avoid D3D12 validation complaints about first-use reads.

## Scene Pipelines

The scene pass uses a shared pipeline layout:

- Group 0: `SceneUniforms`.
- Group 1: UV checker texture and sampler.
- Group 2: IBL maps and sampler.

There are four scene pipelines:

- Mesh pipeline: triangle list, Reversed-Z depth write enabled, small negative
  depth bias to reduce coplanar wireframe z-fighting.
- Line pipeline: line list, no depth write, used by grid, wireframe, bounding
  box, normal lines, and UV wireframe.
- UV fill pipeline: triangle list, no depth write, used in the 2D UV viewport.
- Skybox pipeline: fullscreen triangle, always passes depth, no depth write.

All scene pipelines declare the same three color targets. The mesh/line/UV
pipelines alpha-blend scene color, bloom source, and ambient radiance in linear
light. The skybox writes color/bloom opaquely and leaves ambient at zero so SSAO
never darkens the background.

## Scene Shader

`scene.wgsl` handles several paths in one shader module:

- Zero-normal vertices are treated as overlays. They output their baked color and
  write no bloom or ambient contribution.
- `fs_ssao_gbuffer` writes view-space normal and view Z for the separate SSAO
  G-buffer pass.
- Source material color is decoded from sRGB to linear.
- UV checker mode samples an embedded checker texture.
- Vertex-color mode can display RGB, alpha as grayscale, or RGB plus alpha.
- Unlit mode emits flat material color and also writes linear color to the bloom
  source.
- Shaded mode uses either IBL/PBR or an analytic fallback light when IBL is off.
- Shaded mode writes both total linear radiance and the diffuse/fill ambient
  radiance SSAO may attenuate.
- Tone mapping and final sRGB encoding are handled once by the post pass.

The material model is currently simple: imported base color and smoothness are
baked into vertices. Metallic is fixed at 0, and there are no texture maps,
normal maps, alpha modes, or per-material draw groups.

## Image-Based Lighting

`ibl.rs` precomputes the maps used by shaded mode:

- Embedded HDR equirectangular texture is decoded to `Rgba16Float`.
- Equirectangular environment is rendered into a 256x256 cubemap.
- Diffuse irradiance cubemap is rendered at 32x32.
- Prefiltered specular cubemap is rendered at 128x128 with 5 mips.
- BRDF integration LUT is rendered at 512x512 in `Rg16Float`.

The scene shader samples irradiance, prefiltered specular, and the BRDF LUT for a
dielectric split-sum IBL approximation. The environment cubemap can also be drawn
as a skybox. IBL resources rebuild only when the selected environment map
changes.

## Bloom

Bloom uses `bloom.rs` and `bloom.wgsl`:

1. Bright-pass samples the scene's linear-HDR bloom MRT and writes over-threshold
   radiance into a half-resolution HDR texture.
2. A separable Gaussian blur ping-pongs between two half-resolution textures.
3. The post pass upsamples the blurred result and adds it to the linear scene
   color before tone mapping, scaled by the bloom intensity.

Bloom is 3D-only. The UV viewport disables it, and overlay line vertices write
zero to the bloom target so grid, wireframe, panels, and normal lines do not
glow.

## SSAO

SSAO uses `ssao.rs` and `ssao.wgsl`:

1. A mesh-only, single-sample pass writes view-space normal and view-space Z into
   the SSAO G-buffer.
2. The SSAO pass reconstructs view-space positions, rotates a deterministic
   16-sample hemisphere kernel per pixel, projects samples back to screen, and
   compares sampled depths.
3. The raw `R8Unorm` occlusion texture is bilateral-blurred with depth and normal
   weights into another full-resolution `R8Unorm` texture.
4. The post pass subtracts only the occluded portion of the ambient radiance, so
   direct and specular lighting are not darkened.

Radius and bias are stored in UI as fractions of the framed scene radius, then
scaled into view units by the renderer. Orthographic and perspective projection
reconstruction are both handled in the shader.

## Anti-Aliasing

The renderer has two independent AA layers:

- Scene MSAA: Off, 2x, 4x, 8x, or 16x, gated by adapter support for the HDR color
  and depth formats. Pipelines and scene targets rebuild when the effective
  sample count changes.
- FXAA: optional fullscreen edge blend in `post.wgsl`, run over the final
  display-space color after ambient AO, bloom, tone mapping, and sRGB encoding.

egui's own framebuffer uses fixed 4x MSAA, separate from scene MSAA.

## UV Viewport

UV mode reuses the same callback/resource system but switches to `SceneCallback::new_uv`:

- A 2D `UvCamera` produces an orthographic UV-space view projection.
- The 0..1 grid is static.
- UV wireframe is derived from original polygon topology and active UV channel.
- Filled UV triangles can be solid or per-island colored.
- Bloom and SSAO are disabled.

Texture mode runs its own minimal wgpu callback (`TexCallback` in
`render/src/tex.rs` + `tex.wgsl`), separate from the scene callback. `ui/src/
texture_view.rs` owns only interaction — pan/zoom/fit, the background fill, and the
channel pick — and hands the selected pooled image to the callback, which paints it
on the background layer behind the chrome. The callback uploads the source texture
once into a mip-mapped `Rgba8Unorm` texture (path-keyed LRU cache bounded to 16
entries, reused across frames + channel switches) and draws a fullscreen triangle that maps the
framebuffer pixel to an image UV from a placement uniform, discards fragments
outside the image (so the background fill shows through), and isolates the channel
by a `channel` uniform the fragment shader swizzles on — so RGB/R/G/B/A switches
are a buffer write, never a re-upload. Faithfulness across the sRGB seam: the
texture is `Rgba8Unorm`, so the sample returns the raw stored bytes for every
channel alike, and a `target_srgb` flag pre-compensates the framebuffer's
encode-on-write so the on-screen byte equals the source byte. This pipeline is
deliberately outside the scene's linear-HDR / tone-map / MRT path, so the pixels
are shown exactly as decoded — now with real mip minification (egui's single-level
upload shimmered when zoomed out).

## Resource Lifecycle

`SceneResources` is stored in egui callback resources and persists between
frames. The important sync points are:

- Output format change: recreate `SceneResources`.
- Framebuffer size or scene MSAA change: recreate scene targets, bloom targets,
  SSAO targets, and post bind group; rebuild scene pipelines when sample count
  changes.
- Model revision change: rebuild mesh buffers and clear derived debug buffers.
- UV channel change in 3D checker mode: rebuild only the mesh vertex buffer UVs.
- Debug toggles/color/length changes: rebuild or free only the relevant derived
  line buffer.
- Environment map change: rebuild IBL precomputed maps.
- UV mode changes: build/free UV wireframe and fill buffers on demand.

Steady-state frames should allocate no major GPU resources.

## What Is Good

- The offscreen target seam is already in place, which is the right foundation
  for post effects and future buffer visualization.
- MRT separation keeps display color, bloom source, and SSAO data independent.
- Scene MSAA, egui MSAA, and FXAA are correctly separated.
- Resource rebuilds are driven by explicit state changes rather than per-frame
  churn.
- Debug overlays are generated from original topology where it matters, so quads
  and n-gons do not display fake triangulation edges.
- Linear HDR scene color, IBL, bloom, SSAO, skybox, UV view, and debug lines all share a coherent
  renderer resource model.
- Adapter capability checks are present for optional/high-risk features.
- Scene depth uses `Depth32Float` Reversed-Z, with camera near/far fitting still
  used for near placement and orthographic range.

## What Can Be Better

- Reduce target memory pressure. Three `Rgba16Float` MRTs plus resolves and
  high MSAA can get expensive quickly. The ambient and SSAO G-buffer targets
  could use more compact encodings if format support and filtering requirements
  allow it.
- Add a real material/texture path. The renderer currently has one mesh draw and
  baked per-vertex material values. Texture loading, sampler/material bindings,
  normal maps, roughness/metallic, alpha handling, and per-material batching are
  still future work.
- Consider GPU or cached derived debug views for very large meshes. CPU line
  generation is simple and correct, but expensive overlays can still cost on
  dense production assets.
- Batch IBL precompute submissions if environment switching stutters. The current
  implementation submits many small passes, which is fine for MVP but may become
  visible on slower systems.
- Add renderer verification. There are no shader layout tests, no shader compile
  tests, no render golden tests, and no GPU timing diagnostics.
- Extend the Tex image viewer to compressed (DDS/KTX2) source textures, which the
  CPU `image` decode path does not cover.

## Near-Term Renderer Roadmap

1. Add tests around camera math, UV island generation, and `SceneUniforms` /
   `SceneVertex` layout assumptions.
2. Introduce texture/material resources and draw grouping.
3. Add in-app renderer diagnostics for target size, MSAA level, active passes,
   adapter/backend, and approximate render target memory.
4. Add renderer capture/golden tests for depth, HDR, bloom, and SSAO regressions.
