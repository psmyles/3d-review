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
//! The offline precompute that *produces* these assets lives at the bottom of this
//! file behind the `bake` feature (D19), so it shares every size and format constant
//! with the loader rather than keeping a second copy that could drift. It compiles
//! only into the `bake_ibl` binary; the shipped viewer contains none of it.

use crate::EnvironmentMap;
#[cfg(feature = "bake")]
use crate::rhi::bake::{self, Baker, CubeTarget, Target2D};
use crate::rhi::{Bindings, Format, GpuResult, Texture};
#[cfg(feature = "bake")]
use crate::rhi::{Sampler, backend, shader};
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

// ---------------------------------------------------------------------------
// The offline precompute (`bake` feature only) — Platform decisions D19
// ---------------------------------------------------------------------------

/// The HDR colour format the precompute *renders* into and reads back:
/// `Rgba16Float`. The cube payloads are then BC6H-compressed offline, and the
/// runtime creates `Bc6hRgbUfloat` textures from them.
#[cfg(feature = "bake")]
const ENV_FORMAT: Format = Format::Rgba16F;
/// The BRDF LUT's render-target and readback format. It stays uncompressed — the
/// runtime uploads exactly these f16 bytes.
#[cfg(feature = "bake")]
const BRDF_FORMAT: Format = Format::Rg16F;
/// Bytes of one BC6H 4×4 block (128-bit), the unit the written payload is checked
/// against for a dead pass.
#[cfg(feature = "bake")]
const BC6H_BLOCK_BYTES: usize = 16;
/// Bytes per texel of `Rgba16Float` (4×f16), which sizes the readback before BC6H.
#[cfg(feature = "bake")]
const RGBA16F_BPP: u32 = 8;

/// Largest value representable as a finite `f16`. HDR suns routinely exceed it;
/// left unclamped, `f16::from_f32` rounds them to `inf`, which propagates as `NaN`
/// through the convolutions (the unbounded irradiance integral especially) and shows
/// up as black speckles on the model with a dead spot at the sun. Clamping every
/// channel here keeps the uploaded environment finite. (`f32::min(NaN, x)` returns
/// `x`, so a stray non-finite source texel is sanitized too.)
#[cfg(feature = "bake")]
const F16_MAX: f32 = 65504.0;

/// Per-face / per-pass uniform, mirroring the shader's `face_params` (invariant 11).
/// `forward/right/up` are the cube face basis; `params.x` is the prefilter roughness.
#[cfg(feature = "bake")]
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceUniform {
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    params: [f32; 4],
}

// Both assertions, per invariant 11: the literal, and the one that pins this to the
// *shader* rather than to a hand-typed number. A uniform upload is rejected only when
// it is larger than the block, so a field added on one side alone uploads happily and
// the shader reads every later field shifted — here that silently corrupts the
// committed maps instead of failing the bake.
#[cfg(feature = "bake")]
const _: () = assert!(size_of::<FaceUniform>() == 64);
#[cfg(feature = "bake")]
const _: () = assert!(size_of::<FaceUniform>() == size_of::<generated::FaceParams>());
#[cfg(feature = "bake")]
const _: () = assert!(size_of::<FaceUniform>() == size_of::<generated::FaceParamsFs>());
// ...and field by field, since equal totals do not mean equal offsets: these four
// `vec4`s are interchangeable by size, and swapping two would tilt every baked
// face rather than fail anything.
#[cfg(feature = "bake")]
crate::shaders::assert_same_layout!(FaceUniform => generated::FaceParams, {
    forward => forward,
    right => right,
    up => up,
    params => params,
});
#[cfg(feature = "bake")]
crate::shaders::assert_same_layout!(FaceUniform => generated::FaceParamsFs, {
    forward => forward,
    right => right,
    up => up,
    params => params,
});

/// World-space basis (forward, right, up) per cube face, in layer order
/// `+X −X +Y −Y +Z −Z`. A fullscreen-triangle clip position `(x, y)` maps to the
/// direction `forward + x*right + y*up`, matching `vs_ibl` and the cube sampling in
/// the scene shader.
#[cfg(feature = "bake")]
const FACE_BASES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]), // +X
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), // -X
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]), // +Y
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), // -Y
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),  // +Z
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
];

/// HDR source filename for an environment, read from disk by the bake tool. (The
/// shipped binary embeds only the baked maps, never these.)
#[cfg(feature = "bake")]
fn hdr_filename(environment: EnvironmentMap) -> &'static str {
    match environment {
        EnvironmentMap::Hdr01 => "T_HDR_01.hdr",
        EnvironmentMap::Hdr02 => "T_HDR_02.hdr",
        EnvironmentMap::Hdr03 => "T_HDR_03.hdr",
        EnvironmentMap::Hdr04 => "T_HDR_04.hdr",
        EnvironmentMap::Hdr05 => "T_HDR_05.hdr",
        EnvironmentMap::Hdr06 => "T_HDR_06.hdr",
    }
}

/// Build the per-face/pass uniform from a cube-face basis and a prefilter roughness
/// (0 for the passes that have none).
#[cfg(feature = "bake")]
fn face_uniform(basis: ([f32; 3], [f32; 3], [f32; 3]), roughness: f32) -> FaceUniform {
    let (forward, right, up) = basis;
    FaceUniform {
        forward: [forward[0], forward[1], forward[2], 0.0],
        right: [right[0], right[1], right[2], 0.0],
        up: [up[0], up[1], up[2], 0.0],
        params: [roughness, 0.0, 0.0, 0.0],
    }
}

/// Decode an environment's HDR from `assets/textures` and upload it as an
/// `Rgba16Float` equirect texture — the bake's one input. Paths resolve against the
/// crate manifest, so the tool runs from any working directory.
#[cfg(feature = "bake")]
fn load_equirect_from_file(
    environment: EnvironmentMap,
) -> Result<Texture, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/textures")
        .join(hdr_filename(environment));
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let image = image::load_from_memory(&bytes)?;
    let rgba = image.to_rgba32f();
    let (width, height) = rgba.dimensions();
    let halfs: Vec<u16> = rgba
        .iter()
        .map(|&c| half::f16::from_f32(c.min(F16_MAX)).to_bits())
        .collect();
    Ok(Texture::immutable_2d(
        bytemuck::cast_slice(&halfs),
        width,
        height,
        ENV_FORMAT,
        c"ibl equirect source",
    )?)
}

/// BC6H-compress one `size`×`size` face of `Rgba16Float` texels into the GPU-native
/// unsigned BC6H blocks the runtime uploads.
///
/// The encoder's **highest-quality** profile (`very_slow_settings`): this runs once
/// offline, so the extra bake time buys a sharper shipped map (see the settled
/// decisions in docs/ARCHITECTURE.md). Output size is identical across profiles —
/// BC6H is fixed-rate at 16 bytes per block —
/// only encode time and per-block accuracy change.
#[cfg(feature = "bake")]
fn compress_bc6h_face(size: u32, rgba_f16: &[u8]) -> Vec<u8> {
    let surface = intel_tex_2::RgbaSurface {
        width: size,
        height: size,
        stride: size * RGBA16F_BPP,
        data: rgba_f16,
    };
    intel_tex_2::bc6h::compress_blocks(&intel_tex_2::bc6h::very_slow_settings(), &surface)
}

/// Reject a baked buffer that is one value repeated — the signature of a pass that
/// rendered nothing.
///
/// This exists because the bake **overwrites committed assets in place**: a silent
/// failure does not merely produce a bad build, it destroys known-good maps that took
/// a GPU run to make. That is exactly what a D3D11 RTV/SRV hazard did to the
/// irradiance cubes once — every pass reported success and wrote a flat black cube.
/// sokol's closed passes make that particular hazard impossible now, but the check
/// costs nothing and catches the next one.
///
/// `chunk` is the unit one value occupies: 16 bytes for a BC6H block, 4 for an
/// `Rg16Float` texel. Note the deliberate limit — this catches *dead* output, not
/// merely wrong output. A pass that samples the wrong source still varies.
#[cfg(feature = "bake")]
fn reject_degenerate(
    label: &str,
    bytes: &[u8],
    chunk: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut chunks = bytes.chunks_exact(chunk);
    let Some(first) = chunks.next() else {
        return Err(format!("{label}: bake produced no data").into());
    };
    if chunks.len() > 0 && chunks.all(|other| other == first) {
        return Err(format!(
            "{label}: every {chunk}-byte unit of the {} byte output is identical - the pass \
             rendered nothing. Check the pass attachments before trusting this bake.",
            bytes.len()
        )
        .into());
    }
    Ok(())
}

/// Read a whole rendered cube (all mips × six faces) back, BC6H-compress each face,
/// and write it in mip-major, faces-contiguous order — the exact layout
/// [`Texture::cube_block_compressed`] re-reads at runtime.
#[cfg(feature = "bake")]
fn write_cube(
    cube: &CubeTarget,
    base_size: u32,
    mips: u32,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    for mip in 0..mips {
        let size = (base_size >> mip).max(1);
        for face in 0..6 {
            let raw = backend::read_image_subresource(
                cube.image(),
                mip,
                face,
                size,
                size,
                ENV_FORMAT,
                RGBA16F_BPP,
            )?;
            data.extend_from_slice(&compress_bc6h_face(size, &raw));
        }
    }
    // Validated before the write, so a bad bake leaves the previous good asset intact.
    reject_degenerate(
        &path.file_name().unwrap_or_default().to_string_lossy(),
        &data,
        BC6H_BLOCK_BYTES,
    )?;
    std::fs::write(path, &data)?;
    Ok(())
}

/// Offline IBL bake: precompute every environment's maps on a headless device and
/// write them to `assets/ibl_baked/`. Re-exported as `review_render::bake_ibl_assets`
/// for the `bake_ibl` binary. Needs a real GPU.
///
/// Each pass declares a *different* set of resources, and sokol validates a draw
/// against exactly what its shader declared — no more, no less — so the bindings and
/// uniforms below are per-pass rather than bound once and left. shdc strips what a
/// program does not read: `ibl_brdf` reads nothing but the vertex uniform block,
/// `ibl_equirect` and `ibl_irradiance` never read the *fragment* copy of it, and only
/// `ibl_prefilter` does (for the roughness).
#[cfg(feature = "bake")]
pub fn bake_ibl_assets() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ibl_baked");
    std::fs::create_dir_all(&out_dir)?;

    let baker = Baker::new()?;

    let equirect_pipeline = bake::pipeline(
        generated::ibl_equirect_shader_desc,
        shader::bytecode!("ibl_equirect"),
        ENV_FORMAT,
        c"ibl equirect",
    )?;
    let irradiance_pipeline = bake::pipeline(
        generated::ibl_irradiance_shader_desc,
        shader::bytecode!("ibl_irradiance"),
        ENV_FORMAT,
        c"ibl irradiance",
    )?;
    let prefilter_pipeline = bake::pipeline(
        generated::ibl_prefilter_shader_desc,
        shader::bytecode!("ibl_prefilter"),
        ENV_FORMAT,
        c"ibl prefilter",
    )?;
    let brdf_pipeline = bake::pipeline(
        generated::ibl_brdf_shader_desc,
        shader::bytecode!("ibl_brdf"),
        BRDF_FORMAT,
        c"ibl brdf",
    )?;

    let sampler = Sampler::linear_clamp()?;

    // Render targets, reused across environments (each is re-rendered per env). The
    // env cube is both a render target (the equirect pass) and a sampled source (the
    // two convolutions) — which under D3D11 was the hazard this bake had to route
    // around by hand, and under sokol is simply two views of one image.
    let env_cube = CubeTarget::new(ENV_CUBE_SIZE, 1, ENV_FORMAT, c"ibl env cube")?;
    let irradiance = CubeTarget::new(IRRADIANCE_SIZE, 1, ENV_FORMAT, c"ibl irradiance")?;
    let prefilter = CubeTarget::new(PREFILTER_SIZE, PREFILTER_MIPS, ENV_FORMAT, c"ibl prefilter")?;
    let brdf = Target2D::new(BRDF_SIZE, BRDF_SIZE, BRDF_FORMAT, c"ibl brdf")?;

    // The BRDF integration LUT is environment-independent — bake it once. It samples
    // nothing (it is pure math over its own fragment coordinate), so it binds
    // nothing: sokol rejects an *empty* bindings struct rather than ignoring one.
    let identity = face_uniform(FACE_BASES[0], 0.0);
    baker.draw_fullscreen(
        &brdf_pipeline,
        brdf.attachment(),
        BRDF_SIZE,
        None,
        &[(generated::UB_FACE_PARAMS, &identity)],
    );
    baker.flush();
    let brdf_bytes = backend::read_image_subresource(
        brdf.image(),
        0,
        0,
        BRDF_SIZE,
        BRDF_SIZE,
        BRDF_FORMAT,
        RG16F_BPP as u32,
    )?;
    reject_degenerate("T_IBL_BRDF.bin", &brdf_bytes, RG16F_BPP)?;
    std::fs::write(out_dir.join("T_IBL_BRDF.bin"), &brdf_bytes)?;

    for (index, environment) in EnvironmentMap::ALL.iter().enumerate() {
        let nn = format!("{:02}", index + 1);
        let equirect = load_equirect_from_file(*environment)?;

        // Equirect -> env cube (six faces).
        let mut equirect_bindings = Bindings::new();
        equirect_bindings.texture(generated::VIEW_SRC_EQUIRECT, &equirect);
        equirect_bindings.sampler(generated::SMP_SRC_SAMPLER, &sampler);
        for (face, basis) in FACE_BASES.iter().enumerate() {
            let uniform = face_uniform(*basis, 0.0);
            baker.draw_fullscreen(
                &equirect_pipeline,
                env_cube.attachment(face as u32, 0),
                ENV_CUBE_SIZE,
                Some(&equirect_bindings),
                &[(generated::UB_FACE_PARAMS, &uniform)],
            );
        }

        // Env cube -> diffuse irradiance (six faces) and prefiltered specular
        // (mip = roughness, six faces each). Both sample the cube just rendered;
        // nothing has to be unbound first, because its pass is closed.
        let mut cube_bindings = Bindings::new();
        cube_bindings.cube_target(generated::VIEW_SRC_CUBE, &env_cube);
        cube_bindings.sampler(generated::SMP_SRC_SAMPLER, &sampler);

        for (face, basis) in FACE_BASES.iter().enumerate() {
            let uniform = face_uniform(*basis, 0.0);
            baker.draw_fullscreen(
                &irradiance_pipeline,
                irradiance.attachment(face as u32, 0),
                IRRADIANCE_SIZE,
                Some(&cube_bindings),
                &[(generated::UB_FACE_PARAMS, &uniform)],
            );
        }

        for mip in 0..PREFILTER_MIPS {
            let roughness = mip as f32 / PREFILTER_MAX_LOD;
            let size = (PREFILTER_SIZE >> mip).max(1);
            for (face, basis) in FACE_BASES.iter().enumerate() {
                let uniform = face_uniform(*basis, roughness);
                baker.draw_fullscreen(
                    &prefilter_pipeline,
                    prefilter.attachment(face as u32, mip),
                    size,
                    Some(&cube_bindings),
                    // The only pass that reads the roughness in the fragment stage,
                    // so the only one that binds the fragment copy of the block.
                    &[
                        (generated::UB_FACE_PARAMS, &uniform),
                        (generated::UB_FACE_PARAMS_FS, &uniform),
                    ],
                );
            }
        }

        // Everything is drawn; nothing is in flight that a readback could race.
        baker.flush();

        write_cube(
            &env_cube,
            ENV_CUBE_SIZE,
            1,
            &out_dir.join(format!("T_IBL_{nn}_env.bin")),
        )?;
        write_cube(
            &irradiance,
            IRRADIANCE_SIZE,
            1,
            &out_dir.join(format!("T_IBL_{nn}_irradiance.bin")),
        )?;
        write_cube(
            &prefilter,
            PREFILTER_SIZE,
            PREFILTER_MIPS,
            &out_dir.join(format!("T_IBL_{nn}_prefilter.bin")),
        )?;
        println!(
            "baked {} -> assets/ibl_baked/T_IBL_{nn}_*.bin",
            environment.label()
        );
    }
    println!(
        "IBL bake complete: {} environments + shared BRDF LUT -> {}",
        EnvironmentMap::ALL.len(),
        out_dir.display()
    );
    Ok(())
}
