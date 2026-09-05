//! Image-based lighting: load an HDR environment and precompute the maps a
//! metallic-roughness PBR shaded path samples (CLAUDE.md render roadmap, Phase 3).
//!
//! From the chosen equirectangular HDR (`EnvironmentMap`) this builds, in a
//! one-time burst of GPU passes:
//!   * an environment **cubemap** (also used for the optional skybox background),
//!   * a diffuse **irradiance** cubemap (cosine-convolved),
//!   * a **prefiltered specular** cubemap whose mips hold increasing roughness,
//!   * a **BRDF integration LUT** (the split-sum scale/bias).
//!
//! At runtime these are **loaded** (not computed) from the offline-baked assets in
//! `assets/ibl_baked/` by [`IblD3d::from_baked`] — a pure Direct3D 11 upload, so the
//! first frame and every environment switch are near-instant, and never touched per
//! frame otherwise. The Direct3D 11 maps are bound to the scene's fixed pixel-shader
//! slots `t1..t4`. The precompute that produced those assets (and the `ibl.hlsl`
//! shaders) is compiled only into the offline `bake_ibl` tool (the `bake` feature)
//! via `bake_ibl_assets`, which runs the passes on a headless Direct3D 11 device.
//!
//! The three HDR cubes ship as GPU-native BC6H (`Bc6hRgbUfloat`); the shared BRDF
//! LUT stays `Rg16Float`. The device hard-requires `TEXTURE_COMPRESSION_BC` (set at
//! device creation), so IBL is always available on the desktop D3D11 11_0+ target.

use crate::EnvironmentMap;
#[cfg(feature = "bake")]
use crate::rhi::bake::{Baker, CubeTarget, Target2D};
#[cfg(feature = "bake")]
use crate::rhi::{BlendMode, DynamicConstantBuffer, Pipeline, PipelineDesc, Sampler};
use crate::rhi::{Format, Gpu, GpuResult, Texture};
#[cfg(feature = "bake")]
use bytemuck::{Pod, Zeroable};

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

/// The HDR color format the precompute *renders* into and reads back (bake only):
/// `Rgba16Float`. The baked cube payloads are then BC6H-compressed offline; the
/// runtime cube textures are `Bc6hRgbUfloat`, created by [`IblD3d::from_baked`].
#[cfg(feature = "bake")]
const ENV_FORMAT: Format = Format::Rgba16F;
/// The BRDF integration LUT format (`Rg16Float`): the precompute render target +
/// readback format (bake only); the runtime uploads the same f16 bytes raw.
#[cfg(feature = "bake")]
const BRDF_FORMAT: Format = Format::Rg16F;

/// Bytes of one BC6H 4×4 block (128-bit) — the granularity the bake's written
/// payload is validated against. The runtime upload no longer needs it: the block
/// size now comes from the `Format` itself inside `Texture::cube_block_compressed`,
/// so the two can't disagree.
#[cfg(feature = "bake")]
const BC6H_BLOCK_BYTES: u32 = 16;

/// Bytes per texel of the *uncompressed* maps. `Rgba16Float` (4×f16) sizes the
/// bake readback before BC6H compression; `Rg16Float` (2×f16) is the BRDF LUT,
/// which stays uncompressed and is uploaded raw by `from_baked`.
#[cfg(feature = "bake")]
const RGBA16F_BPP: u32 = 8;
const RG16F_BPP: u32 = 4;

/// The largest mip index of the prefiltered specular cube, exposed to the scene
/// shader so it can map roughness → mip LOD. Kept here so the two stay in step.
pub(crate) const PREFILTER_MAX_LOD: f32 = (PREFILTER_MIPS - 1) as f32;

/// Per-face / per-pass uniform fed to the precompute shaders (matches the HLSL
/// `FaceUniform`, invariant 11). `forward/right/up` are the cube face basis;
/// `params.x` is the prefilter roughness.
#[cfg(feature = "bake")]
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FaceUniform {
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    params: [f32; 4],
}

// Byte-size lock against `ibl.hlsl`'s `b0`. The cbuffer is sized from
// `size_of::<T>()` and an upload is rejected only when it is *larger* than the
// buffer, so a field added on one side alone grows both and uploads happily while
// the shader keeps reading the old offsets — here that silently corrupts the
// committed maps rather than failing the bake.
#[cfg(feature = "bake")]
const _: () = assert!(std::mem::size_of::<FaceUniform>() == 64);
#[cfg(feature = "bake")]
const _: () = assert!(
    std::mem::size_of::<FaceUniform>()
        == std::mem::size_of::<crate::shaders::generated::FaceParams>()
);

/// World-space basis (forward, right, up) for each cubemap face, in layer order
/// `+X, -X, +Y, -Y, +Z, -Z`. A fullscreen-triangle clip position `(x, y)` maps to
/// the direction `forward + x*right + y*up`, matching `vs_fullscreen` in the
/// shader and the cube sampling in `scene.hlsl`.
#[cfg(feature = "bake")]
const FACE_BASES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]), // +X
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), // -X
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]), // +Y
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), // -Y
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),  // +Z
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
];

/// The Direct3D 11 image-based-lighting maps: the baked BC6H cubes (irradiance /
/// prefilter / env) + the raw `Rg16Float` BRDF LUT, each as a sampled [`Texture`].
/// Bound for every scene draw at the fixed PS slots `t1..t4`; only the shaded path +
/// the skybox sample them. Reloaded (a pure upload) when the chosen environment
/// changes.
pub(crate) struct IblD3d {
    /// Which environment these maps were loaded from (so the scene only reloads on
    /// an actual change).
    pub(crate) environment: EnvironmentMap,
    irradiance: Texture,
    prefilter: Texture,
    brdf: Texture,
    env_cube: Texture,
}

impl IblD3d {
    /// Load the baked maps for `environment` as D3D11 textures — the three HDR cubes
    /// from their mip-major BC6H `.bin` payloads, the shared BRDF LUT from raw f16.
    /// A pure upload (no precompute), so the first frame and every environment switch
    /// are near-instant.
    pub(crate) fn from_baked(gpu: &Gpu, environment: EnvironmentMap) -> GpuResult<Self> {
        let irradiance = Texture::cube_block_compressed(
            gpu,
            IRRADIANCE_SIZE,
            1,
            Format::Bc6hUf16,
            baked_irradiance_bytes(environment),
        )?;
        let prefilter = Texture::cube_block_compressed(
            gpu,
            PREFILTER_SIZE,
            PREFILTER_MIPS,
            Format::Bc6hUf16,
            baked_prefilter_bytes(environment),
        )?;
        let env_cube = Texture::cube_block_compressed(
            gpu,
            ENV_CUBE_SIZE,
            1,
            Format::Bc6hUf16,
            baked_env_bytes(environment),
        )?;
        let brdf = Texture::immutable_2d(
            gpu,
            BRDF_SIZE,
            BRDF_SIZE,
            Format::Rg16F,
            BRDF_SIZE * RG16F_BPP,
            BAKED_BRDF_BYTES,
        )?;
        Ok(Self {
            environment,
            irradiance,
            prefilter,
            brdf,
            env_cube,
        })
    }

    /// Bind the IBL maps to the scene pixel-shader slots `irradiance t1`,
    /// `prefilter t2`, `brdf t3`, `env t4` (see the register plan in `scene.hlsl`).
    pub(crate) fn bind_ps(&self, gpu: &Gpu) {
        self.irradiance.bind_ps(gpu, 1);
        self.prefilter.bind_ps(gpu, 2);
        self.brdf.bind_ps(gpu, 3);
        self.env_cube.bind_ps(gpu, 4);
    }
}

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

// --- Offline IBL bake (only compiled with the `bake` feature) -------------------
//
// Precomputes every environment's IBL maps on a headless Direct3D 11 device and
// writes them to `assets/ibl_baked/` (BC6H cubes + raw-f16 BRDF LUT), so the
// shipping binary loads them by upload (`IblD3d::from_baked`) instead of running
// the precompute at startup. Run via the `bake_ibl` binary; outputs are committed.
// All D3D11 plumbing lives in `rhi::bake`; the code below only drives the passes.

/// Compiled IBL precompute DXBC (see `build.rs`). Bake-only: the runtime samples
/// the baked maps and never creates these pipelines.
#[cfg(feature = "bake")]
const IBL_VS: &[u8] = include_bytes!("hlsl/ibl.vs.dxbc");
#[cfg(feature = "bake")]
const IBL_EQUIRECT_PS: &[u8] = include_bytes!("hlsl/ibl.equirect.ps.dxbc");
#[cfg(feature = "bake")]
const IBL_IRRADIANCE_PS: &[u8] = include_bytes!("hlsl/ibl.irradiance.ps.dxbc");
#[cfg(feature = "bake")]
const IBL_PREFILTER_PS: &[u8] = include_bytes!("hlsl/ibl.prefilter.ps.dxbc");
#[cfg(feature = "bake")]
const IBL_BRDF_PS: &[u8] = include_bytes!("hlsl/ibl.brdf.ps.dxbc");

/// Largest value representable as a finite `f16`. HDR suns routinely exceed this;
/// left unclamped, `f16::from_f32` rounds them to `inf`, which then propagates as
/// `NaN` through the IBL convolutions (the unclamped irradiance integral
/// especially) and the raw skybox sample — blowing out into black speckles on the
/// model and a dead spot at the sun. Clamp every channel to this ceiling so the
/// uploaded environment stays finite. (`f32::min(NaN, x)` returns `x`, so this
/// also sanitizes a stray non-finite source texel.) Bake-time only now — the
/// runtime clamps live in `ibl.hlsl` and the maps are already finite once baked.
#[cfg(feature = "bake")]
const F16_MAX: f32 = 65504.0;

/// HDR source filename for an environment, read from disk by the bake tool. (The
/// runtime no longer embeds the raw HDRs — only the baked maps.)
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

/// Build the per-face/pass [`FaceUniform`] from a cube-face basis + prefilter
/// roughness (0 for the non-prefilter passes).
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

/// Decode `environment`'s HDR from `assets/textures` on disk and upload it as an
/// `Rgba16Float` equirect [`Texture`] — the bake tool's input. Paths resolve
/// relative to the render crate's manifest so the tool runs from any working
/// directory.
#[cfg(feature = "bake")]
fn load_equirect_from_file(
    baker: &Baker,
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
    let texture = Texture::immutable_2d(
        baker.gpu(),
        width,
        height,
        ENV_FORMAT,
        width * RGBA16F_BPP,
        bytemuck::cast_slice(&halfs),
    )?;
    Ok(texture)
}

/// BC6H-compress one `size`×`size` face of `Rgba16Float` (FP16) texels into the
/// GPU-native unsigned BC6H blocks the runtime uploads. Bake-only: the encoder
/// (Intel ISPC, prebuilt kernels) trades offline bake time for an ~8× smaller,
/// sample-bandwidth-cheaper runtime map.
///
/// Uses the encoder's **highest-quality** profile (`very_slow_settings`): this
/// runs once offline, so we spend the extra bake time to make the shipped
/// runtime map as accurate as BC6H allows (project rule — see CLAUDE.md §4).
/// Output size is identical across profiles (BC6H is fixed-rate, 16 B/block);
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

/// Reject a baked buffer that is a single value repeated — the signature of a pass
/// that rendered nothing (a null SRV, an unbound target, a cleared-but-never-drawn
/// surface).
///
/// This exists because the bake **overwrites committed assets in place**: a silent
/// failure here doesn't just produce a bad build, it destroys known-good maps that
/// took a GPU run to make. That is exactly what a D3D11 RTV/SRV hazard did to the
/// irradiance cubes once — every pass reported success and wrote a flat black cube.
/// Failing loudly costs nothing and makes that class of bug impossible to miss.
///
/// `chunk` is the unit a single value occupies: 16 bytes for a BC6H block, 4 for an
/// `Rg16Float` texel. A buffer of one chunk or less can't be judged and passes.
///
/// Note the deliberate limit: this catches *dead* output, not merely wrong output.
/// A pass that samples the wrong source still produces varied bytes and gets through.
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
            concat!(
                "{}: every {}-byte unit of the {} byte output is identical - the pass ",
                "rendered nothing. Check for an RTV/SRV conflict (a source still bound as ",
                "a render target binds as a null SRV) before trusting this bake."
            ),
            label,
            chunk,
            bytes.len()
        )
        .into());
    }
    Ok(())
}

/// Read a whole rendered cube (all mips × 6 faces) back, BC6H-compress each
/// subresource, and write it to `path` in mip-major, faces-contiguous order — the
/// exact byte layout `Texture::cube_block_compressed` re-reads at runtime.
#[cfg(feature = "bake")]
fn write_cube(
    baker: &Baker,
    cube: &CubeTarget,
    base_size: u32,
    mips: u32,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    for mip in 0..mips {
        let size = (base_size >> mip).max(1);
        for face in 0..6 {
            // D3D11 cube subresource index: mip + face * mip_count.
            let subresource = mip + face * mips;
            let raw = baker.readback_subresource(
                cube.texture(),
                subresource,
                size,
                size,
                ENV_FORMAT,
                RGBA16F_BPP,
            )?;
            data.extend_from_slice(&compress_bc6h_face(size, &raw));
        }
    }
    // Validate before the write, so a bad bake leaves the previous good asset intact.
    reject_degenerate(
        &path.file_name().unwrap_or_default().to_string_lossy(),
        &data,
        BC6H_BLOCK_BYTES as usize,
    )?;
    std::fs::write(path, &data)?;
    Ok(())
}

/// Offline IBL bake entry point: precompute every environment's maps on a headless
/// Direct3D 11 device and write them to `assets/ibl_baked/`. Re-exported as
/// `review_render::bake_ibl_assets` for the `bake_ibl` binary. Needs a real GPU.
#[cfg(feature = "bake")]
pub fn bake_ibl_assets() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ibl_baked");
    std::fs::create_dir_all(&out_dir)?;

    let baker = Baker::new()?;
    // Every bake resource is built from the headless device the baker owns, exactly
    // as the runtime builds its own from the windowed one.
    let gpu = baker.gpu();

    // Shared bake resources: the per-face/pass uniform (`b0`), the linear-clamp
    // sampler (`s0`), and the four fullscreen pipelines (all share `IBL_VS`,
    // depth-less opaque overwrite — they cover the whole target).
    let cbuffer = DynamicConstantBuffer::new::<FaceUniform>(gpu)?;
    let sampler = Sampler::linear_clamp(gpu)?;
    let fullscreen = |ps: &[u8]| -> GpuResult<Pipeline> {
        Pipeline::new(
            gpu,
            &PipelineDesc::fullscreen(IBL_VS, ps, BlendMode::Opaque),
        )
    };
    let equirect_pipeline = fullscreen(IBL_EQUIRECT_PS)?;
    let irradiance_pipeline = fullscreen(IBL_IRRADIANCE_PS)?;
    let prefilter_pipeline = fullscreen(IBL_PREFILTER_PS)?;
    let brdf_pipeline = fullscreen(IBL_BRDF_PS)?;

    // Render targets, reused across environments (each is re-rendered per env). The
    // env cube is both a render target (equirect pass) and a sampled source (the
    // irradiance / prefilter convolutions).
    let env_cube = CubeTarget::new(gpu, ENV_CUBE_SIZE, 1, ENV_FORMAT)?;
    let irradiance = CubeTarget::new(gpu, IRRADIANCE_SIZE, 1, ENV_FORMAT)?;
    let prefilter = CubeTarget::new(gpu, PREFILTER_SIZE, PREFILTER_MIPS, ENV_FORMAT)?;
    let brdf = Target2D::new(gpu, BRDF_SIZE, BRDF_SIZE, BRDF_FORMAT)?;

    // The uniform + sampler stay bound throughout (only the source texture + RTV
    // change per pass); `cbuffer.update` re-fills `b0` per face/mip.
    cbuffer.bind_vs(gpu, 0);
    cbuffer.bind_ps(gpu, 0);
    sampler.bind_ps(gpu, 0);

    // The BRDF integration LUT is environment-independent — bake it once.
    brdf_pipeline.bind(gpu);
    cbuffer.update(gpu, &face_uniform(FACE_BASES[0], 0.0))?;
    baker.begin_target(brdf.rtv(), BRDF_SIZE);
    baker.draw_fullscreen();
    baker.unbind_targets();
    let brdf_bytes = baker.readback_subresource(
        brdf.texture(),
        0,
        BRDF_SIZE,
        BRDF_SIZE,
        BRDF_FORMAT,
        RG16F_BPP,
    )?;
    reject_degenerate("T_IBL_BRDF.bin", &brdf_bytes, RG16F_BPP as usize)?;
    std::fs::write(out_dir.join("T_IBL_BRDF.bin"), &brdf_bytes)?;

    for (index, environment) in EnvironmentMap::ALL.iter().enumerate() {
        let nn = format!("{:02}", index + 1);
        let equirect = load_equirect_from_file(&baker, *environment)?;

        // Equirect -> env cube (6 faces).
        equirect_pipeline.bind(gpu);
        equirect.bind_ps(gpu, 0);
        for (face, basis) in FACE_BASES.iter().enumerate() {
            cbuffer.update(gpu, &face_uniform(*basis, 0.0))?;
            baker.begin_target(env_cube.rtv(face as u32, 0), ENV_CUBE_SIZE);
            baker.draw_fullscreen();
        }

        // Env cube -> diffuse irradiance (6 faces).
        //
        // The env cube's last face is still bound as a *render target* from the loop
        // above, and D3D11 refuses to bind a resource as an SRV while it is writable:
        // it resolves the hazard by silently nulling the SRV, so the convolution would
        // sample nothing and bake a flat black cube. Release the render target first.
        // (The prefilter pass below is safe only by accident — by then the irradiance
        // target has displaced the env cube from the RTV slot.)
        baker.unbind_targets();
        irradiance_pipeline.bind(gpu);
        env_cube.bind_ps_srv(gpu, 1);
        for (face, basis) in FACE_BASES.iter().enumerate() {
            cbuffer.update(gpu, &face_uniform(*basis, 0.0))?;
            baker.begin_target(irradiance.rtv(face as u32, 0), IRRADIANCE_SIZE);
            baker.draw_fullscreen();
        }

        // Env cube -> prefiltered specular (mip = roughness, 6 faces each).
        prefilter_pipeline.bind(gpu);
        env_cube.bind_ps_srv(gpu, 1);
        for mip in 0..PREFILTER_MIPS {
            let roughness = if PREFILTER_MIPS > 1 {
                mip as f32 / PREFILTER_MAX_LOD
            } else {
                0.0
            };
            let size = (PREFILTER_SIZE >> mip).max(1);
            for (face, basis) in FACE_BASES.iter().enumerate() {
                cbuffer.update(gpu, &face_uniform(*basis, roughness))?;
                baker.begin_target(prefilter.rtv(face as u32, mip), size);
                baker.draw_fullscreen();
            }
        }

        // Release the env-cube SRV + render targets before reading back (and before
        // the next environment re-renders the env cube into it).
        baker.unbind_srvs();
        baker.unbind_targets();

        write_cube(
            &baker,
            &env_cube,
            ENV_CUBE_SIZE,
            1,
            &out_dir.join(format!("T_IBL_{nn}_env.bin")),
        )?;
        write_cube(
            &baker,
            &irradiance,
            IRRADIANCE_SIZE,
            1,
            &out_dir.join(format!("T_IBL_{nn}_irradiance.bin")),
        )?;
        write_cube(
            &baker,
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
