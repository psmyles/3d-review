//! Generates the menu bar's typed message keys (invariant 12).
//!
//! macOS only: the menu is the one piece of chrome AppKit draws rather than egui,
//! and every string in it is `cfg`-gated to that platform. Generating the keys
//! anywhere else would make each one dead code, which `-D warnings` would then
//! fail the build on.

fn main() {
    #[cfg(target_os = "macos")]
    {
        let locales = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../localization/locales");
        let out =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        if let Err(error) = review_localization_build::generate_keys(&locales, &["menu-"], &out) {
            panic!("localization catalog: {error}");
        }
    }
}
