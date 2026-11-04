# 3D Review - Work Plan

**Status, November 2026:** the console tool has just been renamed from `review-cli` to
**`3d-review-cli`**. The work below hasn't started. It's ordered for a stretch of work on a Mac. Items
that need Windows are collected at the end.

## Context

Two designs touch this repository:

- **Asset Check** checks art assets committed to a Unity project against a technical artist's rules,
  and shows artists what is wrong, why, and how to fix it. 3D Review's part: `3d-review-cli` checks the
  models.
  - Design: `asset-check-plan.md` in [psmyles/fire-decode](https://github.com/psmyles/fire-decode)
    (private; it moves to the rules repo later). Decisions are numbered AC1-AC19; this repo's changes
    are AC13.
- **The fire-decode migration** makes fire-decode the decoder shared by fire, 3D Review and bite. 3D
  Review then decodes its textures through it (`texture()`), replacing `crates/render/src/texture.rs`'s
  decode paths and `crates/psd`.
  - Design: `fire-decode-plan.md` in psmyles/fire-decode; a copy is in fire's `docs/`. 3D Review's
    adoption is step 4, after fire-decode's first tag.

**Priority: Asset Check first.** Order across repositories:
1. `asset-rules` - its `docs/PLAN.md`;
2. this repo's A-items, alongside fire's `fire-cli`;
3. the Launcher's results table;
4. the migration.

## 0. Check the rename on the Mac first

The rename was made and tested on Windows, where the macOS code could only be compiled with
`cargo clippy --target aarch64-apple-darwin`, not run.

- **Bundle:** `scripts/build-mac.sh` produces `3D Review.app/Contents/MacOS/3d-review-cli`, signed.
  `scripts/dev-app.sh` copies it into a development bundle.
- **Install Command Line Tool** (`crates/shell-macos/src/cli_install.rs`):
  - it creates `/usr/local/bin/3d-review-cli`;
  - it removes an old `/usr/local/bin/review-cli` **only** when that link points into an app bundle.

  To test the removal, first make the old link by hand, e.g.
  `ln -s "/Applications/3D Review.app/Contents/MacOS/review-cli" /usr/local/bin/review-cli`. Try it both
  with and without the administrator prompt.
- **The launch notice** appears while no `3d-review-cli` is reachable.
- **`scripts/check.sh` passes.**
- **Release notes for the next version** announce the rename. Release 3D Review before the next
  Launcher release, whose 3D Review add-on now looks only for `3d-review-cli`.

## A. Asset Check: `3d-review-cli` changes (AC13)

The CLI's existing conventions stay: exit codes 0 / 1 / 2, an atomic `--report`, and lenient profile
loading (`docs/cli.md`).

1. **`note` on every rule** in the profile format.
   - Add it to `RuleConfig` (`crates/audit/src/profile/mod.rs`, around line 317), so a hand-written
     note survives a re-save from the profile editor. Today unknown fields are dropped silently.
   - The editor doesn't need to edit notes.
2. **`--results <file>` in the shared format** (AC10), written through asset-rules.
   - Each failing rule becomes a finding with:
     - rule id and severity;
     - title: the rule's display name;
     - **what**: the measured value against the threshold, with numbers, built from the result's
       `measured`, `threshold` and `total`;
     - **why** and **how**: `rule_why` / `rule_fix` in `crates/labels/src/audit.rs` (catalogue
       `labels-audit.ftl`), which exist for all 43 checks but which only the viewer shows today;
     - the profile's `note`;
     - `measured` / `limit`;
     - `objects`: the offenders' names;
     - `fix: null`.
   - An unreadable model gets `status: unreadable` with its problem.
   - `--report` stays as it is.
3. **`--rules <rules.json>`** through asset-rules.
   - For each model it resolves the profile and stage, and loads that rules file with the existing
     profile loader.
   - `profile`, `stage` and `stage_from` go into the results, and a name that matches no profile
     becomes the `file.no_profile` finding.
   - `--profile` keeps working for direct use. Giving both is a usage error (exit 2).
   - `rules check` and `rules explain` come from asset-rules.
4. **`--staged`** (staged files through asset-rules' git helpers, with its cache) and
   **`--files-from <list>`** (one path per line). Windows limits a command line to about 32,000
   characters. Only `.fbx` files are taken; other staged files are ignored.
5. **`docs/cli.md`** documents all of the above, and **`crates/cli/tests/cli.rs`** covers each.

- **Dependency:** `asset-rules` by git tag once `v0.1.0` exists. Until then, an untracked
  `.cargo/config.toml`:

  ```toml
  [patch."https://github.com/psmyles/asset-rules"]
  asset-rules = { path = "../asset-rules" }
  ```

*Done when* Asset Check step 2's gate passes for this half:
- it gives the same file the same profile and stage as `fire-cli`;
- its results validate against the schema;
- a commit with typical models is checked quickly, and almost instantly when cached.

## B. The fire-decode migration: 3D Review's side, later

1. **`links = "fire_psd"` in `crates/psd/Cargo.toml`** (migration step 0).
   - Both PSD crates export the same `fire_psd_*` symbols from a static library named `fire_psd`, and
     neither declares `links`, so cargo would let both into one binary without complaint.
   - Cheap; do it any time.
2. **macOS baselines (step 0):**
   - the startup gate (`scripts/gate.sh`);
   - the sizes of the app binary and `libreview_quicklook.dylib`.
3. **Step 4: adopt `texture()`.** This waits for fire-decode's first tag, which needs the Windows HEIF
   libraries. It covers:
   - `heif` on in every binary (viewer, shell DLL, Quick Look dylib, snapshot);
   - deleting `crates/psd` in the same commit;
   - release switching to `panic = "unwind"`, with the decode-only abort hook in the viewer (shells
     keep their guards);
   - a decode worker pool replacing thread-per-texture;
   - licences: the first LGPL component, with notices covering the DLL and dylib;
   - README / D22 / invariant 9 / the DDS non-goal reworded.

   The gates are in the fire-decode plan's step 4.

## Needs Windows (later)

- **The upgrade check:** Setup's new `[InstallDelete]` removes the old `review-cli.exe` from the
  install folder, so verify it.
- **Windows baselines:** startup gate, sizes and import tables of `3d-review.exe` and
  `review_shell.dll`.
- **The static CRT (FD20)** for every binary, checked by import tables with no `MSVCP140` /
  `VCRUNTIME140*` entries.

## References

- [`docs/cli.md`](cli.md): the CLI reference.
- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) and [`docs/GOTCHAS.md`](GOTCHAS.md): the committed project
  rules.
- In psmyles/fire-decode: `asset-check-plan.md` (AC13 is this repo) and `fire-decode-plan.md` (step 4
  is this repo).
- `docs/PLAN.md` in psmyles/asset-rules, psmyles/fire and psmyles/launcher.
