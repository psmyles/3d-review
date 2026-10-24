### The notice cards `app` raises.
###
### Library error text is deliberately *not* localized (invariant 12): an
### `ImportError` or `OptError` is a diagnostic, it is what a bug report quotes,
### and its crate must not know about text. So the lead line is a message here and
### the diagnostic rides in as `{ $detail }`.

## Loading

app-notifications-loading = Loading { $file }...
app-notifications-loading-files = Loading { $count } files...
app-notifications-loaded = Loaded { $file } in { $seconds }s
app-notifications-couldnt-load = Couldn't load { $file }: { $detail }

# Animation files attached to the model on screen, so their clips play on it.
app-notifications-attached = { $clips ->
        [one] Added one animation from { $file }
       *[other] Added { $clips } animations from { $file }
    }
app-notifications-attached-files = Added { $clips } animations from { $count } files
# An attached animation file that does not line up with the model everywhere, as
# the title of a card listing what differs.
app-notifications-attached-mismatch = { $file } does not fully match { $model }

# Files opened together that were left out, as the title of a card listing them.
app-notifications-not-loaded = { $count ->
        [one] One file was not loaded
       *[other] { $count } files were not loaded
    }
app-notifications-not-loaded-second-model =
    { $file } is a second model. Only one model is shown at a time, so open it on its own
    to look at it.
app-notifications-not-loaded-no-match = Nothing in { $file } matches { $model }.
# The model itself is fine and already on screen; what failed is the capture the
# exporter writes back, so the notice has to say that rather than read as a load
# failure.
app-notifications-couldnt-read-properties =
    Couldn't read the file's extra properties: { $detail }
    The model itself is fine and you can keep working with it. But if you export it,
    the extra settings the viewer does not show (such as custom properties) would be
    lost.
# A file that loaded with parts the importer could not read left out - a malformed
# animation key, blend shape or skin binding is dropped on its own rather than
# failing the file. Each line under it is the importer's own description of one.
app-notifications-loaded-with-repairs =
    { $file } loaded, but some of it could not be read and was left out.
app-notifications-import-repair-line = { $detail }

# Import stages, shown on the loading card as the worker moves through them.
# `import` keeps its own English `label()` for the profiling channel; these are
# what the card says.
app-notifications-stage-reading = Reading
app-notifications-stage-building = Building mesh
app-notifications-stage-extras = Reading source properties
app-notifications-stage-measuring = Measuring animation
app-notifications-stage-finishing = Measuring stats

# The stage line under the loading card's title: what the import is doing, and a
# percentage for the one stage that has a real denominator.
app-notifications-stage-line = { $stage }...
app-notifications-stage-line-percent = { $stage }... { $percent }%

## Textures

app-notifications-decoding = Decoding { $file }...
app-notifications-reloading = Reloading { $file }...
app-notifications-texture-loaded = Loaded { $file }
app-notifications-texture-reloaded = Reloaded { $file }
app-notifications-texture-failed = Couldn't load { $file }
# The textures the model's own materials name, found where the file says they are
# (or beside it) or embedded in it, and bound as the model loads.
app-notifications-loading-textures = Loading the model's textures...
# A card listing the textures the model names that could not be loaded, one per
# line under it.
app-notifications-textures-not-loaded = { $count ->
        [one] One texture { $model } uses could not be loaded
       *[other] { $count } textures { $model } uses could not be loaded
    }
app-notifications-texture-not-found = { $file } was not found where the model says it is, or beside it
app-notifications-texture-unreadable = { $file } could not be read: { $detail }

## Opt

app-notifications-optimizing = Optimizing mesh...

# The stage line under the optimizing card's title: which step of the run is
# going, and - for a step that works through a list - how far through it is. The
# operations that rebuild one object at a time name the object, because on a
# dense mesh a single one can take a while and its name is what says the run is
# still moving.
app-notifications-opt-stage = { $stage }...
app-notifications-opt-stage-count = { $stage }... { $done } of { $total }
app-notifications-opt-stage-object = { $stage }: { $object } ({ $done } of { $total })
app-notifications-opt-stage-preparing = Preparing the mesh
app-notifications-opt-stage-measuring = Measuring the source
app-notifications-opt-stage-assembling = Rebuilding the mesh
app-notifications-exporting = Exporting FBX...
app-notifications-export-failed = Export failed: { $detail }
app-notifications-optimization-warning = Optimization warning
app-notifications-optimization-warnings = { $count } optimization warnings
app-notifications-couldnt-save = Couldn't save { $file }
app-notifications-couldnt-read = Couldn't read { $file }
app-notifications-preset-loaded = Loaded { $file }

## Updates

app-notifications-checking-for-updates = Checking for updates...
app-notifications-update-available = Version { $version } is available. Opening the releases page...
app-notifications-update-latest = You have the latest version ({ $version }).
app-notifications-update-check-failed = Couldn't check for updates: { $detail }

## The command-line tool (macOS)

# At launch, while no review-cli is on the PATH. The menu path is the
# application menu's own item (menu-install-command-line-tool).
app-notifications-cli-not-on-path =
    The { $command } command is not on your PATH. Choose { $product } > Install
    Command Line Tool... to add it.
app-notifications-cli-installed =
    Installed the { $command } command at { $link }. Open a new terminal window to use it.
app-notifications-cli-install-not-bundled =
    There is no command-line tool to install: this is not a packaged build of { $product }.
app-notifications-cli-install-not-in-applications =
    Move { $product } to your Applications folder and open it from there first: a link
    to this copy would stop working.
app-notifications-cli-install-occupied =
    { $link } already exists and is not a link to { $product }. Remove it and try again.
app-notifications-cli-install-failed = Couldn't install the { $command } command: { $detail }

## Tracy

app-notifications-tracy-on-next-launch = Tracy profiling will be on from the next time you start the viewer.
app-notifications-tracy-off-next-launch = Tracy profiling will be off from the next time you start the viewer.

## The graphics device

app-notifications-gpu-scene-failed = Scene render failed: { $detail }
app-notifications-gpu-ui-failed = UI render failed: { $detail }
# `$detail` is the device's own reason code.
app-notifications-gpu-device-lost = Graphics device lost ({ $detail }) - restart the viewer
app-notifications-device-lost-startup = The GPU stopped responding while starting up ({ $reason }). The view may stay blank, so please restart the viewer.

## Mode notices

# Named after the key that switched the view, so it is one word and always fits.
app-notifications-buffer-mode = Buffer: { $buffer }

## The export's own report

app-notifications-exported-one = Exported { $file } ({ $triangles } triangles)
app-notifications-exported-many = Exported { $files } files ({ $triangles } triangles)
# When the export wrote nothing named - a chain with no file to point at.
app-notifications-exported-fallback = the mesh

app-notifications-export-incomplete = Export incomplete
app-notifications-export-incomplete-file = { $file } could not be replaced: { $reason }
app-notifications-export-nothing-changed = Nothing on disk was changed.
app-notifications-export-replaced = These files were replaced by this export:

## The rest of the notices

app-notifications-dialog-failed = Couldn't open the file dialog
# Asked for a second file dialog while one is still open, which may be behind
# another window.
app-notifications-dialog-already-open = A file dialog is already open. Finish or cancel it first.
app-notifications-opt-start-failed = Couldn't start mesh optimization
app-notifications-optimization-failed = Optimization failed: { $detail }
app-notifications-nothing-to-export = Nothing to export yet - add an operation to the list first.
app-notifications-export-start-failed = Couldn't start the export
app-notifications-preset-build-failed = Couldn't build the preset: { $detail }
app-notifications-preset-saved = Saved { $file }
app-notifications-preset-load-failed = Couldn't load that preset: { $detail }
app-notifications-watcher-unavailable = Textures will not reload on their own (the file watcher could not start)
    .description = Normally a texture refreshes by itself when you save it in your paint
        program. That will not happen this time; reopen the model to pick up a change.
app-notifications-watch-failed = { $file } will not reload on its own when it changes
