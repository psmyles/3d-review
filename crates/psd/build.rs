//! Build script for `review-psd` — compiles the vendored psd_sdk C++ (`vendor/Psd/`)
//! plus the C-ABI `src/wrapper.cpp` into a static lib with `cc`.
//!
//! This is the same model `crates/import` (ufbx) and `crates/optimize` (meshoptimizer,
//! ufbx_write) already use: vendored source compiled by `cc`, no prebuilt binary and no
//! bindgen/libclang at build time. `src/bindings.rs` is committed Rust that mirrors
//! `src/wrapper.h` by hand — see that header.
//!
//! Unlike ufbx/meshoptimizer this tree is NOT existence-gated: PSD decode has no
//! "unavailable" path in `lib.rs`, so a missing `vendor/Psd/` is a build error that says
//! so rather than a silently featureless build.
//!
//! psd_sdk is already clang-aware (`PsdPch.h` sets `PSD_USE_CLANG`, `PsdPlatform.h` gates
//! `<windows.h>` on `_WIN32`), and `wrapper.cpp` reads through its own in-memory
//! `psd::File`, so the platform `NativeFile` sources are never instantiated and can all
//! be excluded from the build.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wrapper_cpp = crate_dir.join("src").join("wrapper.cpp");
    let psd_dir = crate_dir.join("vendor").join("Psd");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/wrapper.cpp");
    println!("cargo:rerun-if-changed=src/wrapper.h");
    println!("cargo:rerun-if-changed=vendor/Psd");

    if !psd_dir.join("Psd.h").exists() {
        panic!(
            "psd_sdk source missing at {} — vendor it (see vendor/NOTICE.txt)",
            psd_dir.display()
        );
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(&psd_dir)
        .include(crate_dir.join("src"))
        .file(&wrapper_cpp)
        .warnings(false); // third-party code; don't fail or make noise on its warnings

    // MSVC C++17 + exceptions (`wrapper.cpp` catches at the FFI boundary, so the
    // unwind tables have to exist). `/MD` — cc's default — is the CRT Rust's MSVC
    // target links in both debug and release.
    build.flag_if_supported("/std:c++17");
    build.flag_if_supported("/EHsc");
    // clang/gcc C++17 (macOS and any other non-MSVC toolchain).
    build.flag_if_supported("-std=c++17");

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&psd_dir)
        .expect("read vendor/Psd")
        .filter_map(|entry| {
            let path = entry.expect("read vendor/Psd entry").path();
            (path.extension().and_then(|e| e.to_str()) == Some("cpp")).then_some(path)
        })
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // The platform `NativeFile` sources. `_Linux.cpp` is POSIX aio and `_Mac.mm`
            // is Objective-C++ (never matched — not a `.cpp`); `PsdNativeFile.cpp` is the
            // Win32 `CreateFileW` + overlapped-IO one, which only compiles under MSVC.
            // None are reachable: `wrapper.cpp` reads through its own `MemoryFile`.
            !name.ends_with("_Linux.cpp") && (windows || name != "PsdNativeFile.cpp")
        })
        .collect();
    // `read_dir` order is filesystem-defined; sort so the object list — and so the
    // archive — is identical from one machine to the next.
    sources.sort();
    build.files(&sources);

    // A `cpp(true)` build emits its own C++ stdlib link flag (`c++` on macOS), and on
    // MSVC the CRT/STL arrive as `DEFAULTLIB` directives inside the objects, so there is
    // nothing to name here.
    build.compile("fire_psd");
}
