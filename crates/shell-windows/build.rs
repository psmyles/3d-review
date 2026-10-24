//! The shell extension's identity, from `product.json` — the one place the
//! installer, the registration script and this DLL all read it from — plus its
//! typed message keys (invariant 12) and its version resource.
//!
//! Windows only, judged by the platform being built *for* (`CARGO_CFG_TARGET_OS`;
//! see `shell-macos/build.rs` for why the host's own `cfg` is the wrong
//! question). Anywhere else the crate is empty and needs none of it.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde_json::Value;

const PRODUCT_JSON: &str = "../../product.json";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));

    // LNK4104 asks for `DllGetClassObject` / `DllCanUnloadNow` to be exported
    // PRIVATE (left out of the import library). The export list rustc writes has
    // no way to say so, and nothing links against this DLL's import library —
    // COM finds the exports by name — so the warning says nothing to act on.
    println!("cargo:rustc-cdylib-link-arg=/IGNORE:4104");

    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
    if let Err(error) =
        review_localization_build::generate_keys(&locales, &["shell-preview-"], &out)
    {
        panic!("localization catalog: {error}");
    }

    let product = std::fs::read_to_string(PRODUCT_JSON)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| {
            panic!("{PRODUCT_JSON} must parse: the shell extension's CLSIDs are in it")
        });
    let text = |value: &Value, key: &str| -> String {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{PRODUCT_JSON} has no `{key}`"))
            .to_owned()
    };
    let name = text(&product, "productName");
    let shell = product
        .get("shellExtension")
        .unwrap_or_else(|| panic!("{PRODUCT_JSON} has no `shellExtension`"));
    println!("cargo:rustc-env=REVIEW_PRODUCT={name}");

    let mut ids = String::from("// Generated from product.json by build.rs.\n");
    for (constant, key) in [
        ("THUMBNAIL_CLSID", "thumbnailClsid"),
        ("PREVIEW_CLSID", "previewClsid"),
    ] {
        let guid = text(shell, key);
        let hex: String = guid.chars().filter(char::is_ascii_hexdigit).collect();
        assert_eq!(
            hex.len(),
            32,
            "{key} in {PRODUCT_JSON} is not a GUID: {guid}"
        );
        let _ = writeln!(
            ids,
            "/// `{guid}`, product.json's `shellExtension.{key}`.\n\
             pub(crate) const {constant}: windows::core::GUID = windows::core::GUID::from_u128(0x{hex});"
        );
    }
    std::fs::write(out.join("ids.rs"), ids).expect("write ids.rs");

    embed_version(&product, &name);
}

/// The version-info table Explorer's Properties > Details shows for the DLL.
#[cfg(windows)]
fn embed_version(product: &Value, name: &str) {
    let field = |key: &str| {
        product
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", name);
    res.set("FileDescription", name);
    res.set("CompanyName", &field("author"));
    res.set("LegalCopyright", &field("copyright"));
    res.set("ProductVersion", &field("version"));
    res.set("FileVersion", &field("version"));
    if let Err(error) = res.compile() {
        println!("cargo:warning=no version resource for the shell extension: {error}");
    }
}

#[cfg(not(windows))]
fn embed_version(_product: &Value, _name: &str) {}
