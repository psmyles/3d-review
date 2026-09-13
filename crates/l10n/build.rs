//! Embeds every locale's `.ftl` files and cross-checks each translation against
//! the English catalog, which is the source of truth the generated keys are built
//! from (invariant 12).
//!
//! The work itself lives in `review-l10n-build` so there is exactly one `.ftl`
//! parser in the workspace — the same one the consumers' `build.rs` generate their
//! typed keys with, and the same one the runtime loads through. A catalog that
//! builds is therefore a catalog that loads.

use std::path::PathBuf;

fn main() {
    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

    if let Err(error) = review_l10n_build::generate_resources(&locales, &out) {
        panic!("localization catalog: {error}");
    }
}
