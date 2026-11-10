//! The UV-checker bake's mip filter (`build/mips.rs`) is the run-time's, to the byte.
//!
//! The run-time builds every chain with fire-decode's builder (its `mips` feature, FD11).
//! The build script keeps a copy of the filter rather than compile fire-decode a second
//! time for the build host, so the checker's baked chain is only the chain the viewer
//! would build if the two agree exactly. This holds them to it: noise at odd and even
//! sizes in both modes, and the two checker PNGs themselves.

#[path = "../build/mips.rs"]
mod bake;

use fire_decode::PixelFormat;
use fire_decode::mips::MipOptions;

/// The chain the run-time builds: fire-decode's, capped at sokol's levels.
fn run_time(rgba: &[u8], width: u32, height: u32, srgb: bool) -> Vec<Vec<u8>> {
    let mut options = MipOptions::new(srgb);
    options.max_levels = sokol::gfx::MAX_MIPMAPS as u32;
    fire_decode::mips::chain(rgba, width, height, PixelFormat::Rgba8Unorm, &options)
}

/// A reproducible buffer of `len` bytes that changes in every texel.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}

#[test]
fn the_bakes_level_cap_is_sokols() {
    assert_eq!(bake::MAX_LEVELS as usize, sokol::gfx::MAX_MIPMAPS);
}

#[test]
fn the_bakes_filter_is_the_run_times() {
    let sizes = [
        (1, 1),
        (2, 1),
        (5, 3),
        (64, 64),
        (257, 129),
        (1000, 3),
        (3, 700),
        (512, 512),
    ];
    for (seed, (width, height)) in sizes.into_iter().enumerate() {
        let rgba = noise(width as usize * height as usize * 4, seed as u32 + 1);
        for srgb in [true, false] {
            assert_eq!(
                bake::levels_below_base(&rgba, width, height, srgb),
                run_time(&rgba, width, height, srgb),
                "{width}x{height}, srgb={srgb}"
            );
        }
    }
}

#[test]
fn the_checkers_bake_the_chain_the_viewer_would_build() {
    let textures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/textures");
    for name in ["T_UV_Checker_BW.png", "T_UV_Checker_CLR.png"] {
        let image = review_render::decode_image(&textures.join(name)).expect(name);
        assert_eq!(
            bake::levels_below_base(&image.rgba, image.width, image.height, true),
            run_time(&image.rgba, image.width, image.height, true),
            "{name}"
        );
    }
}
