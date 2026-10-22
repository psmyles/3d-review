### The Aud workspace's chrome: the Issues tab, the Inspector's finding and
### profile views, and the toolbar's findings count. What a check, a severity or a
### measured value is called is `labels-audit.ftl`'s, shared with `review-cli`.

## The Issues tab

ui-audit-profile-row = Profile: { $name }
    .description = The rules and limits the model is checked against. Click to see and
        change them in the Inspector.
ui-audit-copy-summary = Copy summary
    .description = Copies a short text summary of the findings, ready to paste into a
        ticket or a chat.
ui-audit-save-report = Save report...
    .description = Saves every finding to a JSON file a pipeline can read. Faces and
        points are numbered the way your modelling program numbers them.
ui-audit-group-by-check = By check
    .description = Lists each check, then the parts that fail it.
ui-audit-group-by-object = By object
    .description = Lists each part, then the checks it fails.
ui-audit-show-passed = Show passed
    .description = Also lists the checks that found nothing, and the ones that did not
        run, so you can see everything that was checked.
ui-audit-running = Checking the model...
ui-audit-no-model = Open a model to check it.
ui-audit-all-clear = Nothing to report.
    .description = Every check that ran passed. Turn on Show passed to see them.
ui-audit-scene-group = Whole file
ui-audit-row-count = { $count }
ui-audit-not-evaluated = Not checked

## Severities, statuses and why a check did not run

ui-audit-status-passed = Passed

## The toolbar's findings count

ui-audit-bubble = { $errors ->
        [one] One error
       *[other] { $errors } errors
    }, { $warnings ->
        [one] one warning
       *[other] { $warnings } warnings
    }
    .description = Checks the model fails, from the Audit workspace. Info findings are not
        counted.
ui-audit-bubble-running = Checking the model...
ui-audit-bubble-running-text = ...

## Diagnostic views and their legend

ui-audit-view-unsupported = This graphics adapter cannot draw the overdraw views: they
    need to blend into a half-float target.
ui-audit-see-also = See also
ui-audit-legend-overdraw = Surfaces per pixel
ui-audit-legend-quad-overdraw = Shading per visible pixel
ui-audit-legend-texel-target = Target: { $value }
ui-audit-legend-texel-size = Tex size: { $size }
ui-audit-legend-triangle = Triangle size at full screen
ui-audit-legend-dense = denser
ui-audit-legend-coarse = coarser

## The Inspector: a finding

ui-audit-measured = Measured
ui-audit-limit = Limit
ui-audit-profile = Profile
ui-audit-what = What this checks
ui-audit-why = Why it matters
ui-audit-fix = How to fix it
ui-audit-objects = Parts
    .description = Click a part to show only its problems.
ui-audit-scene-wide = The whole file
ui-audit-truncated = Showing the first { $shown } of { $total }.
ui-audit-summary-heading = Summary
ui-audit-summary-errors = Errors
ui-audit-summary-warnings = Warnings
ui-audit-summary-info = Info
ui-audit-summary-passed = Passed
ui-audit-summary-not-evaluated = Not checked
ui-audit-summary-time = Time
ui-audit-summary-hint = Pick a finding in the Issues list to see it on the model.

## The Inspector: the profile

ui-audit-profile-heading = Audit profile
ui-audit-profile-engine = Target engine
    .description = The engine the model is going to. It sets the starting values of every
        check, and how each finding explains why it matters.
ui-audit-profile-reset = Reset to { $engine }
    .description = Replaces every setting with the built-in values for this engine.
ui-audit-profile-save = Save profile...
    .description = Saves this profile to a file you can share, so a team checks every model
        the same way. The file's name becomes the profile's name.
ui-audit-profile-load = Load profile...
    .description = Replaces this profile with one from a file.
ui-audit-category-enabled = All checks in this category
    .description = Turns every check in this category on or off. A dash means some of
        them are on and some are off.
ui-audit-rule-enabled = Check
    .description = Whether this check runs.
ui-audit-rule-severity = Severity
    .description = How serious a failure of this check is. Info findings are not counted on
        the toolbar.
ui-audit-pattern-hint = Patterns, separated by commas (SM_*, *_LOD?)
ui-audit-pattern-invalid = Not a valid pattern

## Values

ui-audit-value-milliseconds = { $value } ms
