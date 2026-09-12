# Code quality analysis

Reviewed: 12 September 2026  
Project: 3D Review, Rust workspace  
Revision: `07e5678` (working tree)

## Overall assessment

This is a thoughtfully structured native application with strong separation between model data, import/export, rendering, UI, and application coordination. Its best qualities are explicit architectural constraints, careful preservation of source asset data, meaningful regression tests, and performance-conscious resource sharing. The current workspace passes its lint and test gates.

The main weaknesses are in lifecycle transitions and failure handling: settings crossing model boundaries, late asynchronous results, failed GPU allocations, and interrupted file writes. These deserve attention before a broad refactor. The existing architecture provides useful patterns for addressing most of them.

## Scope and validation

The review covered the nine-crate workspace, with targeted inspection of application loading/optimization/texture workflows, model and mesh validation, import FFI, export persistence, GPU resource wrappers and layouts, UI state, tests, build scripts, and architecture documentation. Native bridges and vendored implementation details were sampled where necessary to establish a finding; this was not an exhaustive audit of third-party libraries.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. Stable rustfmt emitted warnings about nightly-only settings in the vendored Sokol configuration. No formatting was applied. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed. |
| `cargo test --workspace --locked --offline -- --nocapture` | Passed: **367 tests passed, 0 failed, 2 ignored documentation examples**. No fixture-skip messages were observed. |
| `packaging/check-shader-bytecode.ps1` | Passed; reported current bytecode for 30 programs. Checked before building to avoid regeneration of stale shaders. |

Validation ran on Windows with Cargo 1.98.1. Test output is retained locally in [target/code-quality-tests.log](D:/Dev/3d-review-rs/target/code-quality-tests.log). No source code, manifests, shaders, or tests were edited. The pre-existing modification to `docs/TODO.md` was left untouched.

The findings below are based on code inspection unless explicitly described as a check result. Interactive race conditions, GPU allocation failures, interrupted writes, macOS behavior, and release performance were not reproduced during this review. Passing tests do not establish coverage of those scenarios. No dependency vulnerability audit or coverage measurement was performed.

## What's good

### 1. Clear, useful architectural boundaries

`review-model` depends only on `glam`, keeping geometry and animation independent of windowing and GPU APIs. `app`, `model`, and `ui` forbid unsafe code. UI actions are represented as values in `UiOutput`, and application code applies them to the renderer. These are enforceable boundaries that reduce the scope of platform changes and make pure logic easier to test.

Evidence: [model manifest](D:/Dev/3d-review-rs/crates/model/Cargo.toml), [model safety policy](D:/Dev/3d-review-rs/crates/model/src/lib.rs:3), [application safety policy](D:/Dev/3d-review-rs/crates/app/src/main.rs:6), [UI intents](D:/Dev/3d-review-rs/crates/ui/src/state/mod.rs:106).

### 2. Strong attention to asset fidelity and input validation

The data model preserves authored topology, deformation data, source properties, and statistics instead of treating the rendering mesh as the complete asset. Meshoptimizer wrappers check triangle counts, index ranges, stream lengths, and allocation arithmetic. PSD decoding has dimension and output-size limits, plus an owning handle that frees the native document on early returns. The C import bridge includes explicit multiplication-overflow guards.

Evidence: [mesh validation](D:/Dev/3d-review-rs/crates/optimize/src/meshopt/mod.rs:123), [PSD ownership and limits](D:/Dev/3d-review-rs/crates/psd/src/lib.rs:71), [import allocation checks](D:/Dev/3d-review-rs/crates/import/src/ufbx_bridge.c:47), [source extras validation](D:/Dev/3d-review-rs/crates/model/src/extras.rs).

### 3. Performance choices are reflected in the design

Model data and expensive undo state are shared through `Arc`; undo history is capped and continuous edits are coalesced. Model loading publishes drawable data before completing secondary measurements. Optimization permits one processing run at a time and coalesces edits while it is busy. Redraw scheduling sleeps when there is no work rather than continuously rendering an idle scene.

These are sensible mechanisms, not proof of a particular performance level. The repository also supplies profiling and comparative performance-gate tooling for obtaining that evidence.

Evidence: [undo snapshots](D:/Dev/3d-review-rs/crates/app/src/undo.rs), [staged loading](D:/Dev/3d-review-rs/crates/app/src/loading.rs), [optimization scheduling](D:/Dev/3d-review-rs/crates/app/src/opt.rs:189), [redraw scheduler](D:/Dev/3d-review-rs/crates/app/src/redraw.rs), [performance gate](D:/Dev/3d-review-rs/scripts/gate.ps1).

### 4. Tests exercise meaningful behavior

The suite includes real FBX fixtures, export/reimport round trips, animation and deformation preservation, topology checks, PSD decoding, UI interactions, and regressions for previously broken optimization scheduling. The real-model optimization suite explicitly checks that its five named fixtures exist, which prevents a fixture rename from silently disabling those tests.

Evidence: [export round trips](D:/Dev/3d-review-rs/crates/optimize/tests/export_round_trip.rs), [real-model tests](D:/Dev/3d-review-rs/crates/optimize/tests/real_models.rs), [fixture existence check](D:/Dev/3d-review-rs/crates/optimize/tests/real_models.rs:530), [UI stats interactions](D:/Dev/3d-review-rs/crates/ui/tests/stats_rows.rs), [optimization scheduling regressions](D:/Dev/3d-review-rs/crates/app/src/opt.rs).

### 5. Rendering contracts and build hygiene have concrete safeguards

GPU structs use `repr(C)` and `Pod`/`Zeroable`, with compile-time size checks against generated shader layouts. Shader packaging verifies content hashes, rather than relying only on timestamps. Dependencies are centralized, the lockfile is committed, Clippy policy is inherited from the workspace, and vendored dependencies have provenance documentation.

Evidence: [GPU layout assertions](D:/Dev/3d-review-rs/crates/render/src/scene/gpu_types.rs:47), [shader freshness check](D:/Dev/3d-review-rs/packaging/check-shader-bytecode.ps1), [workspace manifest](D:/Dev/3d-review-rs/Cargo.toml), [vendor notice](D:/Dev/3d-review-rs/vendor/NOTICE.txt).

## What's bad: actionable findings

Priority definitions: **P1** = address first because asset output or existing files can be affected; **P2** = correctness, reliability, or enforcement weakness; **P3** = maintainability improvement. Priority reflects impact, not a claim that the issue occurs on every run.

### F1 — P1: Per-object overrides survive a change of model identity

**Evidence:** [reset_opt_for_new_model](D:/Dev/3d-review-rs/crates/app/src/opt.rs:635) calls [clamp_to_model](D:/Dev/3d-review-rs/crates/optimize/src/stack/mod.rs:159), which retains every override whose numeric node index is below the new node count. [Preset loading](D:/Dev/3d-review-rs/crates/app/src/opt.rs:614) uses the same check.

**Failure scenario:** Exclude node 2 in model A, then open unrelated model B with at least three nodes. The override is still valid numerically and can exclude B's node 2 from processing, although it represents a different object. The comments promise to prevent this cross-model carryover, but bounds checking does not establish identity.

**Improvement:** Clear per-object overrides when replacing the model while preserving general operations. If presets should transfer object-specific settings, bind them to validated source identity or explicitly resolved object paths, with unresolved mappings reported to the user.

**Regression test:** Replace a model with a same-sized node table containing different objects and assert that no object-specific exclusion or parameter override transfers implicitly.

### F2 — P1: Export writes directly over destination files

**Evidence:** [export_fbx](D:/Dev/3d-review-rs/crates/optimize/src/export/mod.rs:130) writes directly to the requested path; the per-LOD branch writes final paths sequentially at line 154. The [C export bridge](D:/Dev/3d-review-rs/crates/optimize/src/export_bridge.c:1554) calls the vendored writer, whose [file-open implementation](D:/Dev/3d-review-rs/third_party/ufbx-write/ufbx_write.c:12406) uses `fopen(..., "wb")`.

**Failure scenario:** Replacing an existing export truncates it before the new write completes. Disk exhaustion or process termination can leave a damaged replacement. In a multi-file export, a later error leaves earlier files updated while the function returns only an error, without its accumulated report.

**Improvement:** Write each output to a temporary sibling, verify successful completion and close, then replace its destination. Plan the entire LOD output set before writing and define recovery/reporting for partial replacement. A collection of renames is not automatically an atomic multi-file transaction. Apply the same persistence discipline to saved presets, which currently use direct `std::fs::write` in `app/src/opt.rs`.

**Regression test:** Inject a write failure and verify that a pre-existing destination remains intact; fail a later LOD and verify that the user receives an accurate account of completed and incomplete outputs.

### F3 — P2: Texture decode results have no scene or request generation

**Evidence:** [TextureDecode and its request](D:/Dev/3d-review-rs/crates/app/src/texture_manager.rs:28) carry a path and result but no identity token. [handle_texture_decoded](D:/Dev/3d-review-rs/crates/app/src/texture_manager.rs:208) always inserts successful results into the current cache. [reset_texture_state](D:/Dev/3d-review-rs/crates/app/src/texture_manager.rs:342) clears state without invalidating pending messages.

**Failure scenarios:** A slow image import finishing after opening a different model or resetting the scene can repopulate the new scene's texture pool and watches. Two reloads of the same file can also finish out of order, allowing an older decoded image to replace a newer one.

**Improvement:** Carry a scene generation plus a per-path request revision, accept only matching results, and balance activity notifications even for discarded results. The model-loading generation checks provide an existing pattern to follow.

**Regression tests:** Deliver a decode after scene reset; deliver two reload completions in reverse order; assert that only the intended scene and latest image are updated.

### F4 — P2: Some failed GPU allocations lose their handles without cleanup

**Evidence:** [TransientBuffer creation](D:/Dev/3d-review-rs/crates/render/src/rhi/buffer.rs:55), [immutable buffer creation](D:/Dev/3d-review-rs/crates/render/src/rhi/buffer.rs:341), and [sampler creation](D:/Dev/3d-review-rs/crates/render/src/rhi/sampler.rs:89) call `make_*`, then propagate `require_valid(...)?` before constructing an owner. The [vendored buffer destroy path](D:/Dev/3d-review-rs/vendor/sokol-rust/src/sokol/c/sokol_gfx.h:27016) explicitly handles failed resources and returns their slots to the pool.

**Impact:** When creation returns a failed resource with an allocated handle, the error return bypasses destruction. Repeated failures can consume resource-pool slots and make recovery worse. This is a failure-path issue; it does not imply ordinary successful allocations leak.

**Improvement:** Establish an owning guard before validation, or destroy the handle explicitly on error. [Render-target construction](D:/Dev/3d-review-rs/crates/render/src/rhi/target.rs:101) already demonstrates the first approach, and texture/pipeline constructors contain explicit failure cleanup.

**Regression test:** Under an injected failed-resource result, verify one destruction per allocated handle and no degradation after repeated attempts.

### F5 — P2: Import scene ownership is not protected during unwinding

**Evidence:** [load_fbx](D:/Dev/3d-review-rs/crates/import/src/ffi/bridge.rs:151) marshals the native scene and frees it manually afterward. [model_from_bridge_scene](D:/Dev/3d-review-rs/crates/import/src/ffi/marshal_model.rs:429) calls the caller-provided progress sink directly. Unlike the C progress trampoline, this Rust-side callback has no panic guard.

**Failure scenario:** In an unwinding build, a progress sink that panics during the Building stage bypasses `review_import_free_scene`, leaking the native scene if the caller catches the panic. Ordinary `Result::Err` paths are correctly freed. The release profile uses `panic = "abort"`, so this finding should not be misread as a promise of recoverable release panics.

**Improvement:** Wrap the native scene in a `Drop` owner immediately after a successful C call, as already done for extras and PSD documents. Document one consistent policy for progress-callback panics.

**Regression test:** In an unwinding test build, use a progress sink that panics during marshaling and a cleanup counter to verify that the scene is freed exactly once.

### F6 — P2: The raw-slice helper's safe signature overstates its guarantees

**Evidence:** [checked_slice](D:/Dev/3d-review-rs/crates/import/src/ffi/raw.rs:46) accepts a raw pointer and returns `&'a [T]` for a caller-selected lifetime without borrowing the scene owner. It checks null/empty cases, but cannot establish allocation extent, alignment, initialization, or lifetime from that signature.

**Impact:** The current internal call sites rely on the native scene staying alive, but the type system does not enforce that relationship. A future safe call inside the module can request an overlong lifetime or pass an invalid non-null pointer. This is an unsound internal abstraction, not evidence that the current application has a reproduced use-after-free or that the helper is publicly exposed.

**Improvement:** Place raw conversion inside an explicitly unsafe boundary with documented preconditions, then expose safe field accessors whose returned slices borrow an owning scene wrapper. This preserves safe marshaling code while making the ownership contract enforceable. Apply the same reasoning to raw C-string conversion.

### F7 — P2: Superseded imports and texture reloads can accumulate background work

**Evidence:** [model loading](D:/Dev/3d-review-rs/crates/app/src/loading.rs:190) and [texture decoding](D:/Dev/3d-review-rs/crates/app/src/texture_manager.rs:192) spawn a thread per request. Model generations discard stale results, but workers still continue parsing, marshaling extras, and measuring bounds/statistics. Texture file events can independently start repeated decodes.

**Impact:** Rapidly opening large models or receiving a burst of image-change notifications can keep multiple expensive jobs and their allocations alive. Ignoring a result prevents stale UI state but does not recover the CPU time or peak memory spent producing it.

**Improvement:** Bound worker concurrency, coalesce reloads by path, and check cancellation between costly import stages. Where the native parser supports cancellation, propagate it through the existing progress boundary. Reuse the optimization subsystem's one-in-flight/latest-dirty approach where appropriate.

**Regression test:** Submit many replacements or file events, assert a bounded number of active workers, and verify that obsolete work stops before optional measurements.

### F8 — P2: Quality gates are documented but not fully enforced by the repository

**Evidence:** No checked-in `.github` workflows were found. [Fixture helpers](D:/Dev/3d-review-rs/crates/optimize/tests/common/mod.rs:48) and the [PSD integration test](D:/Dev/3d-review-rs/crates/psd/tests/real_psd.rs) can return successfully when assets or native capabilities are missing. Some suites are compiled out behind capability flags. The real-model fixture existence test is a useful partial safeguard, and no fixture skips occurred in this run.

**Impact:** Another checkout or external build can report success while exercising less functionality. This review cannot establish whether private CI exists elsewhere. [rust-toolchain.toml](D:/Dev/3d-review-rs/rust-toolchain.toml) also selects moving `stable`, so the stated Rust 1.88 minimum is not tested by the successful run on this machine.

**Improvement:** Make a required Windows/macOS validation matrix visible in the repository or document the external equivalent. Require the fixture inventory and native capabilities for full integration jobs; test reduced-capability builds separately. Run a minimum-supported-toolchain job and record the exact release toolchain. Include shader freshness checks and a small rendering smoke test on suitable GPU runners.

## What can be improved further

### Focus tests on transitions and failures

Keep the existing real-asset tests, and add the lifecycle scenarios listed above. Extract small state-transition functions or injectable worker/resource boundaries where needed; this makes races and failures deterministic without requiring a full interactive session. Add bounded malformed-input/property tests for parallel geometry tables and native bridge payloads. Native fuzzing or sanitizer runs would complement the current Rust tests, but none were run here.

### Consolidate and refresh architecture documentation — P3

Documentation explains many design decisions well, but some comments still point to removed implementation details. For example, [gpu_types.rs](D:/Dev/3d-review-rs/crates/render/src/scene/gpu_types.rs:1) refers to `super::d3d`, `scene.hlsl`, and `tex_d3d.rs`, while the active shader source is `review.glsl`. [ARCHITECTURE.md](D:/Dev/3d-review-rs/docs/ARCHITECTURE.md:609) describes roughly 200-line platform leaves even though the current Windows leaf is substantially larger.

Prefer one maintained architecture map, link to it from local comments, and describe ownership or invariants beside the relevant code. Remove obsolete migration references rather than adding another layer of historical explanation. This will make the existing documentation more trustworthy without reducing useful rationale.

### Keep modules cohesive rather than optimizing for line count

Large files such as animation, AO, extras marshaling, and export writing carry real domain complexity and often include tests. Their size alone is not a defect. Preserve the export writer's documented lifetime chain; splitting it mechanically could obscure ownership. Extract independently testable phases, validation, or state transitions where doing so reduces reasoning effort, especially around the app's asynchronous workflows.

### Make memory budgets explicit

The undo stack's entry limit and shared images are good foundations, but 128 snapshots do not bound retained bytes when edited textures are large. Add measured CPU/GPU memory accounting and use it to decide whether texture history or caches need byte budgets. Prioritize worker bounds first, since concurrent imports and decodes can multiply memory use even when each input passes its individual size checks.

### Strengthen contract checks where they buy real protection

GPU size assertions catch many layout mistakes, but equal total size does not prove equal field offsets. Add offset/attribute checks where reflection provides stable information. For hand-maintained C/Rust bridge structures, add targeted ABI size/alignment checks or generated comparison probes. Prefer these specific checks over broad lint suppression or defensive copying.

## Suggested order of work

| Order | Work | Desired outcome |
| --- | --- | --- |
| 1 | Fix model-bound overrides and safe export replacement (F1–F2). | Optimization acts on the intended objects; failed writes preserve existing assets. |
| 2 | Add texture generations and correct failed GPU cleanup (F3–F4). | Late work cannot contaminate a new scene; allocation errors do not consume handles. |
| 3 | Introduce scene ownership guards and lifetime-bound accessors (F5–F6). | Native ownership is enforced rather than dependent on manual call ordering. |
| 4 | Bound/cancel obsolete background work (F7). | Repeated user actions have predictable CPU and memory cost. |
| 5 | Enforce cross-platform checks and full fixture execution (F8). | A green build has a consistent, documented meaning. |
| 6 | Refresh documentation and add measured memory/layout checks. | Further development remains understandable and regression-resistant. |

The project has a strong foundation. The most valuable next step is to extend its existing ownership, generation, and validation patterns consistently across the remaining failure and transition paths.
