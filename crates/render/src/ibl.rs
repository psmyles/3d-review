//! Image-based lighting: the maps a metallic-roughness PBR shaded path samples.
//!
//! For each environment there are four: an environment **cubemap** (also the skybox
//! background), a diffuse **irradiance** cube (cosine-convolved), a **prefiltered
//! specular** cube whose mips hold increasing roughness, and the shared **BRDF
//! integration LUT** (the split-sum scale/bias).
//!
//! At runtime they are **loaded, not computed**, from the offline-baked assets in
//! `assets/ibl_baked/` by [`IblMaps::from_baked`] — a pure upload, so the first frame
//! and every environment switch are near-instant, and nothing is touched per frame
//! after that. The three HDR cubes ship GPU-native BC6H (`Bc6hRgbUfloat`); the LUT
//! stays `Rg16Float`.
//!
//! The `.bin` layout is mip-major with the six faces contiguous per mip, which is
//! **exactly** what sokol's `sg_image_data` wants — one range per mip covering all
//! six faces, in `+X −X +Y −Y +Z −Z` order. The D3D11 path had to re-index that into
//! `face * mips + mip` subresources; that step is gone.
//!
//! The offline precompute that *produced* these assets is still parked in
//! `src/port_pending/ibl.rs` and returns with `mac-port-plan.md` Phase 1 step 6.

use crate::EnvironmentMap;
use crate::rhi::{Bindings, Format, GpuResult, Texture};
use crate::shaders::generated;

/// Env cubemap face resolution. The sources are 1024×512 equirect, so 256² faces
/// resolve the environment without upsampling blur.
const ENV_CUBE_SIZE: u32 = 256;
/// Diffuse irradiance is very low frequency — a tiny cube is plenty.
const IRRADIANCE_SIZE: u32 = 32;
/// Prefiltered specular base face resolution + mip count (one roughness per mip).
const PREFILTER_SIZE: u32 = 128;
const PREFILTER_MIPS: u32 = 5;
/// BRDF integration LUT resolution.
const BRDF_SIZE: u32 = 512;

/// Bytes per texel of the BRDF LUT (`Rg16Float`, 2×f16), which is uploaded raw.
const RG16F_BPP: usize = 4;

/// The largest mip index of the prefiltered specular cube, handed to the scene
/// shader so it can map roughness → mip LOD. Kept here so the two stay in step.
pub(crate) const PREFILTER_MAX_LOD: f32 = (PREFILTER_MIPS - 1) as f32;

/// The uploaded image-based-lighting maps, bound for every scene draw at the view
/// slots the shader declares. Only the shaded path and the skybox sample them;
/// reloaded (a pure upload) when the chosen environment changes.
pub(crate) struct IblMaps {
    /// Which environment these were loaded from, so the scene only reloads on an
    /// actual change.
    pub(crate) environment: EnvironmentMap,
    irradiance: Texture,
    prefilter: Texture,
    brdf: Texture,
    env_cube: Texture,
}

impl IblMaps {
    /// Load the baked maps for `environment`: the three HDR cubes from their
    /// mip-major BC6H payloads, the shared BRDF LUT from raw f16.
    pub(crate) fn from_baked(environment: EnvironmentMap) -> GpuResult<Self> {
        Ok(Self {
            environment,
            irradiance: Texture::cube_block_compressed(
                IRRADIANCE_SIZE,
                1,
                Format::Bc6hUf16,
                baked_irradiance_bytes(environment),
                c"ibl irradiance",
            )?,
            prefilter: Texture::cube_block_compressed(
                PREFILTER_SIZE,
                PREFILTER_MIPS,
                Format::Bc6hUf16,
                baked_prefilter_bytes(environment),
                c"ibl prefilter",
            )?,
            env_cube: Texture::cube_block_compressed(
                ENV_CUBE_SIZE,
                1,
                Format::Bc6hUf16,
                baked_env_bytes(environment),
                c"ibl env",
            )?,
            brdf: Texture::immutable_2d(
                BAKED_BRDF_BYTES,
                BRDF_SIZE,
                BRDF_SIZE,
                Format::Rg16F,
                c"ibl brdf",
            )?,
        })
    }

    /// Bind the three maps the **shaded mesh** samples. Deliberately not the
    /// environment cube: `fs_main` never reads it, so shdc strips it from that
    /// program, and sokol rejects a draw that binds a view its shader did not
    /// declare.
    pub(crate) fn bind_shading(&self, bindings: &mut Bindings) {
        bindings.texture(generated::VIEW_IRRADIANCE_CUBE, &self.irradiance);
        bindings.texture(generated::VIEW_PREFILTER_CUBE, &self.prefilter);
        bindings.texture(generated::VIEW_BRDF_LUT, &self.brdf);
    }

    /// Bind the environment cube, which is all the **skybox** program declares —
    /// and, for the same reason as above, all it may be given.
    pub(crate) fn bind_env(&self, bindings: &mut Bindings) {
        bindings.texture(generated::VIEW_ENV_CUBE, &self.env_cube);
    }
}

// The BRDF LUT's payload has to cover the texture it is uploaded into; a truncated
// baked asset is a packaging bug, and `Texture::immutable_2d` would reject it at
// runtime, but this catches it at build time instead.
const _: () = assert!(
    BAKED_BRDF_BYTES.len() == BRDF_SIZE as usize * BRDF_SIZE as usize * RG16F_BPP,
    "the baked BRDF LUT is not 512x512 Rg16Float"
);

/// Expand one `fn baked_*_bytes(EnvironmentMap) -> &'static [u8]` accessor per
/// baked-map kind: a six-arm `include_bytes!` match over
/// `assets/ibl_baked/T_IBL_NN_<kind>.bin`, written once instead of three times.
macro_rules! baked_bytes_fn {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        fn $name(environment: EnvironmentMap) -> &'static [u8] {
            match environment {
                EnvironmentMap::Hdr01 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_01_", $kind, ".bin"))
                }
                EnvironmentMap::Hdr02 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_02_", $kind, ".bin"))
                }
                EnvironmentMap::Hdr03 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_03_", $kind, ".bin"))
                }
                EnvironmentMap::Hdr04 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_04_", $kind, ".bin"))
                }
                EnvironmentMap::Hdr05 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_05_", $kind, ".bin"))
                }
                EnvironmentMap::Hdr06 => {
                    include_bytes!(concat!("../../../assets/ibl_baked/T_IBL_06_", $kind, ".bin"))
                }
            }
        }
    };
}

baked_bytes_fn!(
    /// Baked environment cube bytes (`assets/ibl_baked/T_IBL_NN_env.bin`).
    baked_env_bytes,
    "env"
);
baked_bytes_fn!(
    /// Baked diffuse irradiance cube bytes (`assets/ibl_baked/T_IBL_NN_irradiance.bin`).
    baked_irradiance_bytes,
    "irradiance"
);
baked_bytes_fn!(
    /// Baked prefiltered specular cube bytes (`assets/ibl_baked/T_IBL_NN_prefilter.bin`).
    baked_prefilter_bytes,
    "prefilter"
);

/// Baked BRDF integration LUT bytes (environment-independent — one shared file).
const BAKED_BRDF_BYTES: &[u8] = include_bytes!("../../../assets/ibl_baked/T_IBL_BRDF.bin");
