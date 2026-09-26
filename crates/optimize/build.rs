use std::path::{Path, PathBuf};

/// Compile the vendored C/C++ libraries, mirroring how `crates/import/build.rs`
/// gates vendored ufbx: each tree is *optional*, so its absence is not a build
/// failure — it just leaves the matching `cfg` unset and the dependent
/// operations report the matching "unavailable" error.
///
/// They are separate `cc::Build` invocations because the trees disagree about
/// language and flags: meshoptimizer is exception-free C++ and ufbx_write is C.
/// One build cannot compile both correctly.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_meshopt)");
    println!("cargo:rustc-check-cfg=cfg(has_ufbxw)");
    println!("cargo:rustc-check-cfg=cfg(has_ufbxw_probe)");

    build_meshoptimizer();
    build_ufbx_write();
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

    check_meshopt_header(&header);

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

/// The meshoptimizer release `src/ffi.rs` was written against.
const MESHOPT_PINNED_VERSION: u32 = 1030;

/// Every declaration `src/ffi.rs` binds, as it appears in the pinned header
/// (the `MESHOPTIMIZER_API` / `MESHOPTIMIZER_EXPERIMENTAL` prefix dropped,
/// whitespace collapsed). Rust cannot see a C header, so a parameter list that
/// changes under a re-vendor would otherwise link cleanly and pass arguments in
/// the wrong slots. Experimental entry points (`meshopt_remesh`,
/// `meshopt_generateNormals`, the `PreserveFolds` / `ErrorClamped` /
/// `Remesh*` bits) are exactly the ones upstream reserves the right to change.
const MESHOPT_BOUND: &[&str] = &[
    "size_t meshopt_generateVertexRemap(unsigned int* destination, const unsigned int* indices, size_t index_count, const void* vertices, size_t vertex_count, size_t vertex_size);",
    "size_t meshopt_generateVertexRemapCustom(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, int (*callback)(void*, unsigned int, unsigned int), void* context);",
    "void meshopt_remapIndexBuffer(unsigned int* destination, const unsigned int* indices, size_t index_count, const unsigned int* remap);",
    "size_t meshopt_filterIndexBufferMulti(unsigned int* destination, const unsigned int* indices, size_t index_count, size_t vertex_count, const struct meshopt_Stream* streams, size_t stream_count);",
    "size_t meshopt_filterIndexBuffer(unsigned int* destination, const unsigned int* indices, size_t index_count, const void* vertices, size_t vertex_count, size_t vertex_size, size_t vertex_stride);",
    "void meshopt_optimizeVertexCache(unsigned int* destination, const unsigned int* indices, size_t index_count, size_t vertex_count);",
    "void meshopt_optimizeOverdraw(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, float threshold);",
    "size_t meshopt_optimizeVertexFetchRemap(unsigned int* destination, const unsigned int* indices, size_t index_count, size_t vertex_count);",
    "size_t meshopt_simplify(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, size_t target_index_count, float target_error, unsigned int options, float* result_error);",
    "size_t meshopt_simplifyWithAttributes(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, const float* vertex_attributes, size_t vertex_attributes_stride, const float* attribute_weights, size_t attribute_count, const unsigned char* vertex_lock, size_t target_index_count, float target_error, unsigned int options, float* result_error);",
    "size_t meshopt_simplifySloppy(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, const unsigned char* vertex_lock, size_t target_index_count, float target_error, float* result_error);",
    "size_t meshopt_simplifyPrune(unsigned int* destination, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, float target_error);",
    "float meshopt_simplifyScale(const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride);",
    "struct meshopt_VertexCacheStatistics meshopt_analyzeVertexCache(const unsigned int* indices, size_t index_count, size_t vertex_count, unsigned int cache_size, unsigned int warp_size, unsigned int primgroup_size);",
    "struct meshopt_VertexFetchStatistics meshopt_analyzeVertexFetch(const unsigned int* indices, size_t index_count, size_t vertex_count, size_t vertex_size);",
    "struct meshopt_OverdrawStatistics meshopt_analyzeOverdraw(const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride);",
    "void meshopt_generateTangents(float* result, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, const float* vertex_normals, size_t vertex_normals_stride, const float* vertex_uvs, size_t vertex_uvs_stride, unsigned int options);",
    "void meshopt_generateNormals(float* result, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, float crease_angle, float smoothing);",
    "size_t meshopt_remesh(float* destination, size_t max_triangle_count, const unsigned int* indices, size_t index_count, const float* vertex_positions, size_t vertex_count, size_t vertex_positions_stride, int resolution, unsigned int options);",
    "struct meshopt_Stream { const void* data; size_t size; size_t stride; };",
    "meshopt_SimplifyLockBorder = 1 << 0,",
    "meshopt_SimplifyErrorAbsolute = 1 << 2,",
    "meshopt_SimplifyPrune = 1 << 3,",
    "meshopt_SimplifyRegularize = 1 << 4,",
    "meshopt_SimplifyPermissive = 1 << 5,",
    "meshopt_SimplifyRegularizeLight = 1 << 6,",
    "meshopt_SimplifyPreserveFolds = 1 << 7,",
    "meshopt_SimplifyErrorClamped = 1 << 8,",
    "meshopt_TangentZeroFallback = 1 << 1,",
    "meshopt_RemeshShell = 1 << 0,",
    "meshopt_RemeshSolve = 1 << 1,",
];

/// Refuse to build against a meshoptimizer header `src/ffi.rs` was not written
/// for: a different release, or any bound declaration that no longer appears
/// verbatim. Fails loudly rather than letting an ABI mismatch link.
fn check_meshopt_header(header: &Path) {
    let text = std::fs::read_to_string(header)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", header.display()));

    let version = text
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("#define MESHOPTIMIZER_VERSION")?
                .split_whitespace()
                .next()?
                .parse::<u32>()
                .ok()
        })
        .unwrap_or_else(|| panic!("{} has no MESHOPTIMIZER_VERSION", header.display()));
    assert!(
        version == MESHOPT_PINNED_VERSION,
        "third_party/meshoptimizer is version {version}, but crates/optimize/src/ffi.rs \
         is written against {MESHOPT_PINNED_VERSION}: re-check every declaration there \
         (and MESHOPT_BOUND in build.rs) against the new header, then bump the pin"
    );

    // Collapse whitespace so the header's tabs and line breaks (the
    // `meshopt_Stream` struct body spans lines) compare equal to the list.
    let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let header_flat = collapse(&text);
    for declaration in MESHOPT_BOUND {
        assert!(
            header_flat.contains(&collapse(declaration)),
            "meshoptimizer.h no longer declares `{declaration}` — \
             crates/optimize/src/ffi.rs binds it and must be updated to match"
        );
    }
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
