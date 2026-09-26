//! review-localization: the localization runtime behind invariant 12.
//!
//! Every string a user can read is a Fluent message in `locales/<lang>/*.ftl`,
//! reached through a **generated typed key** rather than a literal at the call
//! site. This crate is the runtime half: it negotiates a locale once at startup,
//! holds the bundles, and resolves a [`Key`] to text. The build half —
//! `review-localization-build` — turns the English catalog into those keys inside each
//! consuming crate, so a key that does not exist is a compile error and one that
//! nothing uses is a `dead_code` warning.
//!
//! ```ignore
//! localization::init(Some("de-AT"));              // once, at the top of `main`
//! ui.label(keys::ui_stats::DRAWS);        // `Key: Into<egui::WidgetText>`
//! keys::ui_stats::scope_vis_description(primary_modifier());  // typed formatter
//! ```
//!
//! Three Fluent details this wraps, all of which are wrong by default for a
//! desktop viewer:
//!
//! * `format_pattern` brackets every variable in Unicode isolation marks
//!   (U+2068/U+2069) so a right-to-left value cannot reflow the sentence around
//!   it. Neither bundled font has a glyph for them, so a tooltip reading
//!   `⟨Ctrl⟩+click` came out as two tofu boxes. [`Localizer::new`] turns
//!   isolation off; when an RTL locale is added, the fix is a font that draws the
//!   marks, not re-concatenating text by hand.
//! * a multiline value keeps the line breaks it was *written* with. A catalog is
//!   wrapped at 90-odd columns so it stays readable and diffs one sentence at a
//!   time, and Fluent hands those wraps back as hard newlines — so a tooltip
//!   paragraph broke wherever the `.ftl` file happened to break, a third of the
//!   way across a tooltip that had room for the rest of the line.
//!   [`unwrap_source_wrapping`] undoes it: a single newline is the source's
//!   wrapping and becomes a space, a blank line is a deliberate paragraph break
//!   and survives. That is markdown's rule, and it is the one every author
//!   already knows.
//! * the default `FluentBundle` memoizes intl formatters in a `RefCell` and is
//!   therefore `!Sync`, which a process-wide `OnceLock` cannot hold. The
//!   `new_concurrent` constructor swaps in the `Mutex`-backed memoizer. Formatting
//!   still happens on the main thread by convention (workers post typed events and
//!   `app` turns them into text), but the catalog itself is shared, not per-thread.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentResource, bundle::FluentBundle as GenericBundle};
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::{FluentArgs, FluentValue};
pub use unic_langid::LanguageIdentifier as Locale;

include!(concat!(env!("OUT_DIR"), "/resources.rs"));

/// The catalog as data, so a test can format every message in every locale
/// without naming them one at a time.
pub mod catalog {
    /// One message (or one attribute) the English catalog defines.
    pub struct MessageInfo {
        /// Catalog file stem it lives in (`ui-stats`).
        pub file: &'static str,
        /// Fluent message id (`ui-stats-draws`).
        pub id: &'static str,
        /// Attribute name, for `.description` and friends.
        pub attr: Option<&'static str>,
        /// Variable names the pattern references.
        pub vars: &'static [&'static str],
    }

    include!(concat!(env!("OUT_DIR"), "/catalog.rs"));
}

/// English, the fallback every other locale falls through to and the only catalog
/// the generated keys are checked against.
///
/// Parsed rather than written with `unic_langid`'s `langid!`, which lives behind a
/// proc-macro feature and a second crate — for one tag that is never wrong.
fn fallback() -> LanguageIdentifier {
    "en".parse().expect("`en` is a valid language tag")
}

/// A message, or one attribute of a message, in the catalog.
///
/// Only generated code builds one: [`Key::new`] is hidden, and
/// `tests/no_inline_strings.rs` fails the build if it is called anywhere outside
/// a generated `keys.rs`. That is what makes "every user-visible string is in the
/// catalog" mechanical rather than a convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    id: &'static str,
    attr: Option<&'static str>,
}

impl Key {
    /// Construct a key. Called by generated code only.
    #[doc(hidden)]
    pub const fn new(id: &'static str, attr: Option<&'static str>) -> Self {
        Self { id, attr }
    }

    /// The Fluent message id, without the attribute. Also what [`tr`] falls back
    /// to when a locale is missing the message entirely, so an untranslated
    /// string shows as its id rather than as nothing.
    pub fn id(self) -> &'static str {
        self.id
    }

    /// The attribute name, if this key names one.
    pub fn attr(self) -> Option<&'static str> {
        self.attr
    }
}

/// The negotiated chain of bundles: the requested locale first, English last.
struct Localizer {
    locale: LanguageIdentifier,
    chain: Vec<FluentBundle<FluentResource>>,
}

impl Localizer {
    fn new(requested: Option<&str>) -> Self {
        let available: Vec<LanguageIdentifier> = LOCALES
            .iter()
            .filter_map(|(tag, _)| tag.parse().ok())
            .collect();

        // `Filtering` is the strategy that answers "de-AT" with ["de", "en"]
        // rather than only an exact hit, which is the whole reason to negotiate
        // instead of comparing tags.
        let requested: Vec<LanguageIdentifier> = requested
            .and_then(|tag| tag.parse().ok())
            .into_iter()
            .collect();
        let default = fallback();
        let ordered = fluent_langneg::negotiate_languages(
            &requested,
            &available,
            Some(&default),
            fluent_langneg::NegotiationStrategy::Filtering,
        );

        let mut chain = Vec::new();
        for locale in &ordered {
            if let Some(bundle) = build_bundle(locale) {
                chain.push(bundle);
            }
        }
        // Negotiation always returns the default, but a catalog it cannot build
        // (a resource that failed to parse) would leave the chain empty and every
        // lookup falling through to its raw id.
        if chain.is_empty()
            && let Some(bundle) = build_bundle(&default)
        {
            chain.push(bundle);
        }

        let locale = ordered
            .first()
            .map_or_else(fallback, |locale| (*locale).clone());
        Self { locale, chain }
    }

    /// Format a key through the first bundle in the chain that defines it.
    ///
    /// Lookup and formatting are one step because a `FluentMessage` borrows the
    /// bundle it came from: handing the pattern back to the caller would name
    /// `fluent_syntax`'s AST in this crate's signatures and pull the parser into
    /// the runtime for a type nobody outside needs.
    fn format(&self, key: Key, args: Option<&FluentArgs<'_>>) -> Option<String> {
        self.chain.iter().find_map(|bundle| {
            let message = bundle.get_message(key.id)?;
            let pattern = match key.attr {
                Some(attr) => message.get_attribute(attr).map(|attr| attr.value())?,
                None => message.value()?,
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, args, &mut errors);
            debug_assert!(
                errors.is_empty(),
                "`{}`{}: {errors:?}",
                key.id,
                key.attr.map(|a| format!(".{a}")).unwrap_or_default(),
            );
            Some(unwrap_source_wrapping(text.into_owned()))
        })
    }
}

/// Undo the line wrapping a catalog file is written with.
///
/// Fluent's multiline values keep every newline the author typed, so a
/// description wrapped across three source lines arrived as three hard lines and
/// broke a tooltip a third of the way across its width. Text destined for a
/// wrapping widget must arrive as one run and let the widget break it where it
/// actually runs out of room.
///
/// The rule is markdown's, so nothing new has to be learnt to write a catalog: a
/// single newline is wrapping and becomes a space; a blank line is a deliberate
/// paragraph break and stays one. Every line is trimmed, which is what removes
/// the continuation indent Fluent leaves on a value whose first line is on the
/// `key =` line itself.
fn unwrap_source_wrapping(text: String) -> String {
    // The overwhelmingly common case: a label, on one line, unchanged and
    // un-reallocated.
    if !text.contains('\n') {
        return text;
    }

    let mut out = String::with_capacity(text.len());
    let mut paragraph_break = false;
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            paragraph_break = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if paragraph_break { "\n\n" } else { " " });
        }
        paragraph_break = false;
        out.push_str(line);
    }
    out
}

fn build_bundle(locale: &LanguageIdentifier) -> Option<FluentBundle<FluentResource>> {
    let (_, files) = LOCALES.iter().find(|(tag, _)| {
        tag.parse::<LanguageIdentifier>()
            .is_ok_and(|tag| tag == *locale)
    })?;
    let mut bundle = GenericBundle::new_concurrent(vec![locale.clone()]);
    // See the module docs: the isolation marks Fluent adds around every variable
    // have no glyph in the bundled fonts.
    bundle.set_use_isolating(false);
    for (name, source) in *files {
        match FluentResource::try_new((*source).to_owned()) {
            Ok(resource) => {
                if let Err(errors) = bundle.add_resource(resource) {
                    debug_assert!(
                        false,
                        "{locale}/{name}.ftl: {errors:?} — the build script validates the \
                         catalog, so this means the two parsers disagree"
                    );
                }
            }
            Err((_, errors)) => debug_assert!(
                false,
                "{locale}/{name}.ftl failed to parse at run time: {errors:?}"
            ),
        }
    }
    Some(bundle)
}

static LOCALIZER: OnceLock<Localizer> = OnceLock::new();

fn localizer() -> &'static Localizer {
    // A `tr` before `init` (a panic message on the way up, say) must still return
    // text rather than deadlock or panic, so this settles on English rather than
    // waiting for a call that may never come.
    LOCALIZER.get_or_init(|| Localizer::new(None))
}

/// Choose the locale and build the catalogs. Call once, at the top of `main`,
/// before anything formats a message.
///
/// `requested` is the tag the caller settled on — a `--locale` flag, a saved
/// setting, or the OS locale. It is negotiated against the embedded catalogs, so
/// an unknown or partial tag lands on the nearest match and finally on English
/// rather than failing. A second call is ignored: the catalog is process-wide and
/// swapping it mid-run would leave already-formatted text in the old language.
pub fn init(requested: Option<&str>) {
    let _ = LOCALIZER.set(Localizer::new(requested));
}

/// The locale [`init`] settled on. English until it is called.
pub fn locale() -> &'static LanguageIdentifier {
    &localizer().locale
}

/// Every locale with an embedded catalog, English first.
pub fn available_locales() -> Vec<LanguageIdentifier> {
    LOCALES
        .iter()
        .filter_map(|(tag, _)| tag.parse().ok())
        .collect()
}

/// Resolve a message that takes no variables.
///
/// Falls through the negotiated chain to English and finally to the key's own id,
/// so a missing translation degrades to English and a missing *message* shows an
/// id rather than an empty label. Both are `debug_assert` failures: the build
/// script cannot catch a message that only one locale defines a value for.
pub fn tr(key: Key) -> Cow<'static, str> {
    match localizer().format(key, None) {
        Some(text) => Cow::Owned(text),
        None => {
            debug_assert!(false, "no catalog defines `{}`", key.id);
            Cow::Borrowed(key.id)
        }
    }
}

/// Resolve a message with variables. Prefer the generated typed formatter beside
/// the key, which fills this in with one parameter per variable — a missing
/// argument is then a compile error rather than a `{$name}` left in the text.
pub fn tr_args(key: Key, args: &FluentArgs<'_>) -> String {
    match localizer().format(key, Some(args)) {
        Some(text) => text,
        None => {
            debug_assert!(false, "no catalog defines `{}`", key.id);
            key.id.to_owned()
        }
    }
}

#[cfg(feature = "egui")]
impl From<Key> for egui::WidgetText {
    fn from(key: Key) -> Self {
        tr(key).into_owned().into()
    }
}

#[cfg(feature = "egui")]
impl From<Key> for egui::RichText {
    fn from(key: Key) -> Self {
        egui::RichText::new(tr(key).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_always_available() {
        assert!(available_locales().contains(&fallback()));
    }

    #[test]
    fn an_unknown_locale_falls_back_to_english() {
        // Not through `init` — that is a `OnceLock` and other tests in this
        // binary share it. `Localizer` is what `init` stores, so this exercises
        // the negotiation directly.
        let localizer = Localizer::new(Some("zz-ZZ"));
        assert_eq!(localizer.locale, fallback());
        assert!(!localizer.chain.is_empty());
    }

    /// The bug this was written for: a description wrapped across three lines in
    /// the catalog broke a tooltip into three lines a third of its width.
    #[test]
    fn source_wrapping_becomes_one_run_of_text() {
        let wrapped = "A checker pattern on the surface: the fast read on stretching,\n\
                       mirroring and texel density."
            .to_owned();
        assert_eq!(
            unwrap_source_wrapping(wrapped),
            "A checker pattern on the surface: the fast read on stretching, mirroring \
             and texel density."
        );
    }

    /// A blank line is the author asking for a break, and is the only way to get
    /// one — so it has to survive.
    #[test]
    fn a_blank_line_stays_a_paragraph_break() {
        let text = "First paragraph,\nwrapped.\n\nSecond one.".to_owned();
        assert_eq!(
            unwrap_source_wrapping(text),
            "First paragraph, wrapped.\n\nSecond one."
        );
    }

    /// The common case must not allocate a second string, since every label in
    /// the chrome goes through it every frame.
    #[test]
    fn a_single_line_is_returned_untouched() {
        let text = String::from("Wireframe Only");
        let pointer = text.as_ptr();
        let out = unwrap_source_wrapping(text);
        assert_eq!(out, "Wireframe Only");
        assert_eq!(out.as_ptr(), pointer, "the single-line case reallocated");
    }

    #[test]
    fn a_malformed_tag_does_not_panic() {
        let localizer = Localizer::new(Some("not a language tag"));
        assert_eq!(localizer.locale, fallback());
    }
}
