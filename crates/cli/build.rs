//! Generates the typed message keys of `cli.ftl` (invariant 12), and hands the
//! product version to the binary.
//!
//! The version comes from `product.json`, the one place a version is edited,
//! rather than from this crate's manifest: the release scripts write it into
//! `crates/app/Cargo.toml` and nowhere else, and `--version` and every report
//! have to name the build the viewer beside it is.

use std::path::PathBuf;

const PRODUCT_JSON: &str = "../../product.json";

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let locales = manifest.join("../localization/locales");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Err(error) = review_localization_build::generate_keys(&locales, &["cli-"], &out) {
        panic!("localization catalog: {error}");
    }

    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    let version = std::fs::read_to_string(manifest.join(PRODUCT_JSON))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|product| product.get("version")?.as_str().map(str::to_owned));
    let version = version.unwrap_or_else(|| {
        println!("cargo:warning=no version in {PRODUCT_JSON}; review-cli reports 0.0.0");
        "0.0.0".to_owned()
    });
    println!("cargo:rustc-env=REVIEW_VERSION={version}");
}
