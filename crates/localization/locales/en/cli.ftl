### `review-cli`: what the console tool prints.
###
### Option names (`--out`, `--if-exists`) and the words a choice takes (`skip`,
### `file-per-lod`) are the command line's own syntax and stay as they are in every
### language. So do the program's name and the `.fbx` extension. Every line is read
### in a terminal, in Launcher's run log, or by a script matching it, so a message
### that reports a problem keeps the same wording from one release to the next.

## Help

cli-help-about = Optimizes and checks FBX models without opening 3D Review.
cli-help-usage = Usage:
cli-help-commands = Commands:
cli-help-options = Options:
cli-help-models = Give any mix of models and folders. A folder stands for the FBX files in it.
cli-help-more = Run review-cli <command> --help to see a command's options.
cli-help-usage-main = review-cli <command> <models or folders>... [options]
cli-help-usage-optimize = review-cli optimize <models or folders>... --out <folder> [options]
cli-help-usage-audit = review-cli audit <models or folders>... [options]
cli-help-usage-info = review-cli info <models or folders>... [options]
cli-help-command-optimize = Simplify models, make LODs and save them as FBX.
cli-help-command-audit = Check models against an engine's rules.
cli-help-command-info = Show what a model holds: polygons, triangles, vertices and more.

# What an option takes, shown after it: `--out <folder>`.
cli-help-value-folder = folder
cli-help-value-file = file
cli-help-value-ratios = ratios
cli-help-value-ratio = ratio
cli-help-value-text = text

cli-help-option-out = The folder the optimized models go in. Models found in a folder keep their subfolders.
cli-help-option-preset = A preset saved from 3D Review's Opt workspace: the operations to run.
cli-help-option-lods = The LODs to make, as shares of the triangles: 0.5,0.25 or 50%,25%. none makes no LODs.
cli-help-option-reduce = Simplify the model itself to this share of its triangles (0.5 or 50%) before any LODs are made.
cli-help-option-packaging = single-file puts every LOD in one FBX; file-per-lod writes name_LOD0.fbx, name_LOD1.fbx and so on.
cli-help-option-hierarchy = rebuild keeps the model's hierarchy; flat writes every mesh at the top level.
cli-help-option-format = Write binary or ASCII FBX.
cli-help-option-suffix = Add this to the end of each file name.
cli-help-option-if-exists = When a file it would write is already there: fail that model (the default), skip it, or replace the file.
cli-help-option-replace-models = Let each model's results replace the model itself, when --out and --suffix put them there.
cli-help-option-profile = The rules to check against: unity, unreal, generic (the default), or an audit profile saved from 3D Review.
cli-help-option-fail-on = The lowest severity that makes the run fail: error (the default), warning, info, or never.
cli-help-option-recurse = Also look in the folders inside each folder.
cli-help-option-report = Write what happened to this JSON file, for a script or a tool to read.
cli-help-option-progress-bar = Show progress on the terminal's tab or taskbar button. auto (the default) does where the terminal is known to show it.
cli-help-option-help = Show this help.
cli-help-option-version = Show the version.

## A command line it cannot follow (exit status 2, before anything runs)

cli-usage-hint = Run { $help } to see what it takes.
cli-usage-no-command = Say what to do: optimize, audit or info.
cli-usage-unknown-command = There is no command { $command }. The commands are optimize, audit and info.
cli-usage-unknown-option = There is no option { $option } here.
cli-usage-needs-value = { $option } needs a value.
cli-usage-takes-no-value = { $option } takes no value.
cli-usage-bad-choice = { $option } takes { $choices }, not "{ $value }".
cli-usage-bad-ratio = { $option } takes a share between 0.01 and 1, or a percentage, such as 0.5 or 50%, not "{ $value }".
cli-usage-too-many-lods = { $option } makes at most { $max } LODs.
cli-usage-no-models = Give at least one model or folder.
cli-usage-needs-out = optimize needs --out, the folder the optimized models go in.
cli-usage-nothing-to-do = Say what to do to the models: --preset, --lods or --reduce.

## What a run says

cli-no-models = Found no FBX models to work on.
cli-not-there = { $file } isn't there.
cli-folder-unreadable = Couldn't look in { $folder }: { $reason }
cli-lfs-pointer = It is a Git LFS pointer, not the model itself. Run git lfs pull to get the model.
cli-properties-dropped = The model's source properties couldn't be read, so not everything about it carries over: { $reason }
# Part of a file the importer could not read and loaded the model without - a
# malformed animation key, blend shape or skin binding - in the importer's words.
cli-part-left-out = Part of the file couldn't be read and was left out: { $detail }
cli-warning = Warning: { $text }
cli-note = Note: { $text }
cli-report-failed = Couldn't write the report { $file }: { $reason }

## optimize

cli-optimize-start = { $count ->
        [one] Optimizing one model into { $out }.
       *[other] Optimizing { $count } models into { $out }.
    }
cli-optimize-preset-unreadable = Couldn't read the preset { $file }: { $reason }
cli-optimize-nothing-enabled = The preset has nothing switched on, and no --lods or --reduce was given.
cli-optimize-level = LOD{ $level }: { $triangles } triangles, { $vertices } vertices
cli-optimize-wrote = Wrote { $file }
cli-optimize-skipped = Skipped: { $file } is already there.
cli-optimize-exists = { $file } is already there. Use --if-exists skip or --if-exists replace to go on.
cli-optimize-is-a-model = { $file } is one of the models being optimized. Choose another --out folder, or add a --suffix.
cli-optimize-is-this-model = { $file } is one of the models being optimized: this one. Choose another --out folder, add a --suffix, or replace it with --replace-models.
cli-optimize-written-twice = { $file } is also where { $other } goes. Give the two models different names, or optimize them into different --out folders.
cli-optimize-failed = Couldn't optimize { $file }: { $problem }
cli-optimize-done = Done: { $done } optimized, { $skipped } skipped, { $failed } didn't work.

## audit

cli-audit-start = { $count ->
        [one] Checking one model against the { $profile } profile.
       *[other] Checking { $count } models against the { $profile } profile.
    }
cli-audit-profile-unreadable = Couldn't read the audit profile { $file }: { $reason }
cli-audit-failed = Couldn't check { $file }: { $problem }
cli-audit-done = Done: { $done } checked, { $failed } didn't work.
cli-audit-flagged = { $count ->
        [one] One model has findings at { $severity } or worse.
       *[other] { $count } models have findings at { $severity } or worse.
    }

## info

cli-info-counts = Polygons: { $polygons }, triangles: { $triangles }, vertices: { $vertices }, GPU vertices: { $gpu_vertices }
cli-info-parts = Draws: { $draws }, materials: { $materials }, UV sets: { $uv_sets }, bones: { $bones }, animations: { $clips }
cli-info-unit = Unit: { $unit }
cli-info-failed = Couldn't read { $file }: { $problem }
cli-info-done = Done: { $done } read, { $failed } didn't work.
