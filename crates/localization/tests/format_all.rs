//! Every message must format, with the arguments it declares.
//!
//! The build script checks the catalog *parses* and that every locale's ids and
//! variables match English. This checks the other half: that each pattern
//! actually resolves at run time. A `{ $count -> }` selector with no default
//! variant, or a function reference Fluent does not know, parses fine and fails
//! only when someone opens the screen that shows it.

use review_localization::{FluentArgs, FluentValue, catalog};

/// Format every message once with string arguments and once with numbers.
///
/// Both, because a selector branches on the argument's *type*: a plural rule
/// given a string falls to the default variant and never exercises the others,
/// and a message written for a number but handed a string reports no error while
/// silently choosing the wrong branch.
///
/// Through the public API, so in the locale `init` settles on — English here,
/// since `init` is a `OnceLock` and a test binary gets one. Each locale's own
/// bundle is checked by `every_locale_formats_every_message_it_defines` in the
/// crate's unit tests, which can reach them all.
#[test]
fn every_message_formats() {
    for message in catalog::MESSAGES {
        for numeric in [false, true] {
            let mut args = FluentArgs::new();
            for variable in message.vars {
                let value = if numeric {
                    FluentValue::from(2)
                } else {
                    FluentValue::from("x")
                };
                args.set(*variable, value);
            }

            let key = review_localization::Key::new(message.id, message.attr);
            let text = review_localization::tr_args(key, &args);

            assert_ne!(
                text,
                message.id,
                "`{}`{} did not resolve — no bundle defines it",
                message.id,
                message.attr.map(|a| format!(".{a}")).unwrap_or_default(),
            );
            assert!(!text.is_empty(), "`{}` formatted to nothing", message.id);
            // Fluent's isolation marks have no glyph in either bundled font, so a
            // message carrying one would render as tofu around every variable.
            assert!(
                !text.contains('\u{2068}') && !text.contains('\u{2069}'),
                "`{}` carries Unicode isolation marks — `set_use_isolating(false)` \
                 has been lost",
                message.id,
            );
            // A multiline value keeps the newlines it was *written* with, so a
            // description wrapped across three lines in the catalog arrived as
            // three hard lines and broke a tooltip a third of the way across its
            // width. The runtime undoes the wrapping; this is what says it still
            // does, for every message rather than the one in the unit test. A
            // deliberate break is a blank line in the source, and there are none
            // - add this message to the exception list if that ever changes.
            assert!(
                !text.contains('\n'),
                "`{}` formats to text with a line break in it. A catalog is wrapped \
                 for readability and the runtime is supposed to undo that, so a \
                 surviving break means either `unwrap_source_wrapping` has been \
                 lost or the message has a blank line in it",
                message.id,
            );
        }
    }
}

/// The catalog must not be empty, and it must carry the messages the chrome is
/// built from. A generator that silently produced nothing would make every other
/// assertion here vacuous.
#[test]
fn the_catalog_is_populated() {
    assert!(
        catalog::MESSAGES.len() > 100,
        "only {} messages — has the catalog been truncated?",
        catalog::MESSAGES.len()
    );
    for prefix in ["common-", "ui-", "app-", "menu-"] {
        assert!(
            catalog::MESSAGES
                .iter()
                .any(|message| message.id.starts_with(prefix)),
            "no `{prefix}` messages in the catalog"
        );
    }
}
