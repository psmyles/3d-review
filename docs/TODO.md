3D view
- viewport background controls
- buffer views
- UV stretching
- overdraw
- texture name auto matching
- camera FOV controls

UV view
- wireframe color
- uv overlap

Texture
- flipbook player

Tracy profiler integration

Phase 7 — Opacity polish: alpha draw order (last, honestly approximate)
The scene pass is single forward + alpha-blended MRT, so translucent materials can blend out of order.

Split the per-material loop into opaque first (depth write on) then translucent back-to-front by per-range centroid (depth test on, write off); cutout stays opaque. One extra write-off pipeline; centroids computed once on load.
Risk: not true OIT — interpenetrating translucency is wrong; acceptable for an auditor, flagged in code + UI. Done last so everything else ships without it.
✋ User-verifiable checkpoint (final acceptance): the user loads a model with overlapping translucent materials and confirms they blend in a sensible back-to-front order (no obvious pop-through for non-interpenetrating geometry), with cutout materials still crisp. cargo clippy clean. This closes the Materials & Textures milestone.



Phase 5: MSAA + ResolveSubresource, profiler, capability gates; remove runtime wgpu/naga/pollster + clippy clean

Phase 6: port offline bake tool (ibl.wgsl->HLSL) off wgpu; zero wgpu workspace-wide