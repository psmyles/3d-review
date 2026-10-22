# review-cli

`review-cli` runs the Opt and Aud workspaces' work without opening the viewer:
optimizing models into LODs and FBX files, checking them against an engine's
rules, and reading their stats. It is a console program shipped beside the viewer,
built for scripts, build pipelines and Launcher tools. Everything it does is a
library call the viewer also makes, so a run measures and writes exactly what the
viewer would.

```text
review-cli optimize <models or folders>... --out <folder> [options]
review-cli audit    <models or folders>... [options]
review-cli info     <models or folders>... [options]
review-cli --version | --help | <command> --help
```

## Where it is installed

| | |
|---|---|
| Windows | `review-cli.exe` beside `3d-review.exe`, in the install folder the Uninstall key `{B6F4B0E9-2E1B-4C3A-9D2F-3D5E7A1C0F42}_is1` names as `InstallLocation` (`%LOCALAPPDATA%\Programs\3D Review` for a per-user install, `C:\Program Files\3D Review` for all users). Setup's **Add 3D Review to my PATH** task, off by default, puts that folder on the user's PATH. |
| macOS | `3D Review.app/Contents/MacOS/review-cli`. Find the app by its bundle id, `com.psmyles.3d-review` (`mdfind "kMDItemCFBundleIdentifier == 'com.psmyles.3d-review'"`). A `scripts/dev-app.sh` bundle has it too. |

## The contract

These rules hold for every command, and they are what a tool calling it can rely on.

- **Long options only, no prompts, no standard input.** Nothing ever waits for an
  answer.
- **Exit status 2**: the command line could not be followed (an unknown option, an
  option with no value, a value it does not take, no models given). Nothing ran
  and no report was written, and a report an earlier run left at the line's
  `--report` path is deleted.
- **Exit status 0**: everything worked. Skipping a model under `--if-exists skip`
  counts as working.
- **Exit status 1**: anything else. A model that could not be read or written, a
  preset or profile that could not be read, no models found, an audit finding at
  `--fail-on` or worse, or a report that could not be written.
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

## Options every command takes

| Option | |
|---|---|
| `--recurse` | Also look in the folders inside each folder. |
| `--report <file>` | Write the run's report to this JSON file. Its folder is created if needed. |
| `--progress-bar auto\|on\|off` | `auto` (the default) shows the terminal's progress bar where it is known to be shown. |

## optimize

```text
review-cli optimize props --recurse --out build/lods --lods 50%,25%,12.5%
review-cli optimize hero.fbx --preset presets/hero.json --out build --if-exists replace
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

Without `--preset`, the stack is just what `--lods` and `--reduce` ask for, so at
least one of the three is needed. `--packaging`, `--hierarchy` and `--format`
override the preset's export settings. Added operations go in front of any reorder
operation (vertex cache, overdraw, vertex fetch), which only pays off when nothing
changes the triangles after it.

Two checks run before a model is loaded, whatever `--if-exists` says. A model
never writes over a file another model of the same run is writing, and never over
a model the run is reading, so `--out` can be the source folder when a `--suffix`
keeps the names apart. Both compare files rather than spellings: links and `..`
are resolved, a folder `--out` creates during the run makes no difference, and on
Windows and macOS, whose file systems ignore case, names differing only in case
are one file. The output is always `.fbx` in lower case, so `Rock.FBX` written
into its own folder without a suffix is refused, as are `a/Body.fbx` and
`b/body.fbx` written to one folder.

Nothing is hidden as it can be in the viewer's Outliner. To keep an object out of
an AO bake, exclude it in the preset.

## audit

```text
review-cli audit Assets/Models --recurse --profile unity --report audit.json
review-cli audit hero.fbx --profile team-profile.json --fail-on warning
```

Checks each model against an engine profile, as the Aud workspace does.

| Option | |
|---|---|
| `--profile unity\|unreal\|generic\|<file>` | A built-in profile, or one saved from the Aud workspace (**Save profile**). `generic` is the default. The viewer's last-used profile is never read: a run does what its command line says, on any machine. A file literally named `unity` is passed as `./unity`. |
| `--fail-on error\|warning\|info\|never` | The least severe finding that fails the run (exit status 1). `error` is the default. A model with such a finding is still checked, and its entry is `done` and `flagged`. |

Each model's lines are the Aud workspace's **Copy summary**: the counts, then
one line per failing check, worst first.

## info

```text
review-cli info hero.fbx --report stats.json
```

What each model holds, as the Model Stats panel shows it: polygons, triangles and
vertices as the modelling program counts them, GPU vertices as an engine would
build them, draws, materials, UV sets, bones, animations and the file's unit.

## The report

```json
{
  "schema": "3d-review.cli-report",
  "version": 1,
  "generator": "review-cli 0.7.0",
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

Top level: `profile` (`unity`, `unreal`, `generic` or the file as given),
`fail_on` (`error`, `warning`, `info`, or `null` for `never`), `flagged` (models
with a finding at `fail_on` or worse), and `errors`, `warnings`, `infos` (failing
checks of each severity, over every model).

Each entry, when `done`: `flagged`, `worst` (the worst severity found, or `null`),
`errors`, `warnings`, `infos` (this model's failing checks of each severity),
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
