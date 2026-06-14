use std::path::Path;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_ufbx)");

    let c_path = Path::new("../../third_party/ufbx/ufbx.c");
    let h_path = Path::new("../../third_party/ufbx/ufbx.h");
    let bridge_c_path = Path::new("src/ufbx_bridge.c");
    let bridge_h_path = Path::new("src/ufbx_bridge.h");

    println!("cargo:rerun-if-changed={}", c_path.display());
    println!("cargo:rerun-if-changed={}", h_path.display());
    println!("cargo:rerun-if-changed={}", bridge_c_path.display());
    println!("cargo:rerun-if-changed={}", bridge_h_path.display());

    if c_path.exists() && h_path.exists() {
        cc::Build::new()
            .file(c_path)
            .file(bridge_c_path)
            .include("../../third_party/ufbx")
            .include("src")
            .warnings(false)
            .compile("ufbx");
        println!("cargo:rustc-cfg=has_ufbx");
    }
}
