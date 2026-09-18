use std::path::{Path, PathBuf};

/// Compile the vendored C/C++ libraries, mirroring how `crates/import/build.rs`
/// gates vendored ufbx: each tree is *optional*, so its absence is not a build
/// failure — it just leaves the matching `cfg` unset and the dependent
/// operations report the matching "unavailable" error.
///
/// They are separate `cc::Build` invocations because the trees disagree about
/// language and flags: meshoptimizer is exception-free C++, ufbx_write is C, and
/// Instant Meshes is C++17 over Eigen and throws. One build cannot compile them
/// all correctly.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_meshopt)");
    println!("cargo:rustc-check-cfg=cfg(has_ufbxw)");
    println!("cargo:rustc-check-cfg=cfg(has_ufbxw_probe)");
    println!("cargo:rustc-check-cfg=cfg(has_instant_meshes)");

    build_meshoptimizer();
    build_ufbx_write();
    build_instant_meshes();
}

fn build_meshoptimizer() {
    let dir = Path::new("../../third_party/meshoptimizer");
    let header = dir.join("meshoptimizer.h");

    println!("cargo:rerun-if-changed={}", dir.display());

    if !header.exists() {
        return;
    }

    let Some(sources) = cpp_sources(dir) else {
        return;
    };
    if sources.is_empty() {
        return;
    }

    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    println!("cargo:rerun-if-changed={}", header.display());

    cc::Build::new()
        // meshoptimizer is C++ (no STL, no exceptions) behind a pure-C API.
        // `cpp(true)` also links the C++ runtime, which its default allocator
        // needs (global `operator new` / `operator delete`).
        .cpp(true)
        .files(&sources)
        .include(dir)
        .warnings(false)
        .compile("meshoptimizer");

    println!("cargo:rustc-cfg=has_meshopt");
}

fn build_ufbx_write() {
    let dir = Path::new("../../third_party/ufbx-write");
    let source = dir.join("ufbx_write.c");
    let header = dir.join("ufbx_write.h");
    let bridge_c = Path::new("src/export_bridge.c");
    let bridge_h = Path::new("src/export_bridge.h");

    for path in [&source, &header] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed={}", bridge_c.display());
    println!("cargo:rerun-if-changed={}", bridge_h.display());

    if !source.exists() || !header.exists() {
        return;
    }

    // Two builds, not one: the bridge is this project's own C and compiles with
    // warnings **on**, which they cannot be while the vendored writer shares the
    // invocation. The bridge is emitted first so a linker that resolves in
    // command-line order sees it before the library it calls into.
    cc::Build::new()
        .file(bridge_c)
        .include(dir)
        .include("src")
        .warnings(true)
        // The vendored header the bridge includes declares nameless unions,
        // which MSVC's /W4 flags and we do not patch vendored source over. It is
        // suppressed by number so every *other* warning still reaches the log.
        .flag_if_supported("/wd4201")
        .compile("export_bridge");
    cc::Build::new()
        .file(&source)
        .include(dir)
        .warnings(false)
        .compile("ufbxwrite");

    println!("cargo:rustc-cfg=has_ufbxw");

    // The vendored writer carries this project's patches
    // (`third_party/ufbx-write/review.patch`), and `src/ufbxw_probe.c` writes a
    // scene exercising each so `tests/ufbxw_patches.rs` can read it back. A
    // build script cannot see `cfg(test)`, so the probe is gated on the profile
    // instead: tests run under `dev`, and the release binary carries none of it.
    let probe_c = Path::new("src/ufbxw_probe.c");
    println!("cargo:rerun-if-changed={}", probe_c.display());
    println!("cargo:rerun-if-env-changed=PROFILE");
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        cc::Build::new()
            .file(probe_c)
            .include(dir)
            .warnings(true)
            .flag_if_supported("/wd4201")
            .compile("ufbxw_probe");
        println!("cargo:rustc-cfg=has_ufbxw_probe");
    }
}

/// Compile vendored Instant Meshes plus the `remesh_bridge.cpp` that drives it.
///
/// Gated on **both** trees: the retopologizer is Eigen-only linear algebra and
/// cannot build without it, so a checkout carrying one and not the other leaves
/// `has_instant_meshes` unset exactly as a missing meshoptimizer does. The
/// operation then reports `OptError::RemeshUnavailable` and everything else in
/// the workspace still builds.
///
/// Three build settings here are load-bearing rather than taste:
///
/// * `shim/` goes on the include path **first**, so the tree's own
///   `#include <tbb/...>` resolves to this project's standard-library shim
///   (`third_party/instant-meshes/shim/tbb/tbb.h`) instead of pulling in Intel
///   TBB. Not one line of the vendored sources changes for it.
/// * `opt_level(2)` in *every* profile. Eigen's expression templates collapse to
///   nothing at `-O2` and to a tower of tiny function calls at `-O0`; an
///   unoptimized build turns a hundred-thousand-triangle remesh from seconds
///   into minutes, which `cargo test` would pay on every run.
/// * `NDEBUG` + `EIGEN_NO_DEBUG` for the same reason — Eigen's bounds and
///   alignment assertions are per coefficient access.
fn build_instant_meshes() {
    let dir = Path::new("../../third_party/instant-meshes");
    let eigen = Path::new("../../third_party/eigen");
    let bridge_cpp = Path::new("src/remesh_bridge.cpp");
    let bridge_h = Path::new("src/remesh_bridge.h");

    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-changed={}", eigen.display());
    println!("cargo:rerun-if-changed={}", bridge_cpp.display());
    println!("cargo:rerun-if-changed={}", bridge_h.display());

    let sources_dir = dir.join("src");
    if !sources_dir.join("field.cpp").exists() || !eigen.join("Eigen").join("Core").exists() {
        return;
    }
    let Some(sources) = cpp_sources(&sources_dir) else {
        return;
    };
    if sources.is_empty() {
        return;
    }
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    // Applied to both builds below; the bridge includes the same headers under
    // the same assumptions, so a setting that differed between them would give
    // the two halves incompatible Eigen types.
    let configure = |build: &mut cc::Build| {
        build
            .cpp(true)
            .include(dir.join("shim"))
            .include(&sources_dir)
            .include(dir.join("ext"))
            .include(eigen)
            .define("EIGEN_MPL2_ONLY", None)
            .define("EIGEN_NO_DEBUG", None)
            .define("NDEBUG", None)
            .opt_level(2);
        // MSVC C++17 + exceptions: the extraction throws `std::runtime_error` and
        // the bridge catches it, so the unwind tables have to exist. `/bigobj`
        // because an Eigen-heavy translation unit exceeds the default section
        // limit.
        build.flag_if_supported("/std:c++17");
        build.flag_if_supported("/EHsc");
        build.flag_if_supported("/bigobj");
        // clang/gcc C++17 (macOS and any other non-MSVC toolchain).
        build.flag_if_supported("-std=c++17");
    };

    // Two builds, not one, for the reason `build_ufbx_write` splits: the bridge
    // is this project's own C++ and compiles with warnings **on**, which they
    // cannot be while the vendored tree shares the invocation. The bridge is
    // emitted first so a linker that resolves in command-line order sees it
    // before the library it calls into.
    let mut bridge = cc::Build::new();
    configure(&mut bridge);
    bridge.file(bridge_cpp).include("src").warnings(true);
    // Eigen's headers trip MSVC's /W4 on unreferenced formal parameters and on
    // its own `__declspec(align)` padding; suppressed by number so every *other*
    // warning in the bridge still reaches the log.
    bridge.flag_if_supported("/wd4100");
    bridge.flag_if_supported("/wd4324");
    bridge.compile("remesh_bridge");

    let mut vendored = cc::Build::new();
    configure(&mut vendored);
    vendored.files(&sources).warnings(false);
    vendored.compile("instant_meshes");

    println!("cargo:rustc-cfg=has_instant_meshes");
}

/// Every `*.cpp` in the vendored directory, sorted so the compile order (and
/// therefore the archive layout) is reproducible across machines.
fn cpp_sources(dir: &Path) -> Option<Vec<PathBuf>> {
    let mut sources: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "cpp").then_some(path)
        })
        .collect();
    sources.sort();
    Some(sources)
}
