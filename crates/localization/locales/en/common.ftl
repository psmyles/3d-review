### Strings more than one crate shows, or that appear in many places.
###
### Every id in this file starts with `common-`; the generator turns that into
### `keys::common::*` in whichever crate asks for the `common-` prefix.

# The platform's own file-command modifier, as the chrome names it. `app`'s
# `shortcuts::primary_held` is the dispatch side of the same decision (D10); this
# is only what the UI calls the key. Two messages rather than one with a variant,
# because a translation may want to keep "Cmd" untranslated while translating
# "Ctrl", or the reverse.
common-modifier-ctrl = Ctrl
common-modifier-cmd = Cmd

common-reset-all = Reset all
    .description = Puts every setting in this panel back to how it started.

common-help = Help
common-help-open = Open the manual page for this tool.
common-learn-more = Learn more

common-copied = Copied to clipboard
