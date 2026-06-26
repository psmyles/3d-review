//! Offline IBL bake tool.
//!
//! Precomputes the env-cube / irradiance / prefilter / BRDF maps for every
//! built-in HDR environment and writes them to `assets/ibl_baked/` as raw
//! little-endian `f16`, so the shipping viewer loads them by upload instead of
//! running the 43-pass precompute at startup.
//!
//! Run with (needs a real GPU; dev-only, like the HDR thumbnail script):
//!
//! ```text
//! cargo run -p review-render --features bake --bin bake_ibl
//! ```
//!
//! Re-run whenever an `assets/textures/T_HDR_*.hdr` changes; the outputs are
//! committed. `packaging/generate-ibl-bake.ps1` wraps this.

fn main() {
    if let Err(err) = review_render::bake_ibl_assets() {
        eprintln!("IBL bake failed: {err}");
        std::process::exit(1);
    }
}
