//! Publishes the product identity the binary needs, and — on Windows — embeds the
//! executable icon and version/product metadata.
//!
//! Product identity (name, author, copyright, version) comes from the canonical
//! `product.json` at the repo root, so the exe, the installer, the macOS bundle and
//! the UI all agree on one source of truth. It reaches the compiled binary as the
//! `REVIEW_PRODUCT` / `REVIEW_VERSION` / `REVIEW_COPYRIGHT` environment variables,
//! read back with `env!` — the macOS About panel (`mac-port-plan.md` D15) is what
//! wants them, and it wants them as constants because `product.json` is a repo file,
//! not something the app ships. That half runs on **every** host.
//!
//! The Windows half is the icon and the version-info string table. The icon is what
//! Explorer shows for `3d-review.exe` and what a *pinned*/unlaunched taskbar
//! shortcut uses — a Win32 resource baked into the binary at build time, distinct
//! from the runtime window icon set via winit (see `load_window_icon` in `main.rs`).
//! The string table is what Explorer's Properties → Details tab reads (Product name,
//! File version, Copyright). Its macOS twin is the `Info.plist` and `.icns` that
//! `scripts/build-mac.sh` writes, which no build script can embed.
//!
//! Nothing here is fatal: a `product.json` that cannot be read or parsed, or a
//! missing icon compiler (rc.exe / llvm-rc), is a warning and a fallback, so broken
//! metadata never blocks a build of the viewer itself.

use std::path::PathBuf;

use serde_json::Value;

const PRODUCT_JSON: &str = "../../product.json";

/// The fields of `product.json` this script publishes or embeds, already defaulted.
///
/// `description` and `author` reach only the Windows resource table, so off Windows
/// they are read by nothing — collected all the same, because the parse is one place
/// and splitting the struct per host would buy a `cfg` and no clarity.
struct Product {
    name: String,
    homepage: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    description: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    author: String,
    copyright: String,
    version: String,
}

fn main() {
    generate_message_keys();

    println!("cargo:rerun-if-changed={PRODUCT_JSON}");
    let product = read_product();

    // What the running binary reads back with `env!`.
    println!("cargo:rustc-env=REVIEW_PRODUCT={}", product.name);
    println!("cargo:rustc-env=REVIEW_VERSION={}", product.version);
    println!("cargo:rustc-env=REVIEW_COPYRIGHT={}", product.copyright);

    // The crate version (from Cargo.toml) and product.json must agree; warn loudly
    // if they drift so the exe, the bundle and the installer never disagree.
    if let Ok(crate_version) = std::env::var("CARGO_PKG_VERSION")
        && crate_version != product.version
    {
        println!(
            "cargo:warning=version mismatch: product.json is {} but \
             crates/app/Cargo.toml is {crate_version} — keep them in sync",
            product.version
        );
    }

    // The homepage the Help window links to, so the one place it is written is
    // `product.json` alongside the name and version.
    println!("cargo:rustc-env=REVIEW_HOMEPAGE={}", product.homepage);

    embed_windows_resources(&product);
}

/// Turn the English Fluent catalog into this crate's typed message keys
/// (invariant 12).
///
/// `app-` only: this crate raises notices, opens dialogs and sets the window
/// title, and nothing else. Handing it `common-` as well would pull in every
/// message the chrome shares — each of which the compiler would then report as
/// dead code here, drowning the one warning that matters.
fn generate_message_keys() {
    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Err(error) = review_localization_build::generate_keys(&locales, &["app-"], &out) {
        panic!("localization catalog: {error}");
    }
}

/// `product.json`, with a warned-about fallback for every way it can go wrong.
fn read_product() -> Product {
    let parsed = match std::fs::read_to_string(PRODUCT_JSON) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(value) => Some(value),
            Err(err) => {
                println!("cargo:warning=failed to parse {PRODUCT_JSON}: {err}");
                None
            }
        },
        Err(err) => {
            println!("cargo:warning=failed to read {PRODUCT_JSON}: {err}");
            None
        }
    };
    let field = |key: &str| -> Option<String> {
        parsed
            .as_ref()?
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let name = field("productName").unwrap_or_else(|| "3D Review".to_owned());
    Product {
        homepage: field("homepage").unwrap_or_default(),
        description: field("description").unwrap_or_else(|| name.clone()),
        author: field("author").unwrap_or_default(),
        copyright: field("copyright").unwrap_or_default(),
        version: field("version").unwrap_or_else(|| "0.0.0".to_owned()),
        name,
    }
}

/// Embed the executable icon and the version-info string table.
#[cfg(windows)]
fn embed_windows_resources(product: &Product) {
    const ICON: &str = "../../assets/icons/application-logo.ico";
    println!("cargo:rerun-if-changed={ICON}");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ICON);

    // Task Manager's "Name" column and the Details tab both surface
    // FileDescription, so it must be the product name — not the longer marketing
    // description, which goes in Comments instead.
    res.set("ProductName", &product.name);
    res.set("FileDescription", &product.name);
    res.set("Comments", &product.description);
    res.set("CompanyName", &product.author);
    res.set("LegalCopyright", &product.copyright);
    res.set("ProductVersion", &product.version);
    res.set("FileVersion", &product.version);

    if let Some(packed) = pack_version(&product.version) {
        res.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
    }

    if let Err(err) = res.compile() {
        println!("cargo:warning=failed to embed application resources: {err}");
    }
}

/// The non-Windows arm: there is no executable resource to embed, and the identity
/// published above is all any other host needs from this script.
#[cfg(not(windows))]
fn embed_windows_resources(product: &Product) {
    let _ = product;
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
