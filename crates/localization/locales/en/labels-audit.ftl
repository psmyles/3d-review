### The audit's typed values, as every surface that prints them names them: the
### viewer's Aud workspace and `review3d-cli audit` alike (`crates/labels`).
###
### Every check has four messages - its name, what it checks, why it matters
### (worded per target engine through `$engine`: unity, unreal or generic), and a
### hint for fixing it in the modelling program.

## Profile names

labels-audit-profile-modified = { $name } (changed)

## Severities, statuses and why a check did not run

labels-audit-severity-error = Error
labels-audit-severity-warning = Warning
labels-audit-severity-info = Info
labels-audit-skip-disabled = Switched off in this profile.
labels-audit-skip-no-properties = Needs the file's source properties, which could not be read.
labels-audit-skip-no-skin = The model has no skin.
labels-audit-skip-no-uv-set = The model does not have the UV set this check reads.
labels-audit-skip-invalid-pattern = One of this check's name patterns is not valid.
labels-audit-skip-empty = There is nothing in the model to check.
labels-audit-skip-cancelled = The check was interrupted.

## Categories

labels-audit-category-geometry = Geometry
labels-audit-category-transforms = Transforms
labels-audit-category-uv = UVs
labels-audit-category-skin = Skin
labels-audit-category-density = Density
labels-audit-category-naming = Naming
labels-audit-category-hierarchy = Hierarchy

## Diagnostic views and their legend

labels-audit-view-issues = Issues
    .description = Shows the finding picked in the Issues list on the model, over plain
        grey.
labels-audit-view-texel-density = Texel density
    .description = Colours each face by its texture pixels per meter: blue below the
        profile's target, green on it, red above it. The texture size and target are the
        Texel density check's, in the profile.
labels-audit-view-triangle-density = Triangle density
    .description = Colours each face by how small its triangles are for the size of their
        object: red where they are too small to draw efficiently, green at the limit, blue
        where there is room to spare. The limit is the Needs a LOD check's, in the profile.
labels-audit-view-overdraw = Overdraw
    .description = Replaces the view with a count of the surfaces under each pixel, hidden
        ones included: how many times a GPU with no depth rejection would shade it. Where
        parts of the model are stacked behind each other, the count climbs.
labels-audit-view-quad-overdraw = Quad overdraw
    .description = Replaces the view with the shading each visible pixel really costs. A
        GPU shades pixels in 2x2 quads, so a triangle that covers one pixel of a quad
        still pays for four: 1 is a surface whose triangles fill their quads, 4 is one
        made of triangles too small for the screen.

## Target engines

labels-audit-engine-unity = Unity
labels-audit-engine-unreal = Unreal
labels-audit-engine-generic = Generic

## Axes

labels-audit-axis-x = +X
labels-audit-axis-negative-x = -X
labels-audit-axis-y = +Y
labels-audit-axis-negative-y = -Y
labels-audit-axis-z = +Z
labels-audit-axis-negative-z = -Z

## Values

labels-audit-value-count = { $value }
labels-audit-value-percent = { $value } %
labels-audit-value-meters = { $value } m
labels-audit-value-degrees = { $value } deg
labels-audit-value-per-square-meter = { $value } tris per sq m
labels-audit-value-px-per-meter = { $value } px/m
labels-audit-value-pixels = { $value } px
labels-audit-value-square-pixels = { $value } sq px
labels-audit-value-screen-size = { $value } of the screen
labels-audit-value-unit = { $value } m per unit
labels-audit-value-scale = { $x } x { $y } x { $z }
labels-audit-value-range = { $min } to { $max }
labels-audit-value-at-most = at most { $value }
labels-audit-value-at-least = at least { $value }

## What a failing check found, with its figures: the "what" of a finding in Asset
## Check's results (`review3d-cli audit --results`). It follows the check's own
## "-what" sentence. `$count` is how many triangles, edges or vertices it found and
## `$objects` how many objects they are in; `$measured` is the figure measured and
## `$limit`, `$min`, `$max` and `$target` the profile's limit, each with its unit.

labels-audit-found-triangles = Found { $count ->
        [one] one triangle
       *[other] { $count } triangles
    } in { $objects ->
        [one] one object.
       *[other] { $objects } objects.
    }
labels-audit-found-edges = Found { $count ->
        [one] one edge
       *[other] { $count } edges
    } in { $objects ->
        [one] one object.
       *[other] { $objects } objects.
    }
labels-audit-found-vertices = Found { $count ->
        [one] one vertex
       *[other] { $count } vertices
    } in { $objects ->
        [one] one object.
       *[other] { $objects } objects.
    }
labels-audit-found-objects = Found in { $objects ->
        [one] one object.
       *[other] { $objects } objects.
    }
labels-audit-figure-max = Measured { $measured }; the profile allows at most { $limit }.
labels-audit-figure-min = Measured { $measured }; the profile asks for at least { $limit }.
labels-audit-figure-range = Measured { $measured }; the profile allows { $min } to { $max }.
labels-audit-figure-equals = Measured { $measured }; the profile asks for { $target }.
labels-audit-figure-measured = Measured { $measured }.
labels-audit-figure-patterns = The profile allows names matching { $patterns }.

## The copied summary

labels-audit-text-heading = Audit of { $file } against the { $profile } profile
labels-audit-text-counts = { $errors } errors, { $warnings } warnings, { $info } info, { $passed } passed
labels-audit-text-finding = { $severity }: { $rule } ({ $count })
labels-audit-text-none = No findings.

## Errors a user can act on

labels-audit-profile-newer = This audit profile was saved by a newer version of 3D Review
    (profile version { $found }; this version reads up to { $supported }). Open it with the
    version that saved it.
