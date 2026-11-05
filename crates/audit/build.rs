//! Hands the product's name and version to the crate from `product.json`, the one place
//! they're set, for the `generator` line of the profiles it writes.

use std::path::PathBuf;

const PRODUCT_JSON: &str = "../../product.json";

fn main() {
    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let product: serde_json::Value = std::fs::read_to_string(manifest.join(PRODUCT_JSON))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| panic!("{PRODUCT_JSON} must parse"));
    for (var, key) in [
        ("REVIEW_PRODUCT", "productName"),
        ("REVIEW_VERSION", "version"),
    ] {
        let value = product
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{PRODUCT_JSON} has no `{key}`"));
        println!("cargo:rustc-env={var}={value}");
    }
}
