//! The extension's typed message keys (invariant 12), the product name its log
//! folder is named after, and the dylib's install name.
//!
//! macOS only, judged by the platform being built *for* (`CARGO_CFG_TARGET_OS`;
//! see `shell-macos/build.rs` for why the host's own `cfg` is the wrong
//! question). Anywhere else the crate is empty and needs none of it.

use std::path::PathBuf;

use serde_json::Value;

const PRODUCT_JSON: &str = "../../product.json";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

    // Found by the extensions through their `@rpath` (the containing app's
    // `Contents/Frameworks`), not by the path it happened to be built at.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libreview_quicklook.dylib");

    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
    if let Err(error) =
        review_localization_build::generate_keys(&locales, &["shell-preview-"], &out)
    {
        panic!("localization catalog: {error}");
    }

    let name = std::fs::read_to_string(PRODUCT_JSON)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|product| product.get("productName")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("{PRODUCT_JSON} must name the product"));
    println!("cargo:rustc-env=REVIEW_PRODUCT={name}");
}
