//! Generates this crate's typed message keys from the English Fluent catalog
//! (invariant 12): `labels-` only, the files whose messages name a typed value.

use std::path::PathBuf;

fn main() {
    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Err(error) = review_localization_build::generate_keys(&locales, &["labels-"], &out) {
        panic!("localization catalog: {error}");
    }
}
