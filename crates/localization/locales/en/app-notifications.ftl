### The notice cards `app` raises.
###
### Library error text is deliberately *not* localized (invariant 12): an
### `ImportError` or `OptError` is a diagnostic, it is what a bug report quotes,
### and its crate must not know about text. So the lead line is a message here and
### the diagnostic rides in as `{ $detail }`.

## Loading

app-notifications-loading = Loading { $file }...
app-notifications-loaded = Loaded { $file } in { $seconds }
app-notifications-couldnt-load = Couldn't load { $file }: { $detail }
# The model itself is fine and already on screen; what failed is the capture the
# exporter writes back, so the notice has to say that rather than read as a load
# failure.
app-notifications-couldnt-read-properties =
    Couldn't read the file's extra properties: { $detail }
    The model itself is fine and you can keep working with it. But if you export it,
    the extra settings the viewer does not show (such as custom properties) would be
    lost.

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

## Opt

app-notifications-optimizing = Optimizing mesh...
app-notifications-exporting = Exporting FBX...
app-notifications-export-failed = Export failed: { $detail }
app-notifications-optimization-warning = Optimization warning
app-notifications-optimization-warnings = { $count } optimization warnings
app-notifications-couldnt-save = Couldn't save { $file }
app-notifications-couldnt-read = Couldn't read { $file }
app-notifications-preset-loaded = Loaded { $file }

# Per-object overrides that could not be reattached when a preset was loaded.
app-notifications-overrides-dropped = { $count ->
        [one] One per-object setting in the preset referred to an object this model doesn't have, so it was left out.
       *[other] { $count } per-object settings in the preset referred to objects this model doesn't have, so they were left out.
    }
app-notifications-overrides-by-position = { $count ->
        [one] One per-object setting came from an older preset that did not store object names, so it was matched by its place in the list - please check it landed on the object you meant.
       *[other] { $count } per-object settings came from an older preset that did not store object names, so they were matched by their place in the list - please check they landed on the objects you meant.
    }

## The graphics device

app-notifications-gpu-fault = { $context }: { $detail }
app-notifications-device-lost-startup = The GPU stopped responding while starting up ({ $reason }). The view may stay blank, so please restart the viewer.

## Mode notices

# Named after the key that switched the view, so it is one word and always fits.
app-notifications-buffer-mode = Buffer: { $buffer }

## The errors a user can act on

# Everything else keeps its library diagnostic verbatim; these tell the reader
# what to *do*, which a diagnostic written for a bug report does not.
app-notifications-opt-unavailable = Mesh optimization is not available in this build.
    It was built without the optimizer library, so the Opt workspace can measure a
    model but cannot change one.
app-notifications-remesh-unavailable = Remeshing is not available in this build.
    It was built without the retopology library, so every other operation still runs
    but Remesh is skipped.
app-notifications-preset-newer = This preset was saved by a newer version of the viewer.
    It is version { $found }, and this build understands up to version { $supported }.
    Open it with the version that saved it, or set the operations up again here.

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
