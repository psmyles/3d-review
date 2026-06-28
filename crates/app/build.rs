//! Embeds the Windows executable icon and version/product metadata. The icon is
//! what Explorer shows for `3d-review.exe` and what a *pinned*/unlaunched taskbar
//! shortcut uses — it is a Win32 resource baked into the binary at build time,
//! distinct from the runtime window icon set via winit (see `load_window_icon`
//! in `main.rs`). The version-info string table is what Explorer's
//! Properties → Details tab reads (Product name, File version, Copyright).
//!
//! Product identity (name, author, copyright, version) comes from the canonical
//! `product.json` at the repo root, so the exe, the installer, and the UI all
//! agree on one source of truth.
//!
//! No-op on non-Windows targets. A missing icon compiler (rc.exe / llvm-rc) is
//! downgraded to a warning so the build still succeeds without the embedded
//! resource rather than failing outright.

#[cfg(windows)]
fn main() {
    use serde_json::Value;

    const ICON: &str = "../../assets/icons/application-logo.ico";
    const PRODUCT_JSON: &str = "../../product.json";

    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed={PRODUCT_JSON}");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ICON);

    // Pull product identity from product.json. If it can't be read/parsed, warn
    // and fall back to the winresource defaults rather than failing the build.
    match std::fs::read_to_string(PRODUCT_JSON).map(|raw| serde_json::from_str::<Value>(&raw)) {
        Ok(Ok(meta)) => {
            let get = |key: &str| meta.get(key).and_then(Value::as_str);

            let product_name = get("productName").unwrap_or("3D Review");
            let description = get("description").unwrap_or(product_name);
            let author = get("author").unwrap_or("");
            let copyright = get("copyright").unwrap_or("");
            let version = get("version").unwrap_or("0.0.0");

            // The crate version (from Cargo.toml) and product.json must agree;
            // warn loudly if they drift so the exe and installer never disagree.
            if let Ok(crate_version) = std::env::var("CARGO_PKG_VERSION")
                && crate_version != version
            {
                println!(
                    "cargo:warning=version mismatch: product.json is {version} but \
                     crates/app/Cargo.toml is {crate_version} — keep them in sync"
                );
            }

            // Task Manager's "Name" column and the Details tab both surface
            // FileDescription, so it must be the product name — not the longer
            // marketing description, which goes in Comments instead.
            res.set("ProductName", product_name);
            res.set("FileDescription", product_name);
            res.set("Comments", description);
            res.set("CompanyName", author);
            res.set("LegalCopyright", copyright);
            res.set("ProductVersion", version);
            res.set("FileVersion", version);

            if let Some(packed) = pack_version(version) {
                res.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
                res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
            }
        }
        Ok(Err(err)) => println!("cargo:warning=failed to parse {PRODUCT_JSON}: {err}"),
        Err(err) => println!("cargo:warning=failed to read {PRODUCT_JSON}: {err}"),
    }

    if let Err(err) = res.compile() {
        println!("cargo:warning=failed to embed application resources: {err}");
    }
}

/// Pack a dotted `major.minor.patch` string into the u64 layout winresource
/// expects for `FILEVERSION`/`PRODUCTVERSION` (`major<<48 | minor<<32 |
/// patch<<16 | build`). Returns `None` if the string isn't parseable so the
/// caller can skip the binary version fields and keep the string table.
#[cfg(windows)]
fn pack_version(version: &str) -> Option<u64> {
    let mut parts = version.split('.').map(str::parse::<u64>);
    let major = parts.next()?.ok()?;
    let minor = parts.next().transpose().ok()?.unwrap_or(0);
    let patch = parts.next().transpose().ok()?.unwrap_or(0);
    Some((major << 48) | (minor << 32) | (patch << 16))
}

#[cfg(not(windows))]
fn main() {}
