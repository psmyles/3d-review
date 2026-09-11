use std::path::Path;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_ufbx)");

    let c_path = Path::new("../../third_party/ufbx/ufbx.c");
    let h_path = Path::new("../../third_party/ufbx/ufbx.h");
    let bridge_c_path = Path::new("src/ufbx_bridge.c");
    let bridge_h_path = Path::new("src/ufbx_bridge.h");
    let extras_c_path = Path::new("src/ufbx_extras.c");
    let extras_h_path = Path::new("src/ufbx_extras.h");

    println!("cargo:rerun-if-changed={}", c_path.display());
    println!("cargo:rerun-if-changed={}", h_path.display());
    println!("cargo:rerun-if-changed={}", bridge_c_path.display());
    println!("cargo:rerun-if-changed={}", bridge_h_path.display());
    println!("cargo:rerun-if-changed={}", extras_c_path.display());
    println!("cargo:rerun-if-changed={}", extras_h_path.display());

    if c_path.exists() && h_path.exists() {
        // Two builds, not one: the bridge is this project's own C and compiles
        // with warnings **on**, which they cannot be while the vendored
        // amalgamation shares the invocation. The bridge is emitted first so a
        // linker that resolves in command-line order sees it before the library
        // it calls into.
        cc::Build::new()
            .file(bridge_c_path)
            // The source-property capture is the bridge's second file: same
            // extraction call, same warnings-on build.
            .file(extras_c_path)
            .include("../../third_party/ufbx")
            .include("src")
            .warnings(true)
            .compile("ufbx_bridge");
        cc::Build::new()
            .file(c_path)
            .include("../../third_party/ufbx")
            .warnings(false)
            .compile("ufbx");
        println!("cargo:rustc-cfg=has_ufbx");
    }
}
