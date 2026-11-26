use std::path::Path;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_ufbx)");

    let c_path = Path::new("../../third_party/ufbx/ufbx.c");
    let h_path = Path::new("../../third_party/ufbx/ufbx.h");
    let bridge_c_path = Path::new("src/ufbx_bridge.c");
    let bridge_h_path = Path::new("src/ufbx_bridge.h");
    let extras_c_path = Path::new("src/ufbx_extras.c");
    let extras_h_path = Path::new("src/ufbx_extras.h");
    let internal_h_path = Path::new("src/ufbx_internal.h");

    println!("cargo:rerun-if-changed={}", c_path.display());
    println!("cargo:rerun-if-changed={}", h_path.display());
    println!("cargo:rerun-if-changed={}", bridge_c_path.display());
    println!("cargo:rerun-if-changed={}", bridge_h_path.display());
    println!("cargo:rerun-if-changed={}", extras_c_path.display());
    println!("cargo:rerun-if-changed={}", extras_h_path.display());
    println!("cargo:rerun-if-changed={}", internal_h_path.display());

    if c_path.exists() && h_path.exists() {
        // Two builds, not one: the bridge is this project's own C and compiles
        // with warnings **on**, which they cannot be while the vendored
        // amalgamation shares the invocation. The bridge is emitted first so a
        // linker that resolves in command-line order sees it before the library
        // it calls into.
        exact_float(&mut cc::Build::new())
            .file(bridge_c_path)
            // The source-property capture is the bridge's second file: same
            // extraction call, same warnings-on build.
            .file(extras_c_path)
            .include("../../third_party/ufbx")
            .include("src")
            .warnings(true)
            .compile("ufbx_bridge");
        exact_float(&mut cc::Build::new())
            .file(c_path)
            .include("../../third_party/ufbx")
            .warnings(false)
            .compile("ufbx");
        println!("cargo:rustc-cfg=has_ufbx");
    }
}

/// Floating point that rounds the same on every compiler: no fusing of `a * b + c`
/// into one FMA (Clang's default on Apple Silicon, where GCC on x86-64 cannot),
/// and no reassociation. ufbx's transforms and tangent maths reach every vertex
/// the optimizer writes (`docs/ARCHITECTURE.md`, D27).
fn exact_float(build: &mut cc::Build) -> &mut cc::Build {
    if build.get_compiler().is_like_msvc() {
        // Since Visual Studio 2022, `/fp:precise` contracts nothing
        // unless `/fp:contract` asks it to.
        build.flag("/fp:precise")
    } else {
        build.flag("-ffp-contract=off").flag("-fno-fast-math")
    }
}
