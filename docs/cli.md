# review3d-cli

`review3d-cli` runs the Opt and Aud workspaces' work without opening the viewer:
optimizing models into LODs and FBX files, checking them against an engine's
rules, and reading their stats. It is a console program shipped beside the viewer,
built for scripts, build pipelines and Launcher tools. Everything it does is a
library call the viewer also makes, so a run measures and writes exactly what the
viewer would.

Up to 0.8.0 it was called `review-cli`, and 0.9.0's pre-release builds called it
`3d-review-cli` - a name PowerShell cannot run as typed, since it reads `3d` as a
number. Scripts that call it by an old name need the new one, and so does an Asset
Check `rules.json`, which names it as the `model` type's `tool` and in `requires`.
Setup removes an old `review-cli.exe` or `3d-review-cli.exe` from the install
folder, and on macOS **Install Command Line Tool...** removes an old
`/usr/local/bin/review-cli` or `/usr/local/bin/3d-review-cli` link into the app.

```text
review3d-cli optimize <models or folders>... --out <folder> [options]
review3d-cli audit    <models or folders>... | --staged [options]
review3d-cli info     <models or folders>... [options]
review3d-cli rules check <rules.json>
review3d-cli rules explain <files>... --rules <rules.json>
review3d-cli rules catalog
review3d-cli --version | --help | <command> --help
```

It is also the model half of **Asset Check**, which checks the art committed to a
Unity project against a technical artist's rules: `audit --rules`, `--results`,
`--staged` and the `rules` commands (see [Asset Check](#asset-check) below).

## Where it is installed

| | |
|---|---|
| Windows | `review3d-cli.exe` beside `3d-review.exe`, in the install folder the Uninstall key `{B6F4B0E9-2E1B-4C3A-9D2F-3D5E7A1C0F42}_is1` names as `InstallLocation` (`%LOCALAPPDATA%\Programs\3D Review` for a per-user install, `C:\Program Files\3D Review` for all users). Setup's **Add 3D Review to my PATH** task, off by default, puts that folder on the user's PATH. |
| macOS | `3D Review.app/Contents/MacOS/review3d-cli`. Find the app by its bundle id, `com.psmyles.3d-review` (`mdfind "kMDItemCFBundleIdentifier == 'com.psmyles.3d-review'"`). A `scripts/dev-app.sh` bundle has it too. The .dmg cannot install anything itself, so the app does it: **3D Review > Install Command Line Tool...** symlinks `/usr/local/bin/review3d-cli` to that binary, asking for an administrator password when the folder needs one, and a notice says so at launch while no `review3d-cli` is on the PATH. The link follows the app's updates, and breaks if the app is moved (the launch notice then comes back). |

## The contract

These rules hold for every command, and they are what a tool calling it can rely on.

- **Long options only, no prompts, no standard input.** Nothing ever waits for an
  answer.
- **Exit status 2**: the command line could not be followed (an unknown option, an
  option with no value, a value it does not take, no models given, `--rules`
  with `--profile`). Nothing ran and no report was written, and a report or
  results an earlier run left at the line's `--report` or `--results` path are
  deleted.
- **Exit status 0**: everything worked. Skipping a model under `--if-exists skip`
  counts as working.
- **Exit status 1**: anything else. A model that could not be read or written, a
  preset or profile that could not be read, no models found, an audit finding at
  `--fail-on` or worse, or a report or results that could not be written.
- **A batch never stops early.** Every model is tried, each failure is printed and
  recorded, and the exit status gives the verdict at the end.
- **`--report <file.json>`** writes an account of the run (below). It is written
  whenever the command line was accepted, even when the run stopped before the
  first model, and a report left at that path by an earlier run is deleted first,
  on a refused command line too. So after any run, the file is this run's or is
  not there.
- **Plain lines.** Results go to standard output, problems to standard error,
  each a whole line in the reader's language. A line that reports a problem keeps
  its wording from one release to the next, so a tool can match on it. Writes to a
  closed pipe are ignored, never fatal. Each model's first line (`[3/12]
  chair.fbx`) is yellow in a terminal known to show colour: on macOS and Linux,
  and on Windows in Windows Terminal, ConEmu or one that sets `TERM` (Launcher's
  console). Through a pipe or into a file, and with `NO_COLOR` set, every line is
  plain.
- **Progress** is the terminal's own bar, `OSC 9;4`, sent where it is known to be
  shown: Windows Terminal, ConEmu, WezTerm and Ghostty, or anywhere with
  `--progress-bar on`. Elsewhere it is not sent, because some terminals print the
  sequence or turn it into a notification.

Two rules make it easy to fill in a command line from a form:

- **An empty value is the same as leaving the option out.** `--preset ""` means
  no preset, so a setting nobody filled in can still be passed.
- **A list is one value, separated by commas.** `--lods 0.5,0.25`.

Option values may also be written `--option=value`. `--` ends the options, and
everything after it is a model. Relative paths resolve against the working folder.

## Models and folders

Any mix of files and folders. A file is taken as given. A folder stands for the
`.fbx` files in it, in name order, and with `--recurse` for those in the folders
inside it too, skipping folders whose names start with `.` and never following a
symlinked folder. A model reached twice is worked on once. A model that is a Git
LFS pointer rather than the model itself is reported as one, with the command that
fetches it.

## Options every model command takes

`optimize`, `audit` and `info`; the `rules` commands take none of these.

| Option | |
|---|---|
| `--files-from <list>` | Also work on the models and folders listed in this file, one per line. |
| `--recurse` | Also look in the folders inside each folder. |
| `--report <file>` | Write the run's report to this JSON file. Its folder is created if needed. |
| `--progress-bar auto\|on\|off` | `auto` (the default) shows the terminal's progress bar where it is known to be shown. |

`--files-from` exists because Windows limits a command line to about 32,000
characters, which the models of a big commit or import can pass. Each line is a
model or a folder, relative to the working folder as an argument is; blank lines
are skipped, and the file is read as UTF-8, or as UTF-16 when it starts with that
byte-order mark (what Windows PowerShell 5's `Out-File` writes). A file in the
list that is not `.fbx` is left out, as one in a folder is, since a list is often
everything an import or a commit touched. It may stand in for models on the line,
or add to them. A list that names no models is a run with nothing to do: exit
status 0, where models and folders on the line that hold none are a problem.

## optimize

```text
review3d-cli optimize props --recurse --out build/lods --lods 50%,25%,12.5%
review3d-cli optimize hero.fbx --preset presets/hero.json --out build --if-exists replace
```

Runs an operation stack over each model and writes the result as FBX, as the Opt
workspace's Export does.

| Option | |
|---|---|
| `--out <folder>` | Required. Where the models are written. A model found in a folder keeps its place below it: `props/crates/a.fbx` under `--out build` is written to `build/crates/a.fbx`. |
| `--preset <file>` | A preset saved from the Opt workspace (**Save preset**). Its per-object settings are matched to each model's objects by name, as **Load preset** does. One that matches nothing is dropped, with a warning. |
| `--lods <ratios>\|none` | The LOD chain, as shares of the triangles: `0.5,0.25` or `50%,25%`, at most 8. Replaces the preset's chain. A level the preset already has keeps its error limit, and a new one gets the one before it (0.01 by default, at every level). `none` turns the preset's chain off. |
| `--reduce <ratio>` | Simplify the model itself to this share of its triangles before any LODs are made. Sets the preset's Reduce, or adds one before the chain. |
| `--packaging single-file\|file-per-lod` | `single-file` writes one FBX holding `Name_LOD0`...`Name_LODn`. `file-per-lod` writes `name_LOD0.fbx`, `name_LOD1.fbx`... A run with no chain writes `name.fbx` either way. |
| `--hierarchy rebuild\|flat` | `rebuild` keeps the model's hierarchy; `flat` writes every mesh at the top level, in world space. |
| `--format binary\|ascii` | The FBX encoding. |
| `--suffix <text>` | Added to each file name, before `.fbx` and any `_LOD<n>`. |
| `--if-exists fail\|skip\|replace` | When a file a model would write is already there. `fail` (the default) fails that model, `skip` leaves the model out, `replace` writes over the file. |
| `--replace-models` | Let each model's results replace the model itself, when `--out` and `--suffix` put them there: so an engine that has the model placed in its scenes picks up the new one. There is no undo; keep the models in source control. |

Without `--preset`, the stack is just what `--lods` and `--reduce` ask for, so at
least one of the three is needed. `--packaging`, `--hierarchy` and `--format`
override the preset's export settings. Added operations go in front of any reorder
operation (vertex cache, overdraw, vertex fetch), which only pays off when nothing
changes the triangles after it.

Two checks run before a model is loaded, whatever `--if-exists` says. A model
never writes over a file another model of the same run is writing, and never over
a model the run is reading, so `--out` can be the source folder when a `--suffix`
keeps the names apart. `--replace-models` relaxes the second for a model's own
file only: the model is loaded in full before anything is written, and the export
puts each file in place only once all of it is on disk, so a failed model leaves
its source as it was. A file that is *another* model of the run is still refused,
since that model may not have been read yet. A model replaced this way is replaced
whatever `--if-exists` says; its other files (`file-per-lod`'s levels) still go by
it. Both compare files rather than spellings: links and `..`
are resolved, a folder `--out` creates during the run makes no difference, and on
Windows and macOS, whose file systems ignore case, names differing only in case
are one file. The output is always `.fbx` in lower case, so `Rock.FBX` written
into its own folder without a suffix is refused, as are `a/Body.fbx` and
`b/body.fbx` written to one folder.

Nothing is hidden as it can be in the viewer's Outliner. To keep an object out of
an AO bake, exclude it in the preset.

## audit

```text
review3d-cli audit Assets/Models --recurse --profile unity --report audit.json
review3d-cli audit hero.fbx --profile team-profile.json --fail-on warning
```

Checks each model against an engine profile, as the Aud workspace does.

| Option | |
|---|---|
| `--profile unity\|unreal\|generic\|<file>` | A built-in profile, or one saved from the Aud workspace (**Save profile**). `generic` is the default. The viewer's last-used profile is never read: a run does what its command line says, on any machine. A file literally named `unity` is passed as `./unity`. |
| `--rules <rules.json>` | Asset Check's rules instead: each model's profile and stage come from its name, its folder and the branch. Not with `--profile` (exit status 2). |
| `--fail-on error\|warning\|info\|never` | The least severe finding that fails the run (exit status 1). `error` is the default, and `warning` with `--rules`. A model with such a finding is still checked, and its entry is `done` and `flagged`. |
| `--results <file>` | Write Asset Check's results to this JSON file. |
| `--staged` | Check the `.fbx` models staged for the next commit in the working folder's repository, as staged, instead of models named on the line. |

Each model's lines are the Aud workspace's **Copy summary**: the counts, then
one line per failing check, worst first. With `--rules`, the line before them
names the profile and stage the rules gave the model.

Several models are checked at once, up to four on a machine with the cores for
it: loading a model is one thread's work, so a batch finishes in a fraction of
the time. Each model's lines are said in the models' order once it is done, so
the output, the report and the results are the same however the work fell.

### A profile file

**Save profile** writes every setting; a file written by hand needs only what it
changes, so a technical artist can keep one short:

```json
{
  "version": 1,
  "generator": "hand-written",
  "profile": {
    "name": "Prop",
    "base": "unity",
    "rules": {
      "geometry.triangle_budget": { "severity": "error", "params": { "max_per_object": 5000 },
                                    "note": "Quest budget for props." },
      "geometry.ngons": { "enabled": false }
    }
  }
}
```

- `base` is the built-in the profile starts from: `unity`, `unreal` or
  `generic`, the default.
- A rule listed is switched on unless it says `"enabled": false`. A severity or a
  parameter it leaves out is the base's, as is every rule it does not list.
- `note` is the technical artist's word on the rule. Asset Check's results show
  it under the tool's own explanation. The viewer's profile editor does not show
  or edit notes, but keeps them: a profile loaded and saved again from the viewer
  still has every one.
- Rules this version does not know are skipped, a rule whose settings cannot be
  read takes its default, and numbers are brought back into the range the editor
  allows.

## Asset Check

```text
review3d-cli audit --staged --rules rules/rules.json --results .git/asset-check/results-review3d-cli.json
review3d-cli audit --files-from changed.txt --rules rules/rules.json --results results.json
review3d-cli rules check rules/rules.json
review3d-cli rules explain Assets/Art/Props/SM_Crate.fbx --rules rules/rules.json
```

Asset Check checks the art committed to a Unity project against a technical
artist's rules, from a git `pre-commit` hook, a Fork command, the Unity editor or
CI, and shows the artist what is wrong, why, and how to fix it. `fire-cli` checks
the images and `review3d-cli` the models. Both decide which rules apply to a file
through one shared crate, [`asset-rules`](https://github.com/psmyles/asset-rules),
so they always give a file the same answer, and both write one results format.
The design is `asset-check-plan.md` in the rules repository; this tool's part
is its AC13.

### --rules

`rules.json` lives in the rules repository, a submodule only the technical
artist writes. For each model it gives:

1. **Its type**, from its extension. A file no type claims, or one another tool
   checks, is left out: it is a `skipped` entry in the report and not in the
   results.
2. **Its profile**: the first of that type, in file order, whose `names` match the
   file name without its extension (and whose `paths`, if it has any, match its
   path in the repository). One that matches none gets a single finding,
   `file.no_profile`: a warning, or an error in enforce mode, whose "how" lists the
   names that would match.
3. **Its stage**: the branch's (`branches[]`), else the profile's own, else the
   project's.
4. **Its rules file**: the profile's file for that stage, else its `"*"` file. It
   is a profile file, loaded as `--profile` loads one ([A profile
   file](#a-profile-file)). A profile with no file for the stage, or a file that
   cannot be read, makes that model unreadable.

The current folder's repository is where the branch comes from and what `paths`
are matched against; outside a repository there is no branch, and only profiles
without `paths` can match. When `rules.json`'s `requires` asks for a newer
`review3d-cli`, the run has one more finding, `tool.too_old`, saying what to
update. A `rules.json` too new to read at all is a problem that stops the run,
and the results still carry that finding when it can be made.

### --staged

The models staged for the next commit (added, changed or renamed in the index;
never deleted ones), read as they are staged. A model whose working file has
changed since it was staged is read from a temporary copy of what is staged, or of
its Git LFS object; one that has not is read in place. Every other staged file is
left out. A commit with no models in it is a run with nothing to do: exit status
0, with no files in the results.

With `--rules`, each model's results are cached in `.git/asset-check/cache/`
(the repository's common git folder, shared by its worktrees), keyed on the staged
content, the rules file that applied, this tool's version and the stage. A model
already checked is not checked again: the Fork command's check fills the cache,
and the hook's check of the same commit is then all but instant. A cached model's
report entry says `"cached": true` and has no `audit` document; its lines say it
is unchanged. Entries unused for 30 days are removed.

git is run as the `git` on the PATH, inside a hook as everywhere else, so the
index a hook reads is the one git itself is committing.

### --results

```json
{
  "schema": "asset-check.results",
  "version": 1,
  "tool": { "name": "review3d-cli", "version": "0.9.0" },
  "rules": { "commit": "1a2b3c4", "digest": "sha256:..." },
  "context": { "branch": "main", "mode": "advise" },
  "findings": [],
  "files": [
    {
      "path": "Assets/Art/Props/SM_Crate.fbx",
      "type": "model", "profile": "prop", "profile_label": "Prop",
      "stage": "production", "stage_from": "branch",
      "status": "checked", "problem": null,
      "findings": [
        {
          "rule": "geometry.triangle_budget", "severity": "error", "title": "Triangle budget",
          "what": "Parts, or the whole file, with more triangles than the profile allows. Found in one object. Measured 8200; the profile allows at most 5000.",
          "why": "Triangles cost memory and draw time on every frame the asset is seen.",
          "how": "Reduce the mesh, or split detail into LODs. The Opt workspace can do both.",
          "note": "Quest budget for props.",
          "measured": { "value": 8200, "unit": "count", "total": 1 },
          "limit": { "max": 5000, "unit": "count" },
          "objects": ["SM_Crate"],
          "fix": null
        }
      ]
    }
  ]
}
```

The format is `asset-rules`', shared with `fire-cli`; it is written by replacing
the file, so a reader never sees half of one, and the file an earlier run left at
the path is removed first, on a refused command line too, as `--report` is.

- **`files`**: one entry per model, in the order checked. `path` is the model's
  path in the repository, with `/` separators (outside one, the path as reached).
  `status` is `checked`, or `unreadable` with `problem` saying why: an FBX that
  would not load, a Git LFS pointer, staged content that could not be read, a
  profile with no rules file for the stage.
- **One finding per failing check**, worst first. `title` is the check's name;
  `what` says what was found, how much of the model it is in, and the figure
  measured against the limit; `why` and `how` are the Inspector's own text for the
  check, `why` worded for the profile's engine; `note` is the technical artist's.
  `measured` has the figure as `value` and `unit` when the check states one, and
  `total`, how many triangles, edges, vertices or objects were found; `limit` has
  `max`, `min`, both, `equals` or `patterns`. `objects` names the offending
  objects. `fix` is reserved for automatic fixes and is always `null`.
- **`findings`** at the top are the run's own, such as `tool.too_old`.

Without `--rules`, results are written all the same, for a tool showing a
`--profile` run: `type` is `model`, `profile` is the `--profile` value as given,
`profile_label` its name, `stage` and `stage_from` are `null`, the mode is
`advise`, and `rules.digest` is the profile file's (a built-in's saved form for a
built-in).

### rules check, rules explain and rules catalog

For the technical artist. `rules check` lists the problems in a `rules.json` -
an unknown type, stage or tool, a stage with no rules file, a rules file that is
not there, an invalid or never-matching pattern, a duplicate id, an unknown field
- and in each model rules file it names, read against review3d-cli's rules
catalog: a rule review3d-cli doesn't have, a parameter of the wrong kind or out
of range, a field it doesn't know (`fire-cli` checks the texture rules files).
One per line, and it exits 1 if there is any, warnings included, so a CI step can
stop on it. `rules catalog` prints every rule review3d-cli checks - its
category, parameters, defaults per engine and texts - as JSON, with the tool's
version, for Rule Editor. `rules explain` says, for each file given, which type, profile, stage
and rules file apply to it and why, or which patterns it was tried against and
did not match; the files need not exist. Their text is `asset-rules`', in English.

### Building against unreleased asset-rules

The workspace depends on `asset-rules` by tag. To build against a checkout beside
this one, patch it from a `.cargo/config.toml` in the folder that holds both - not
this repository's own, which is committed (it carries the static CRT, D25) - so
the same file serves `fire-cli`'s checkout too:

```toml
# <folder holding 3d-review-rs and asset-rules>/.cargo/config.toml
[patch."https://github.com/psmyles/asset-rules"]
asset-rules = { path = "asset-rules" }
```

Cargo merges it with the repository's own, and a path in it is relative to the
folder that holds that `.cargo`. Delete it again before committing a
`Cargo.lock`, which would otherwise record the local checkout.

## info

```text
review3d-cli info hero.fbx --report stats.json
```

What each model holds, as the Model Stats panel shows it: polygons, triangles and
vertices as the modelling program counts them, GPU vertices as an engine would
build them, draws, materials, UV sets, bones, animations and the file's unit.

## The report

```json
{
  "schema": "3d-review.cli-report",
  "version": 1,
  "generator": "review3d-cli 0.7.0",
  "created_unix": 1791907669,
  "command": "optimize",
  "ok": false,
  "problem": null,
  "models": 4,
  "done": 3,
  "skipped": 0,
  "failed": 1,
  "written": 3,
  "outputs": ["build/a.fbx", "build/b.fbx", "build/props/c.fbx"],
  "results": [
    { "file": "props/a.fbx", "status": "done", "notices": [], "...": "..." },
    { "file": "props/d.fbx", "status": "failed", "problem": "...", "notices": [] }
  ]
}
```

Keys are ids and never prose, and their order means nothing. A version only ever
adds fields; a change to what one means is a new version.

### Every command

| Field | |
|---|---|
| `ok` | The exit status as a field: `true` exactly when it is 0. |
| `problem` | Why the run stopped before working on any model (an unreadable preset or profile, no models found), or `null`. |
| `models` | How many models the run set out to work on, counting arguments that turned out not to be there. Always `done + skipped + failed`. |
| `done`, `skipped`, `failed` | How many models came to each. |
| `results` | One entry per model, in the order they were worked on: `file` (the path as reached), `status` (`done`, `skipped` or `failed`), `problem` (why, when it was not `done`), `notices` (warnings and notes that did not stop it, as printed). |

### optimize

Top level: `written` (files written) and `outputs` (every file written, in order,
so a next step can pick them up).

Each entry, when `done`: `outputs`; `source` (`triangles`, `vertices`, and the
`acmr` / `atvr` / `overdraw` / `overfetch` figures of the model as it came, after
the lossless indexing every engine does); `levels` (per LOD, level 0 first:
`level`, `polygons`, `triangles`, `vertices`, the same four figures, and
`simplify_error`, or `null` for a level that was not simplified). When the preset
had per-object settings, `overrides` gives `rebound`, `by_position` and `dropped`.
These are the figures the Opt workspace's stats cards show, against the same
baseline.

### audit

Top level: `profile` (`unity`, `unreal`, `generic` or the file as given; `null`
with `--rules`), `rules` (the `rules.json` as given, with `--rules` only),
`fail_on` (`error`, `warning`, `info`, or `null` for `never`), `flagged` (models
with a finding at `fail_on` or worse), and `errors`, `warnings`, `infos` (failing
checks of each severity, over every model).

With `--rules`, every entry also has `profile`, `profile_label`, `stage` and
`stage_from`, as the results have them, and a model no profile matches is `done`,
with its `file.no_profile` finding counted in `warnings` (or `errors`). A model of
another tool's type is `skipped`.

Each entry, when `done`: `flagged`, `worst` (the worst severity found, or `null`),
`errors`, `warnings`, `infos` (this model's failing checks of each severity),
`cached` (`true` when a `--staged` run took it from the cache, which leaves out
the next three),
`passed`, `not_evaluated`, and `audit`, the full `3d-review.audit-report` document
the Aud workspace's **Save report** writes, with every finding's objects, faces and
points numbered as the modelling program numbers them. Its `source.unit_meters` and
`source.up_axis` are written as `info`'s `unit_meters` and `up_axis` are.

### info

Top level: `polygons`, `triangles`, `vertices`, `gpu_vertices`, summed over every
model, so a single-model run's are that model's.

Each entry, when `done`: `counts` (`polygons`, `triangles`, `vertices`,
`gpu_vertices`, `draws`, `materials`, `uv_sets`, `bones`, `clips`, `nodes`),
`unit_meters` (the file's unit in meters, `0` when it declares none, written as the
shortest number that is it: `0.01` for centimeters) and `up_axis` (`positive_x`,
`negative_x`, `positive_y`, `negative_y`, `positive_z` or `negative_z`, the words
an audit profile's up-axis target uses, or `null` when the file names none).
