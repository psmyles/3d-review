# UX Audit — 3D Review (Rust)

## Context

Goal: a thorough end-user UX review of the native FBX-review viewer, with every
finding bucketed by **implementation difficulty** so the work can be tackled in
waves. This is an *audit deliverable* — findings + fix direction, not a committed
implementation. Pick a tier (or specific items) to implement next.

**Method & confidence.** Findings are derived from a close read of `crates/ui`,
`crates/app`, `crates/render/src/config.rs`, and the project docs. Interaction/
visual claims were cross-checked against the actual code; one dramatic
auto-finding was **disproved and dropped** — the viewport *does* render the model.
The `DORMANT (D3D11 migration)` comments in [overlay.rs](crates/ui/src/overlay.rs#L25)
and [texture_view.rs](crates/ui/src/texture_view.rs#L140) are explanatory seams;
the scene is drawn by `app` through D3D11 *before* egui paints chrome
([frame.rs:88](crates/app/src/frame.rs#L88)). Visual/interaction items below
should still get a manual GPU run-through to confirm (repo convention: GPU checks
are manual).

**Not issues — intentional by design (don't "fix"):** FBX-only file filter
([main.rs:713](crates/app/src/main.rs#L713)); WASD orbit firing on key *release*
(deliberate — one held key emits repeat key-downs that would snap with no
animation, one release = a single clean animated step,
[main.rs:860](crates/app/src/main.rs#L860)); no DDS/KTX2 import (engine-cooked
formats, out of scope).

---

## Tier 1 — Quick wins (localized; tooltips, labels, small state wiring)

Mostly text/affordance/feedback polish in `crates/ui` + a couple of one-line
`app` changes. High UX-per-effort.

1. **Help is one-shot and unrecoverable.** The startup overlay is dismissed by
   *any* key press and there's no way to bring it back
   ([main.rs:650](crates/app/src/main.rs#L650), [help.rs](crates/ui/src/help.rs)).
   Add a persistent `?`/help button (toolbar or status bar) + an `F1`/`H`
   shortcut to re-open it.
2. **Help omits mouse/right-click controls.** It lists key shortcuts but never
   mentions "right-click a tool button for options", drag gestures, or
   double-click-to-open ([help.rs](crates/ui/src/help.rs)). Add those lines.
3. **Right-click-opens-options is undiscoverable & inconsistent.** Only some
   buttons show the green-underline cue / "(right-click for options)" tooltip
   ([toolbar.rs:176](crates/ui/src/toolbar.rs#L176),
   [widgets.rs](crates/ui/src/widgets.rs)). Apply the same affordance to every
   option-bearing button.
4. **Dropdowns & the clickable zoom readout look inert.** `compact_combo` and the
   zoom % label have no caret/▼ or button styling
   ([widgets.rs](crates/ui/src/widgets.rs),
   [status_bar.rs:255](crates/ui/src/status_bar.rs#L255)). Add an interactive cue.
5. **Slider values lack units/meaning.** IBL Intensity (multiplier), Environment
   Rotation (degrees), GTAO Radius (fraction of scene radius), Normal length all
   show bare numbers ([environment.rs](crates/ui/src/panels/environment.rs),
   [gtao.rs](crates/ui/src/panels/gtao.rs),
   [uv_checker.rs](crates/ui/src/panels/uv_checker.rs)). Add value suffixes /
   tooltips.
6. **Failure toasts give no reason.** Load failure shows only "Couldn't load X";
   the real `ImportError` goes to Tracy only
   ([main.rs:776](crates/app/src/main.rs#L776),
   [texture_manager.rs](crates/app/src/texture_manager.rs)). Append a short cause
   (unsupported / not found / parse error) from the existing error.
7. **Unsupported dropped file → silent model-load attempt.** A non-image drop is
   assumed to be a model and fails generically
   ([main.rs:659](crates/app/src/main.rs#L659)). Detect unknown extensions and say
   "Unsupported file type".
8. **No empty-state guidance in the 3D viewport.** After the help overlay is
   dismissed with no model, the user faces a blank scene. Add a persistent
   centered "Drop an FBX here · Ctrl+O to open" hint while no model is loaded
   ([overlay.rs](crates/ui/src/overlay.rs)).
9. **Texture mode empty state is blank.** Picker hides when the pool is empty
   ([status_bar.rs:527](crates/ui/src/status_bar.rs#L527),
   [texture_view.rs:44](crates/ui/src/texture_view.rs#L44)). Strengthen
   `draw_empty_hint` with an actionable message.
10. **Escape-clear isn't undoable** though Outliner-click selection is
    ([main.rs:796](crates/app/src/main.rs#L796), [undo.rs](crates/app/src/undo.rs)).
    Route Escape's clear through the same snapshot path.
11. **Unlabeled affordances:** Outliner visibility checkbox has no tooltip
    ([outliner.rs:60](crates/ui/src/panels/outliner.rs#L60)); panel color swatches
    have no legend/tooltip ([wireframe.rs](crates/ui/src/panels/wireframe.rs),
    [bounding_box.rs](crates/ui/src/panels/bounding_box.rs),
    [normals.rs](crates/ui/src/panels/normals.rs)).
12. **Opaque labels:** tonemap "Method" → "Operator / Tone curve"
    ([tonemap.rs:32](crates/ui/src/panels/tonemap.rs#L32)); "Buffers" needs a
    clarifying tooltip/rename ([toolbar.rs:311](crates/ui/src/toolbar.rs#L311)).
13. **Stats overlay contrast.** Muted text on a translucent background can wash out
    over bright renders ([stats.rs:105](crates/ui/src/stats.rs#L105)). Raise
    contrast or add a subtle shadow/opaque chip.

## Tier 2 — Moderate (new small UI elements, feedback systems, state wiring)

Days of work each; touch `ui` + `app` together but no new rendering subsystem.

1. **No loading feedback; large imports freeze the UI.** FBX `load_model` runs on
   the main thread ([main.rs:729](crates/app/src/main.rs#L729)) with no spinner —
   the window looks hung on big files. Add a "Loading…" overlay + busy cursor, and
   ideally move import off-thread (texture decode already is).
2. **Silent destructive resets on model load.** Texture pool and undo history are
   wiped with no notice ([main.rs:753](crates/app/src/main.rs#L753),
   [main.rs:756](crates/app/src/main.rs#L756)). Add a toast ("History & textures
   reset for new model").
3. **Finish the Outliner search field.** `outliner_filter` state exists but is
   never rendered or read ([state.rs:665](crates/ui/src/state.rs#L665) — dead
   plumbing); wire a filter box in [outliner.rs](crates/ui/src/panels/outliner.rs).
   Large scenes are hard to navigate without it.
4. **Workspace mode is invisible.** 3D/UV/Tex change F/R semantics with no active
   indicator ([main.rs:835](crates/app/src/main.rs#L835)). Add a clear
   mode label/segmented control and reflect it in help.
5. **Surface importer warnings.** Non-fatal import issues (the data model carries
   `ModelWarning`) are never shown. Add a warnings toast/panel.
6. **Inconsistent toolbar semantics.** Radio-group shading vs independent toggles
   sit together with no separator ([toolbar.rs:164](crates/ui/src/toolbar.rs#L164));
   some buttons "activate-or-cycle" on left-click, others plain-toggle
   ([toolbar.rs:265](crates/ui/src/toolbar.rs#L265)). Add group separators/labels
   and a "click to cycle" hint.
7. **Discover camera/texture resets.** Gizmo reset-view button only appears on
   hover with no hint ([gizmo.rs:136](crates/ui/src/gizmo.rs#L136)); Tex
   double-click-to-fit and clickable zoom toggle are undiscovered
   ([texture_view.rs:83](crates/ui/src/texture_view.rs#L83)). Add a persistent
   home button / "Fit" button + tooltips.
8. **Per-material "revert to imported"** — only global Ctrl+Z exists; add an
   in-panel revert ([inspector.rs](crates/ui/src/panels/inspector.rs)).
9. **Disk auto-reload can fail silently** (permissions/limits) — texture edits then
   don't live-reload ([texture_manager.rs](crates/app/src/texture_manager.rs)). Add
   a one-time warning toast + a manual "Reload" button.
10. **"Restore defaults" for panels** — only per-panel resets exist; add a single
    reset-all and/or settings-persistence (see Tier 3).
11. **Allow typing exact slider values** in tight-range panels (Slider already
    supports it — ensure enabled, esp. Normal length 0.001–0.10).

## Tier 3 — Substantial / architectural (new subsystems)

Weeks each; several are already documented gaps (`TODO.md`, `PROJECT_STATE.md`).

1. **Viewport picking** — click-to-select a mesh part (and hover-to-inspect
   face/material). Selection is Outliner-only today. Unlocks frame-on-click,
   isolate, and measurement. Reuse the existing BVH in
   [model/src/bvh.rs](crates/model/src/bvh.rs).
2. **In-app error/warning console** — a persistent, dismissible log (beyond
   transient toasts) with full load-error detail + import warnings. *(documented
   gap)*
3. **Screenshot / viewport image export** — `rhi` already has readback in
   `bake.rs`; wire a capture-to-PNG. *(documented TODO)*
4. **Measurement tools** (distance/angle) — expected in a review tool; only bbox
   dimension labels exist today.
5. **Session/settings persistence beyond window bounds** — remember shading / AA /
   background / IBL choices; add a recent-files list; optional project file. Extend
   the [window_state.rs](crates/app/src/window_state.rs) %APPDATA% pattern.
6. **Camera QoL** — FOV control *(TODO)*, undoable camera / saved view bookmarks,
   turntable & fly modes.
7. **Solo / isolate selection** (hide all but selected) — Outliner + render.
8. **Translucent draw-order fix** — single-forward alpha can blend out of order;
   opaque-first then back-to-front. *(documented Phase 7 TODO — visual correctness)*
9. **Extra debug views** — overdraw, UV stretch/overlap. *(documented gaps;
   compute-based, capability-gated per invariant 4)*
10. **Explicitly-deferred, but users will ask:** glTF/OBJ import and animation/
    skeleton playback. Decide whether to scope-in or document as out-of-scope in a
    user-visible way.

---

## Verification

- Findings are code-derived; before/after each implemented item, do a **manual GPU
  run-through** (`cargo run -p review-app`) to confirm the visual/interaction
  change, since GPU checks aren't in CI.
- Run `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all` before claiming any item done (per CLAUDE.md §3).
- For Tier 1 text/affordance items, confirm theme tokens are used (no inline
  literals — invariant 8) and that new strings read naturally in the running app.
