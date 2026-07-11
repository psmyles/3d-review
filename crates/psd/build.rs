//! Build script for `review-psd` — a **link-only** step, no compilation.
//!
//! Unlike a normal `-sys` crate, this one does not build the vendored psd_sdk C++
//! or run bindgen. `vendor/fire_psd.lib` is a committed, prebuilt MSVC static lib
//! (release `/MD` CRT — matches Rust's default MSVC target) and `src/bindings.rs`
//! is committed bindgen output. All this script does is point the linker at the
//! vendored `.lib` and request it. The C++ CRT / STL dependencies (`MSVCRT`,
//! `msvcprt`) are carried as `DEFAULTLIB` directives inside the object files in the
//! archive, so the linker resolves them automatically — no extra link flags here.
//!
//! To regenerate the artifacts (new psd_sdk version, ABI change), see
//! `vendor/NOTICE.txt`; they are produced by fire's `psd-sdk-sys` crate, which owns
//! the full `cc` + `bindgen` compile setup.

use std::path::PathBuf;

fn main() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor");

    // Rebuild only if the prebuilt artifact or this script changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/fire_psd.lib");

    println!("cargo:rustc-link-search=native={}", vendor.display());
    println!("cargo:rustc-link-lib=static=fire_psd");
}
