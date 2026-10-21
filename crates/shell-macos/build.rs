//! Generates the menu bar's typed message keys (invariant 12).
//!
//! macOS only: the menu is the one piece of chrome AppKit draws rather than egui,
//! and every string in it is `cfg`-gated to that platform. Generating the keys
//! anywhere else would make each one dead code, which `-D warnings` would then
//! fail the build on.
//!
//! "macOS" is the platform being built *for*, which a build script has to read
//! from `CARGO_CFG_TARGET_OS`: its own `#[cfg(target_os)]` describes the host it
//! runs on, so a check of the Mac build from Windows (`cargo check --target
//! aarch64-apple-darwin`) generated nothing and `menubar.rs`'s `include!` failed.
//! That is also why `review-localization-build` is a build dependency on every
//! host rather than a macOS-only one: Cargo matches a build dependency's
//! platform against the host, and a Windows host checking the Mac build still
//! needs it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let locales =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Err(error) = review_localization_build::generate_keys(&locales, &["menu-"], &out) {
        panic!("localization catalog: {error}");
    }
}
