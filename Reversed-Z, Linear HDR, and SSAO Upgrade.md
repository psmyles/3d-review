# Reversed-Z, Linear HDR, and SSAO Upgrade

## Summary
Implement the renderer as a fully linear HDR scene pipeline with Reversed-Z scene depth and higher-quality SSAO. The scene pass will output linear radiance, post will apply ambient-only AO, bloom, tone mapping, final sRGB encoding, and FXAA. SSAO will use a separate single-sample G-buffer plus bilateral blur to avoid MSAA edge averaging artifacts.

## Implementation Changes
- **Depth convention**
  - Split depth formats: keep egui/composite depth on its own `EGUI_DEPTH_FORMAT`, and change only scene depth to `Depth32Float`.
  - Use Reversed-Z for scene and SSAO G-buffer passes: clear depth to `0.0`, use `GreaterEqual`, and flip mesh depth bias to negative values.
  - Use `Mat4::perspective_infinite_reverse_rh(fov, aspect, near)` for perspective; use finite reversed orthographic by swapping near/far in `orthographic_rh`.
  - Keep the current near/far fit for near-plane placement and orthographic far range; perspective no longer clips at far.
  - Update skybox ray reconstruction for infinite reversed perspective by unprojecting a near-plane point, with an `is_ortho` uniform branch for orthographic.

- **Linear HDR migration**
  - Change scene MRT location 0 from display-space color to linear HDR scene radiance.
  - Move PBR Neutral tone mapping and final `linear_to_srgb` into `post.wgsl`.
  - Keep the bloom source target linear HDR and keep overlays writing zero bloom.
  - Replace the current scene-pass G-buffer MRT with an “AO-eligible ambient” MRT: IBL diffuse ambient and analytic ambient/fill only; zero for overlays, skybox, unlit, direct light, and specular.
  - Output overlay RGB as linearized color with existing alpha, accepting small look changes and only retuning constants if a regression is obvious.

- **Ambient-only SSAO**
  - In post, compose AO as `scene - ambient * (1 - ao)`, clamped non-negative, then add bloom and tone-map.
  - For IBL shaded mode, AO affects `kd * irradiance * albedo * intensity`; specular reflection is untouched.
  - For analytic fallback, AO affects hemisphere/fill ambient terms; direct diffuse and Blinn-Phong specular are untouched.
  - Unlit, wire-only, UV, skybox, and debug overlays are not darkened by SSAO.

- **SSAO edge quality**
  - Add a separate full-resolution, single-sample SSAO G-buffer target: `Rgba16Float` normal/view-Z plus a single-sample Reversed-Z depth target.
  - Render the SSAO G-buffer in its own mesh-only pass when SSAO is active, using the same camera uniform and no overlays.
  - Update SSAO bind groups so the occlusion pass reads the single-sample G-buffer, not an MSAA-resolved MRT.
  - Replace the box blur with one 5x5 bilateral blur over raw AO, weighting by spatial distance, view-Z delta, and normal agreement. Use depth sigma derived from SSAO radius/bias and a normal cutoff around `dot(n0, n1) >= 0.75`.

## API / Type Changes
- Add/export `EGUI_DEPTH_FORMAT` for `app`/`PostPass`; keep `SCENE_DEPTH_FORMAT` for offscreen scene targets and set it to `Depth32Float`.
- Update `supported_msaa_levels` to gate scene MSAA against `Depth32Float`; unsupported higher MSAA levels may disappear from the UI.
- Update `SceneUniforms`/WGSL layout with a projection-mode flag for skybox reconstruction.
- Update `SceneTargets` so MRT location 2 is ambient-eligible radiance, and add/rebuild separate SSAO G-buffer/depth targets with the AO textures.
- Update `RENDERING_PIPELINE.md` to describe the new linear HDR, Reversed-Z, ambient-AO, and SSAO G-buffer flow.

## Test Plan
- Add camera/projection unit tests: perspective reversed near maps to depth `1`, distant points approach `0`, and orthographic near/far map to `1/0`.
- Run `cargo fmt --check`, `cargo check --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- GPU verify with `review-app`: shaded/unlit/wireframe, UV viewport, IBL skybox, bloom, SSAO, FXAA, MSAA Off/2x/4x/8x/16x where supported, resize, ortho/perspective, and overlay toggles.
- Visual acceptance: no close-face depth flicker, no obvious near clipping, bloom still responds to HDR highlights, SSAO no longer darkens specular/direct highlights, and SSAO halos at silhouettes/creases are reduced.

## Assumptions
- Chosen depth strategy: `Depth32Float` + infinite reversed perspective.
- Chosen SSAO strategy: separate single-sample G-buffer plus bilateral blur.
- Chosen visual priority: linear-correct rendering over exact gamma-era visual matching.
