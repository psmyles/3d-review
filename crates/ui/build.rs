//! Generates the two compile-time tables invariant 12 rests on:
//!
//! * the typed message keys, from the English Fluent catalog — so a key the
//!   chrome names is checked by the compiler and one nothing names is a
//!   `dead_code` warning;
//! * the manual's page table, from the mdBook under `docs/book/src` — so a panel
//!   naming a page that does not exist, or a page linking to one, is a build
//!   error rather than a 404 on the published site.
//!
//! Neither reads anything a plain `cargo build` does not already have: the
//! catalog and the book are both committed source.

use std::path::PathBuf;

#[path = "build/pages.rs"]
mod pages;

fn main() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

    // `common-` and `ui-`: the chrome's own strings plus the ones it shares with
    // `app`. Anything else in the catalog belongs to another crate and would only
    // show up here as an unused key.
    let locales = workspace.join("crates/localization/locales");
    if let Err(error) =
        review_localization_build::generate_keys(&locales, &["common-", "ui-"], &out)
    {
        panic!("localization catalog: {error}");
    }

    if let Err(error) = pages::generate(&workspace.join("docs/book/src"), &out) {
        panic!("manual: {error}");
    }
}
