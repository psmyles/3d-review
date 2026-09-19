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
    println!("cargo:rustc-check-cfg=cfg(has_quadriflow)");

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
    let density_cpp = Path::new("src/remesh_density.cpp");
    let density_h = Path::new("src/remesh_density.h");

    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-changed={}", eigen.display());
    println!("cargo:rerun-if-changed={}", bridge_cpp.display());
    println!("cargo:rerun-if-changed={}", bridge_h.display());
    println!("cargo:rerun-if-changed={}", density_cpp.display());
    println!("cargo:rerun-if-changed={}", density_h.display());

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
            // MSVC declares neither `M_PI` nor its siblings without it, and
            // both engines' headers use them (Instant Meshes' `common.h`,
            // QuadriFlow's `field-math.hpp`) — so it belongs to the shared
            // settings rather than to one of the four builds.
            .define("_USE_MATH_DEFINES", None)
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

    // QuadriFlow is a second, *optional* engine behind the same bridge: it is
    // the only thing that produces an all-quad mesh, and without it the Remesh
    // operation falls back to Instant Meshes with a warning. So it is gated
    // separately, and the bridge is told which of the two it can reach.
    let quadriflow = build_quadriflow(&configure);

    // Two builds, not one, for the reason `build_ufbx_write` splits: the bridge
    // is this project's own C++ and compiles with warnings **on**, which they
    // cannot be while the vendored tree shares the invocation. The bridge is
    // emitted first so a linker that resolves in command-line order sees it
    // before the library it calls into.
    let mut bridge = cc::Build::new();
    configure(&mut bridge);
    bridge.file(bridge_cpp).include("src").warnings(true);
    if quadriflow {
        bridge.define("REVIEW_HAS_QUADRIFLOW", None);
    }
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

    // The density field is this project's own C++ and touches neither engine's
    // headers — it takes flat arrays, which is the whole reason it can be
    // shared. Compiled last because both drivers call into it and a linker that
    // resolves in command-line order wants the callee after its callers.
    let mut density = cc::Build::new();
    density.cpp(true).warnings(true).opt_level(2);
    density.flag_if_supported("/std:c++17");
    density.flag_if_supported("/EHsc");
    density.flag_if_supported("-std=c++17");
    density.file(density_cpp).include("src");
    density.compile("remesh_density");

    println!("cargo:rustc-cfg=has_instant_meshes");
    if quadriflow {
        println!("cargo:rustc-cfg=has_quadriflow");
    }
}

/// Compile vendored QuadriFlow plus the `remesh_quadriflow.cpp` that drives it,
/// returning whether it is present.
///
/// Called from [`build_instant_meshes`] rather than from `main`, because the two
/// share one `configure` (the same Eigen settings — a difference there would
/// give the halves incompatible types) and because the bridge that dispatches
/// between the engines is compiled there.
///
/// The vendored tree is Blender's arrangement of upstream QuadriFlow: Boost's
/// max-flow replaced by a bundled header, lemon bundled under `3rd/`, and every
/// TBB path behind a `WITH_TBB` that is deliberately never defined — so what is
/// compiled here is entirely serial, and reproducible for free.
fn build_quadriflow(configure: &impl Fn(&mut cc::Build)) -> bool {
    let dir = Path::new("../../third_party/quadriflow");
    let lemon = dir.join("3rd").join("lemon-1.3.1");
    // lemon picks its platform arm on `WIN32`, which upstream's CMake defines
    // and MSVC does not predefine (it predefines `_WIN32`). Without it
    // `time_measure.h`, `random.h` and `bits/windows.cc` take the POSIX arm and
    // reach for `unistd.h` / `sys/time.h`, which MSVC does not ship. Both
    // builds below see lemon's headers, so both need it.
    let lemon_platform = matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("windows")
    )
    .then_some("WIN32");
    let driver_cpp = Path::new("src/remesh_quadriflow.cpp");
    let driver_h = Path::new("src/remesh_quadriflow.h");

    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-changed={}", driver_cpp.display());
    println!("cargo:rerun-if-changed={}", driver_h.display());

    let sources_dir = dir.join("src");
    // `lemon/config.h` is generated by upstream's CMake and committed here
    // instead (see the NOTICE); without it lemon's headers do not compile, so it
    // is part of what "the tree is present" means.
    if !sources_dir.join("parametrizer.cpp").exists()
        || !lemon.join("lemon").join("config.h").exists()
    {
        return false;
    }
    let Some(mut sources) = cpp_sources(&sources_dir) else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    // The handful of lemon translation units upstream's own build compiles;
    // everything else it offers is header-only or a solver binding this build
    // has no backend for.
    for unit in [
        "arg_parser",
        "base",
        "color",
        "lp_base",
        "lp_skeleton",
        "random",
        "bits/windows",
    ] {
        let path = lemon.join("lemon").join(format!("{unit}.cc"));
        if !path.exists() {
            return false;
        }
        sources.push(path);
    }
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    // Driver first, vendored tree second, as everything else here splits: this
    // project's own C++ compiles with warnings **on**, and a linker that
    // resolves in command-line order wants the caller before the callee.
    let mut driver = cc::Build::new();
    configure(&mut driver);
    driver
        .file(driver_cpp)
        .include("src")
        .include(&sources_dir)
        .include(dir.join("3rd").join("pcg32"))
        .include(&lemon)
        .warnings(true);
    if let Some(define) = lemon_platform {
        driver.define(define, None);
    }
    // The two Eigen trips MSVC's /W4 on, suppressed by number so every other
    // warning in the driver still reaches the log.
    driver.flag_if_supported("/wd4100");
    driver.flag_if_supported("/wd4324");
    // QuadriFlow's own headers are full of `int i < vec.size()` and set-but-
    // unused locals, and the driver includes them — so these two classes are
    // suppressed by name rather than turning the driver's warnings off
    // wholesale, which would hide a real one in this project's code.
    driver.flag_if_supported("-Wno-sign-compare");
    driver.flag_if_supported("-Wno-unused-but-set-variable");
    driver.flag_if_supported("/wd4018");
    driver.flag_if_supported("/wd4189");
    driver.compile("remesh_quadriflow");

    let mut vendored = cc::Build::new();
    configure(&mut vendored);
    if let Some(define) = lemon_platform {
        vendored.define(define, None);
    }
    vendored
        .files(&sources)
        .include(&sources_dir)
        .include(dir.join("3rd").join("pcg32"))
        .include(&lemon)
        .warnings(false)
        .compile("quadriflow");

    true
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
