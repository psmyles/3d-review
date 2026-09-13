//! Turns the three optimizer errors a user can act on into text that says what
//! to do about them.
//!
//! Everything else keeps its library diagnostic verbatim (invariant 12). That is
//! deliberate: an `OptError` is written for whoever debugs it, it is what a bug
//! report quotes, and `review-optimize` must not learn about catalogs to say it.
//! But three of its variants are not really diagnostics — they describe a
//! situation the reader can resolve, and "preset could not be read (version 4,
//! this build reads 3)" tells them less than "open it with the build that wrote
//! it" does.
//!
//! A fourth variant that turns out to need this is one more arm here, not a trait
//! spread across four crates.

use review_optimize::OptError;

use crate::keys;

/// What to show the user for an optimizer error.
pub(crate) fn explain_opt_error(error: &OptError) -> String {
    match error {
        OptError::Unavailable => {
            review_localization::tr(keys::app_notifications::OPT_UNAVAILABLE).into_owned()
        }
        OptError::PresetVersion { found, supported } => {
            keys::app_notifications::preset_newer(f64::from(*found), f64::from(*supported))
        }
        // `ExportIncomplete` is deliberately absent: `handle_opt_exported` gives
        // it a report of its own that names every file this run did replace, and
        // a one-line summary here would say less.
        //
        // Genuine diagnostics: the library's own wording, unchanged.
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three actionable variants must not fall through to the library's own
    /// `Display`, and everything else must.
    #[test]
    fn only_the_actionable_variants_are_rewritten() {
        let unavailable = explain_opt_error(&OptError::Unavailable);
        assert_ne!(unavailable, OptError::Unavailable.to_string());

        let versioned = OptError::PresetVersion {
            found: 4,
            supported: 3,
        };
        assert_ne!(explain_opt_error(&versioned), versioned.to_string());

        let diagnostic = OptError::IndexCount(7);
        assert_eq!(explain_opt_error(&diagnostic), diagnostic.to_string());
    }
}
