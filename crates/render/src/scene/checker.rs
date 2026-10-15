//! The UV-checker material's texture.
//!
//! The two checkers ship ready for the GPU: the build script (`build/uv_checker.rs`)
//! turns each checker PNG into its full mip chain, BC7-encoded and deflated, so
//! loading one is an inflate and an upload — 2-3 ms and 5.6 MB of VRAM, against the
//! 21 ms and 22 MB that decoding the PNG and building an RGBA8 chain here cost.
//!
//! Even so, only the one the view has picked is ever on the GPU, and only while the
//! checker material is on (invariant 3): a session that never shows the checker
//! pays nothing for it.
//!
//! The chain is what keeps a tiled checker from collapsing into moiré where it
//! minifies, and the anisotropic material sampler it is drawn through
//! (`SceneGpu::mesh_bindings`) is what keeps it sharp at a grazing angle rather
//! than trading the moiré for blur.

use crate::rhi::{Format, GpuResult, Texture};
use crate::{ActiveMaterial, CheckerTexture, SceneDebugOptions};

use super::gpu::SceneGpu;

/// One checker as the build script baked it: the dimensions of its top level, how
/// many levels the chain has, and the chain's BC7 blocks — level after level, largest
/// first — deflated.
struct BakedChecker {
    width: u32,
    height: u32,
    levels: u32,
    /// The BC7 chain's length once inflated.
    len: usize,
    deflated: &'static [u8],
}

// `GREYSCALE` and `COLOR`, one `BakedChecker` per checker PNG.
include!(concat!(env!("OUT_DIR"), "/uv_checker.rs"));

/// One opaque white texel.
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// The checker slot's texture: the picked checker while the material shows it, a
/// placeholder texel otherwise.
pub(super) struct CheckerGpu {
    /// Bound in the checker's slot while no checker is loaded: `fs_main` declares
    /// the slot whether or not it samples it, and sokol validates that every
    /// declared view is bound.
    placeholder: Texture,
    /// The picked checker and which one it is, while the checker material is on.
    loaded: Option<(CheckerTexture, Texture)>,
}

impl CheckerGpu {
    pub(super) fn new() -> GpuResult<Self> {
        Ok(Self {
            placeholder: white_texel()?,
            loaded: None,
        })
    }

    /// Load the checker `debug` picks while its material is the active one, and
    /// drop it the moment it is not. A no-op while the resident one is still the
    /// one wanted.
    pub(super) fn sync(&mut self, debug: &SceneDebugOptions) -> GpuResult<()> {
        if debug.active_material != ActiveMaterial::UvChecker {
            self.loaded = None;
            return Ok(());
        }
        let wanted = debug.uv_checker_texture;
        if self
            .loaded
            .as_ref()
            .is_some_and(|(kind, _)| *kind == wanted)
        {
            return Ok(());
        }
        // Free the old one before unpacking its replacement, so two are never
        // resident at once.
        self.loaded = None;
        let baked = match wanted {
            CheckerTexture::Greyscale => &GREYSCALE,
            CheckerTexture::Color => &COLOR,
        };
        self.loaded = Some((wanted, upload(baked)?));
        Ok(())
    }

    /// What the checker slot binds this frame.
    pub(super) fn texture(&self) -> &Texture {
        self.loaded
            .as_ref()
            .map_or(&self.placeholder, |(_, texture)| texture)
    }
}

impl SceneGpu {
    /// Drop the UV checker (invariant 3) — from the viewports that never draw it
    /// (the UV and Tex views), since leaving the 3D view is not an event the
    /// renderer sees. The 3D frame's own sync loads it again on the way back.
    pub(crate) fn release_checker(&mut self) {
        self.checker.loaded = None;
    }
}

/// Unpack a baked checker and upload its chain as it stands. A payload that does not
/// unpack is a build bug — fall back to a white texel rather than failing the frame.
fn upload(baked: &BakedChecker) -> GpuResult<Texture> {
    match inflate(baked) {
        Ok(blocks) => Texture::block_compressed_2d(
            baked.width,
            baked.height,
            baked.levels,
            Format::Bc7Srgb,
            &blocks,
            c"uv checker",
        ),
        Err(error) => {
            // Degrading to flat white is deliberate, but not silently: a corrupt baked
            // checker is a build bug worth seeing in the log.
            log::warn!("baked UV checker failed to unpack: {error}");
            white_texel()
        }
    }
}

/// The chain's BC7 blocks, checked to be exactly as long as the bake said. The
/// limit is that length too, so a corrupt payload cannot inflate past it.
fn inflate(baked: &BakedChecker) -> Result<Vec<u8>, String> {
    let blocks = miniz_oxide::inflate::decompress_to_vec_with_limit(baked.deflated, baked.len)
        .map_err(|error| format!("{:?}", error.status))?;
    if blocks.len() != baked.len {
        return Err(format!(
            "{} bytes where the bake wrote {}",
            blocks.len(),
            baked.len
        ));
    }
    Ok(blocks)
}

fn white_texel() -> GpuResult<Texture> {
    Texture::immutable_2d(&WHITE, 1, 1, Format::Rgba8Srgb, c"uv checker")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the upload checks, without a GPU: each baked checker unpacks to the
    /// length it claims, that length is a whole BC7 chain for its dimensions, and the
    /// chain runs all the way down to one texel.
    #[test]
    fn every_baked_checker_unpacks_to_its_whole_chain() {
        for (name, baked) in [("greyscale", &GREYSCALE), ("colour", &COLOR)] {
            let blocks = inflate(baked).unwrap_or_else(|error| panic!("{name}: {error}"));
            let chain: usize = (0..baked.levels)
                .map(|level| {
                    let wide = (baked.width >> level).max(1).div_ceil(4) as usize;
                    let high = (baked.height >> level).max(1).div_ceil(4) as usize;
                    wide * high * 16
                })
                .sum();
            assert_eq!(blocks.len(), chain, "{name}");
            assert_eq!(
                baked.width.max(baked.height) >> (baked.levels - 1),
                1,
                "{name}: the chain stops short of one texel"
            );
        }
    }
}
