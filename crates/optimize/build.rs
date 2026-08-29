use std::path::{Path, PathBuf};

/// Compile the vendored meshoptimizer sources, mirroring how
/// `crates/import/build.rs` gates vendored ufbx: the tree is *optional*, so its
/// absence is not a build failure — it just leaves `cfg(has_meshopt)` unset and
/// the crate's operations report `OptError::Unavailable`.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_meshopt)");

    let meshopt_dir = Path::new("../../third_party/meshoptimizer");
    let header = meshopt_dir.join("meshoptimizer.h");

    println!("cargo:rerun-if-changed={}", meshopt_dir.display());

    if !header.exists() {
        return;
    }

    let Some(sources) = cpp_sources(meshopt_dir) else {
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
        .include(meshopt_dir)
        .warnings(false)
        .compile("meshoptimizer");

    println!("cargo:rustc-cfg=has_meshopt");
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
