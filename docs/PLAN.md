# 3D Review - Work Plan

**Status, 4 November 2026:** the Windows half is done.
- **A (AC13) is done:** `note`, `--results`, `--rules`, `--staged`, `--files-from`, the `rules`
  commands, `docs/cli.md`, and the tests in `crates/cli/tests/cli.rs`. Its gate is met on Windows
  (below).
- **B.1 is done**, and so are the **Windows** items: the step 0 baselines, the static CRT (FD20,
  recorded as D25), and the upgrade check, which includes a real upgrade over an installed 0.8.0.
- **Still to do:** section 0 and B.2 on the Mac; B.3, which waits for fire-decode's first tag.

The console tool was renamed from `review-cli` to **`3d-review-cli`** just before this work.

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
3. the Launcher's results table (done), then Asset Check's rules repo;
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
- **`scripts/check.sh` passes.** This now includes A's tests, which make git repositories under
  `target/tmp` and run the CLI in them: `--staged`, the cache, `--rules` on two branches, and the
  `rules` commands. Only Windows has run them so far.
- **The bundle's `3d-review-cli` runs from a hook:** `audit --staged --rules ...` in a project
  repository, started by `git commit`. Hooks there get a minimal `PATH`, so this checks that `git`
  is found (AC19).
- **Release notes for the next version** announce the rename. Release 3D Review before the next
  Launcher release, whose 3D Review add-on now looks only for `3d-review-cli`.
  - The next version is **0.9.0**: asset-rules' fixtures already ask for `3d-review-cli` 0.9.0.
    `product.json` still says 0.8.0.
  - A draft, for the release page:

    > **The console tool is now `3d-review-cli`** (it was `review-cli` up to 0.8.0). Scripts and
    > Launcher tools that call it by the old name need the new one. Setup removes the old
    > `review-cli.exe` from the install folder. On macOS, **Install Command Line Tool...** removes an
    > old `/usr/local/bin/review-cli` link into the app.
    >
    > **Asset Check.** `3d-review-cli audit` can check models against a technical artist's
    > `rules.json` (`--rules`): each model's profile and stage come from its name, its folder and the
    > branch. `--results` writes what is wrong, why it matters and how to fix it, with the figures.
    > `--staged` checks what is staged for a commit, and caches it. `--files-from` reads models from a
    > list, and `rules check` / `rules explain` help whoever writes the rules. Profile files take a
    > `note` on each check, and a hand-written one needs only the checks it changes.
    >
    > **No Visual C++ runtime needed.** 3D Review, `3d-review-cli` and the Explorer extension no
    > longer need the Visual C++ redistributable installed.

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

- **Dependency:** `asset-rules` by git tag `v0.1.0` (done). The repository's own
  `.cargo/config.toml` is committed now (the static CRT), so a local patch for unreleased
  asset-rules goes in a `.cargo/config.toml` one folder up, beside both checkouts (`docs/cli.md`).
  The same applies to fire, whose `.cargo/config.toml` is tracked too. asset-rules' README and plan,
  fire's plan and fire-decode's plan now say the same (5 November 2026).

*Done when* Asset Check step 2's gate passes for this half:
- it gives the same file the same profile and stage as `fire-cli`;
- its results validate against the schema;
- a commit with typical models is checked quickly, and almost instantly when cached.

**Done, 4 November 2026, on Windows.** The gate:
- **Same profile and stage:** both resolve through asset-rules' `resolve`, and
  `rules_give_each_model_the_profile_and_stage_asset_rules_resolves` checks every file's `profile` /
  `stage` / `stage_from` against `resolve` directly, on `main` (from the branch) and on a feature
  branch (from the profile, and from the project).
  - **Checked against `fire-cli` end to end** on Windows on 5 November 2026, with both release builds
    in one scratch repository (fixture `rules.json`, four models, three textures).
  - **`rules explain`:** identical output from both tools for every file, on `main`, `proto/x`, a
    feature branch and a detached HEAD.
  - **`--staged`:** both gave the same profile, stage and `stage_from` on new staged files.
  - **The one difference was expected:** `3d-review-cli` added a run-level `tool.too_old` warning,
    because it still reports 0.8.0 and the fixtures require 0.9.0.
- **Valid results:** every results test reads its file back through `asset_rules::results::Results::read`,
  which refuses anything not shaped as `asset-check.results` v1. asset-rules has no separate JSON
  Schema file to validate against.
- **Speed** (release build, 24 cores):
  - 20 staged models totalling 96 MB, two of them 24 MB scans: 2.1 s cold, 0.13 s cached.
  - The two scans alone are 1.3 s each, so a commit of typical 2-7 MB models is well under that.
  - Before models were checked in parallel, the same 20 took 4.7 s.

Decisions made while building it. `fire-cli` should match them:
- **The `rules` commands' syntax:** `rules check <rules.json>` (or `--rules`), and
  `rules explain <files>... --rules <rules.json>`. `check` exits 1 on any problem, warnings
  included; `explain` exits 0.
- **`--fail-on` defaults to `warning` with `--rules`** (AC10's exit convention). It stays `error`
  with `--profile`.
- **`--staged` excludes models on the line and `--files-from`** (exit 2). A commit with no models in
  it, or a list naming none, exits 0. Non-`.fbx` lines in a list are skipped.
- **Results are written with `--profile` too:** type `model`, the `--profile` value as `profile`, no
  stage, `advise`. The `rules.digest` is the profile's own.
- **A rule in a profile file needs only what it changes.** A listed rule is on unless it says
  `"enabled": false`. A missing severity is the base's, and a missing `base` is `generic`. Before
  this, a rule written as the AC12 sketch writes one (no `enabled`) fell back to its default whole,
  note included. asset-rules' own fixture rules files have no `base`.
- **The cache holds only `checked` entries** (asset-rules' rule). serde_json's
  `float_roundtrip` is needed, or a cached figure differs from a fresh one in its last digit
  (GOTCHAS).
- **The `--report` stays as it was**, plus `profile` / `profile_label` / `stage` / `stage_from` per
  entry with `--rules`, `cached` on a cache hit, and a top-level `rules`.

## B. The fire-decode migration: 3D Review's side, later

1. **`links = "fire_psd"` in `crates/psd/Cargo.toml`** (migration step 0). **Done.**
   - Both PSD crates export the same `fire_psd_*` symbols from a static library named `fire_psd`, and
     neither declares `links`, so cargo would let both into one binary without complaint.
   - fire's `psd-sdk-sys` still has no `links` key. That is fire-decode step 1.
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

## Needs Windows

- **The upgrade check:** Setup's new `[InstallDelete]` removes the old `review-cli.exe` from the
  install folder, so verify it.
  - **The directive works.** A throwaway installer was built with Inno Setup 6, carrying only that
    entry (no uninstaller, no registry). Run over a folder holding a `review-cli.exe` and another
    file, it removed `review-cli.exe`, kept the other file, and installed `3d-review-cli.exe`.
  - **The real upgrade works** (2026-11-04). The new Setup was run over a per-user 0.8.0 install
    with the associate, shellext and addtopath tasks. `review-cli.exe` is gone and
    `3d-review-cli.exe` is in its place. The installed `3d-review-cli` lists the new `rules`
    command. Every other file is kept, including `review-comments.exe`, which Setup does not
    ship. The tasks, the PATH entry and the Explorer `InprocServer32` paths are unchanged.
    None of the installed binaries imports `VCRUNTIME140*`, `MSVCP140` or `api-ms-win-crt-*`.
- **Windows baselines:** startup gate, sizes and import tables of `3d-review.exe` and
  `review_shell.dll`. **Done.** Measured on the working tree, with A and FD20 built in this session.
- **The static CRT (FD20)** for every binary, checked by import tables with no `MSVCP140` /
  `VCRUNTIME140*` entries. **Done:** `.cargo/config.toml`, recorded as D25 in `docs/ARCHITECTURE.md`.

### Step 0 baselines, Windows (4 November 2026)

Release builds from the working tree: `cargo build --release -p review-app -p review-cli` and
`--profile release-shell -p review-shell-windows`. Windows 11 Enterprise 10.0.26300, 24 cores.
Imports are read with `llvm-readobj --coff-imports`.

| Binary | DLL runtime (before FD20) | Static CRT (after) |
| --- | --- | --- |
| `3d-review.exe` | 20,253,184 bytes | 20,561,408 (+308 KB) |
| `3d-review-cli.exe` | 4,336,128 | 4,602,368 (+266 KB) |
| `review_shell.dll` | 7,953,408 | 8,196,096 (+243 KB) |

Import tables before FD20:
- `3d-review.exe`: `MSVCP140`, `VCRUNTIME140`, `VCRUNTIME140_1`, ten `api-ms-win-crt-*` sets, and
  the OS DLLs: ADVAPI32, GDI32, KERNEL32, USER32, `api-ms-win-core-synch-l1-2-0`,
  bcryptprimitives, combase, d3d11, dbghelp, dwmapi, imm32, ntdll, ole32, oleaut32, secur32,
  shell32, shlwapi, uxtheme, ws2_32.
- `3d-review-cli.exe`: `VCRUNTIME140`, `VCRUNTIME140_1`, eight `api-ms-win-crt-*` sets, KERNEL32,
  advapi32, `api-ms-win-core-synch-l1-2-0`, bcryptprimitives, ntdll.
- `review_shell.dll`: `VCRUNTIME140`, `VCRUNTIME140_1`, five `api-ms-win-crt-*` sets, advapi32,
  `api-ms-win-core-synch-l1-2-0`, bcryptprimitives, combase, d3d11, dbghelp, gdi32, kernel32,
  ntdll, oleaut32, shell32, user32.

After FD20, each imports exactly its OS DLLs above: no `MSVCP140`, `VCRUNTIME140*` or
`api-ms-win-crt-*` entry.

Startup gate (`scripts/gate.ps1`): A was the DLL-runtime build and B the static-CRT build, on
`assets/test_models/Bot.fbx`. That fixture is local and untracked (sha256 `4e851e69...d5186a`,
41,128 triangles, 212 bones, one clip). Five interleaved runs each, after a warm-up, at 1600x960,
2x MSAA, GTAO on, clip playing.

| Measure | A (DLL runtime) | B (static CRT) | Delta |
| --- | --- | --- | --- |
| startup | 209.7 ms | 200.6 ms | -9.1 ms (-4.3%) |
| frame | 0.405 ms | 0.393 ms | -0.012 ms |
| frame p95 | 0.608 ms | 0.642 ms | +0.034 ms |
| cpu/frame | 0.351 ms | 0.338 ms | -0.014 ms |
| private | 576.4 MB | 574.8 MB | -0.3% |
| gpu | 336.4 MB | 336.3 MB | 0.0% |
| audit | 44.0 ms | 43.9 ms | -0.3% |

The gate passed. The machine wasn't idle, so treat differences of a few percent as noise. The
static-CRT build also loads as a shell extension would (`LoadLibrary`, then `DllGetClassObject`
and `DllCanUnloadNow` present), and the CLI checks a model.

## References

- [`docs/cli.md`](cli.md): the CLI reference.
- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) and [`docs/GOTCHAS.md`](GOTCHAS.md): the committed project
  rules.
- In psmyles/fire-decode: `asset-check-plan.md` (AC13 is this repo) and `fire-decode-plan.md` (step 4
  is this repo).
- `docs/PLAN.md` in psmyles/asset-rules, psmyles/fire and psmyles/launcher.
