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
//! slots `t1..t4`. The precompute that produced those assets (and the `ibl.wgsl`
//! shaders) is compiled only into the offline `bake_ibl` tool (the `bake` feature)
//! via `bake_ibl_assets`.
//!
//! The three HDR cubes ship as GPU-native BC6H (`Bc6hRgbUfloat`); the shared BRDF
//! LUT stays `Rg16Float`. The device hard-requires `TEXTURE_COMPRESSION_BC` (set at
//! device creation), so IBL is always available on the desktop D3D11 11_0+ target.

#[cfg(feature = "bake")]
use bytemuck::{Pod, Zeroable};
#[cfg(feature = "bake")]
use wgpu::util::DeviceExt;
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_BC6H_UF16, DXGI_FORMAT_R16G16_FLOAT};

use crate::EnvironmentMap;
use crate::rhi::{Gpu, Texture};

/// The IBL precompute WGSL. Only compiled into the bake path (and the WGSL
/// validation test); the runtime samples the baked maps and never builds these
/// pipelines.
#[cfg(any(feature = "bake", test))]
const IBL_SHADER: &str = include_str!("ibl.wgsl");

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

/// The HDR color format the precompute *renders* into and reads back (bake only).
/// The baked cube payloads are then BC6H-compressed offline; the runtime cube
/// textures are `Bc6hRgbUfloat`, created by [`IblD3d::from_baked`].
#[cfg(feature = "bake")]
const ENV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The BRDF integration LUT format (`Rg16Float`), the precompute render target +
/// readback format (bake only); the runtime uploads the same f16 bytes.
#[cfg(feature = "bake")]
const BRDF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// Bytes of one BC6H 4×4 block (128-bit). The runtime cube upload strides the
/// baked payload by whole rows of blocks.
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

/// Per-face / per-pass uniform fed to the precompute shaders (matches the WGSL
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

/// World-space basis (forward, right, up) for each cubemap face, in layer order
/// `+X, -X, +Y, -Y, +Z, -Z`. A fullscreen-triangle clip position `(x, y)` maps to
/// the direction `forward + x*right + y*up`, matching `vs_fullscreen` in the
/// shader and the cube sampling in `scene.wgsl`.
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
    pub(crate) fn from_baked(
        gpu: &Gpu,
        environment: EnvironmentMap,
    ) -> windows::core::Result<Self> {
        let device = gpu.device();
        let irradiance = Texture::cube_block_compressed(
            device,
            IRRADIANCE_SIZE,
            1,
            DXGI_FORMAT_BC6H_UF16,
            BC6H_BLOCK_BYTES,
            baked_irradiance_bytes(environment),
        )?;
        let prefilter = Texture::cube_block_compressed(
            device,
            PREFILTER_SIZE,
            PREFILTER_MIPS,
            DXGI_FORMAT_BC6H_UF16,
            BC6H_BLOCK_BYTES,
            baked_prefilter_bytes(environment),
        )?;
        let env_cube = Texture::cube_block_compressed(
            device,
            ENV_CUBE_SIZE,
            1,
            DXGI_FORMAT_BC6H_UF16,
            BC6H_BLOCK_BYTES,
            baked_env_bytes(environment),
        )?;
        let brdf = Texture::immutable_2d(
            device,
            BRDF_SIZE,
            BRDF_SIZE,
            DXGI_FORMAT_R16G16_FLOAT,
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
    pub(crate) fn bind_ps(&self, ctx: &ID3D11DeviceContext) {
        self.irradiance.bind_ps(ctx, 1);
        self.prefilter.bind_ps(ctx, 2);
        self.brdf.bind_ps(ctx, 3);
        self.env_cube.bind_ps(ctx, 4);
    }
}

/// The four GPU textures the IBL precompute produces, before the bake tool reads
/// them back to disk ([`bake_ibl_assets`]).
#[cfg(feature = "bake")]
struct PrecomputedMaps {
    env_cube: wgpu::Texture,
    irradiance: wgpu::Texture,
    prefilter: wgpu::Texture,
    brdf: wgpu::Texture,
}

/// Run the IBL precompute passes from a decoded equirect environment and return
/// the four resulting textures. The textures carry `COPY_SRC` so the bake tool
/// can copy them out. Only the offline bake path runs this; the runtime loads the
/// baked results via [`IblD3d::from_baked`].
#[cfg(feature = "bake")]
fn precompute_maps(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    equirect_view: &wgpu::TextureView,
) -> PrecomputedMaps {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("review_ibl_shader"),
        source: wgpu::ShaderSource::Wgsl(IBL_SHADER.into()),
    });

    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("review_ibl_face_uniform"),
        size: std::mem::size_of::<FaceUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let sampler = ibl_sampler(device);

    // The precompute targets are rendered into, then read back by the bake tool.
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    let env_cube = create_cube_texture(
        device,
        "review_ibl_env_cube",
        ENV_CUBE_SIZE,
        1,
        ENV_FORMAT,
        usage,
    );
    let irradiance = create_cube_texture(
        device,
        "review_ibl_irradiance",
        IRRADIANCE_SIZE,
        1,
        ENV_FORMAT,
        usage,
    );
    let prefilter = create_cube_texture(
        device,
        "review_ibl_prefilter",
        PREFILTER_SIZE,
        PREFILTER_MIPS,
        ENV_FORMAT,
        usage,
    );

    // --- Pass layouts + pipelines (precompute-only; dropped after this fn). ---
    let equirect_layout = pass_layout(device, "review_ibl_equirect_layout", PassSource::Tex2d);
    let cube_layout = pass_layout(device, "review_ibl_cube_layout", PassSource::Cube);
    let brdf_layout = pass_layout(device, "review_ibl_brdf_layout", PassSource::None);

    let equirect_pipeline = make_pipeline(
        device,
        &shader,
        &equirect_layout,
        "fs_equirect_to_cube",
        ENV_FORMAT,
        "review_ibl_equirect_pipeline",
    );
    let irradiance_pipeline = make_pipeline(
        device,
        &shader,
        &cube_layout,
        "fs_irradiance",
        ENV_FORMAT,
        "review_ibl_irradiance_pipeline",
    );
    let prefilter_pipeline = make_pipeline(
        device,
        &shader,
        &cube_layout,
        "fs_prefilter",
        ENV_FORMAT,
        "review_ibl_prefilter_pipeline",
    );
    let brdf_pipeline = make_pipeline(
        device,
        &shader,
        &brdf_layout,
        "fs_brdf",
        BRDF_FORMAT,
        "review_ibl_brdf_pipeline",
    );

    // --- Pass bind groups (the uniform contents change per face/mip, but the
    //     bindings don't, so each is built once). ---
    let equirect_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("review_ibl_equirect_bind"),
        layout: &equirect_layout,
        entries: &[
            uniform_entry(0, &uniform_buffer),
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(equirect_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let env_cube_sample_view = env_cube.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let cube_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("review_ibl_cube_bind"),
        layout: &cube_layout,
        entries: &[
            uniform_entry(0, &uniform_buffer),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&env_cube_sample_view),
            },
        ],
    });
    let brdf_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("review_ibl_brdf_bind"),
        layout: &brdf_layout,
        entries: &[uniform_entry(0, &uniform_buffer)],
    });

    // --- Equirect -> env cube (6 faces). ---
    for (face, basis) in FACE_BASES.iter().enumerate() {
        write_face_uniform(queue, &uniform_buffer, *basis, 0.0);
        let view = cube_face_view(&env_cube, face as u32, 0);
        run_pass(device, queue, &equirect_pipeline, &equirect_bind, &view);
    }

    // --- Env cube -> diffuse irradiance (6 faces). ---
    for (face, basis) in FACE_BASES.iter().enumerate() {
        write_face_uniform(queue, &uniform_buffer, *basis, 0.0);
        let view = cube_face_view(&irradiance, face as u32, 0);
        run_pass(device, queue, &irradiance_pipeline, &cube_bind, &view);
    }

    // --- Env cube -> prefiltered specular (mip = roughness, 6 faces each). ---
    for mip in 0..PREFILTER_MIPS {
        let roughness = if PREFILTER_MIPS > 1 {
            mip as f32 / PREFILTER_MAX_LOD
        } else {
            0.0
        };
        for (face, basis) in FACE_BASES.iter().enumerate() {
            write_face_uniform(queue, &uniform_buffer, *basis, roughness);
            let view = cube_face_view(&prefilter, face as u32, mip);
            run_pass(device, queue, &prefilter_pipeline, &cube_bind, &view);
        }
    }

    // --- BRDF integration LUT (single 2D pass). ---
    let brdf = create_brdf_texture(device, usage);
    let brdf_target = brdf.create_view(&wgpu::TextureViewDescriptor::default());
    write_face_uniform(queue, &uniform_buffer, FACE_BASES[0], 0.0);
    run_pass(device, queue, &brdf_pipeline, &brdf_bind, &brdf_target);

    PrecomputedMaps {
        env_cube,
        irradiance,
        prefilter,
        brdf,
    }
}

/// The linear, clamped, trilinear sampler the precompute passes use (sampling the
/// equirect / env cube). Bake-only — the runtime uses its own `rhi` samplers.
#[cfg(feature = "bake")]
fn ibl_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("review_ibl_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// Create the single-mip 2D BRDF integration LUT render target. Bake-only; carries
/// `COPY_SRC` so the bake tool can read it back.
#[cfg(feature = "bake")]
fn create_brdf_texture(device: &wgpu::Device, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_ibl_brdf_lut"),
        size: wgpu::Extent3d {
            width: BRDF_SIZE,
            height: BRDF_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: BRDF_FORMAT,
        usage,
        view_formats: &[],
    })
}

/// Baked environment cube bytes (`assets/ibl_baked/T_IBL_NN_env.bin`).
fn baked_env_bytes(environment: EnvironmentMap) -> &'static [u8] {
    match environment {
        EnvironmentMap::Hdr01 => include_bytes!("../../../assets/ibl_baked/T_IBL_01_env.bin"),
        EnvironmentMap::Hdr02 => include_bytes!("../../../assets/ibl_baked/T_IBL_02_env.bin"),
        EnvironmentMap::Hdr03 => include_bytes!("../../../assets/ibl_baked/T_IBL_03_env.bin"),
        EnvironmentMap::Hdr04 => include_bytes!("../../../assets/ibl_baked/T_IBL_04_env.bin"),
        EnvironmentMap::Hdr05 => include_bytes!("../../../assets/ibl_baked/T_IBL_05_env.bin"),
        EnvironmentMap::Hdr06 => include_bytes!("../../../assets/ibl_baked/T_IBL_06_env.bin"),
    }
}

/// Baked diffuse irradiance cube bytes (`assets/ibl_baked/T_IBL_NN_irradiance.bin`).
fn baked_irradiance_bytes(environment: EnvironmentMap) -> &'static [u8] {
    match environment {
        EnvironmentMap::Hdr01 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_01_irradiance.bin")
        }
        EnvironmentMap::Hdr02 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_02_irradiance.bin")
        }
        EnvironmentMap::Hdr03 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_03_irradiance.bin")
        }
        EnvironmentMap::Hdr04 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_04_irradiance.bin")
        }
        EnvironmentMap::Hdr05 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_05_irradiance.bin")
        }
        EnvironmentMap::Hdr06 => {
            include_bytes!("../../../assets/ibl_baked/T_IBL_06_irradiance.bin")
        }
    }
}

/// Baked prefiltered specular cube bytes (`assets/ibl_baked/T_IBL_NN_prefilter.bin`).
fn baked_prefilter_bytes(environment: EnvironmentMap) -> &'static [u8] {
    match environment {
        EnvironmentMap::Hdr01 => include_bytes!("../../../assets/ibl_baked/T_IBL_01_prefilter.bin"),
        EnvironmentMap::Hdr02 => include_bytes!("../../../assets/ibl_baked/T_IBL_02_prefilter.bin"),
        EnvironmentMap::Hdr03 => include_bytes!("../../../assets/ibl_baked/T_IBL_03_prefilter.bin"),
        EnvironmentMap::Hdr04 => include_bytes!("../../../assets/ibl_baked/T_IBL_04_prefilter.bin"),
        EnvironmentMap::Hdr05 => include_bytes!("../../../assets/ibl_baked/T_IBL_05_prefilter.bin"),
        EnvironmentMap::Hdr06 => include_bytes!("../../../assets/ibl_baked/T_IBL_06_prefilter.bin"),
    }
}

/// Baked BRDF integration LUT bytes (environment-independent — one shared file).
const BAKED_BRDF_BYTES: &[u8] = include_bytes!("../../../assets/ibl_baked/T_IBL_BRDF.bin");

/// Which source texture a precompute pass binds, so [`pass_layout`] lists the
/// matching bindings.
#[cfg(feature = "bake")]
enum PassSource {
    /// 2D equirect source (equirect→cube).
    Tex2d,
    /// Cube source (irradiance / prefilter).
    Cube,
    /// No source texture (BRDF LUT — only the vertex uniform).
    None,
}

#[cfg(feature = "bake")]
fn pass_layout(device: &wgpu::Device, label: &str, source: PassSource) -> wgpu::BindGroupLayout {
    // Binding 0 (uniform) is read by the vertex stage (face basis → direction)
    // and the fragment stage (prefilter roughness), so it is visible to both.
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    match source {
        PassSource::Tex2d => {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(sampler_entry(2));
        }
        PassSource::Cube => {
            entries.push(sampler_entry(2));
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    multisampled: false,
                },
                count: None,
            });
        }
        PassSource::None => {}
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

#[cfg(feature = "bake")]
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[cfg(feature = "bake")]
fn uniform_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

#[cfg(feature = "bake")]
fn make_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    fragment_entry: &str,
    target_format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[bind_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

/// Render a fullscreen triangle through `pipeline`/`bind_group` into `target`,
/// submitting immediately. Each precompute draw runs in its own submission so the
/// `queue.write_buffer` that set the per-face uniform is ordered before it.
#[cfg(feature = "bake")]
fn run_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("review_ibl_precompute"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("review_ibl_precompute_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(std::iter::once(encoder.finish()));
}

#[cfg(feature = "bake")]
fn write_face_uniform(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    basis: ([f32; 3], [f32; 3], [f32; 3]),
    roughness: f32,
) {
    let (forward, right, up) = basis;
    let uniform = FaceUniform {
        forward: [forward[0], forward[1], forward[2], 0.0],
        right: [right[0], right[1], right[2], 0.0],
        up: [up[0], up[1], up[2], 0.0],
        params: [roughness, 0.0, 0.0, 0.0],
    };
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(&uniform));
}

#[cfg(feature = "bake")]
fn create_cube_texture(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    mips: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// A single-face, single-mip 2D view of a cube texture, used as a render target.
#[cfg(feature = "bake")]
fn cube_face_view(texture: &wgpu::Texture, face: u32, mip: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("review_ibl_face_target"),
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_mip_level: mip,
        mip_level_count: Some(1),
        base_array_layer: face,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

/// Largest value representable as a finite `f16`. HDR suns routinely exceed this;
/// left unclamped, `f16::from_f32` rounds them to `inf`, which then propagates as
/// `NaN` through the IBL convolutions (the unclamped irradiance integral
/// especially) and the raw skybox sample — blowing out into black speckles on the
/// model and a dead spot at the sun. Clamp every channel to this ceiling so the
/// uploaded environment stays finite. (`f32::min(NaN, x)` returns `x`, so this
/// also sanitizes a stray non-finite source texel.) Bake-time only now — the
/// runtime clamps live in `ibl.wgsl` and the maps are already finite once baked.
#[cfg(feature = "bake")]
const F16_MAX: f32 = 65504.0;

// --- Offline bake tool (only compiled with the `bake` feature) -----------------
//
// Precomputes every environment's IBL maps on a headless device and writes them
// to `assets/ibl_baked/` (BC6H cubes + raw-f16 BRDF LUT), so the shipping binary
// loads them by upload (`IblD3d::from_baked`) instead of running the 43-pass
// precompute at startup. Run via the `bake_ibl` binary; outputs are committed.

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

/// Decode `environment`'s HDR from `assets/textures` on disk and upload it as an
/// `Rgba16Float` equirect texture — the bake tool's input. Paths resolve relative
/// to the render crate's manifest so the tool runs from any working directory.
#[cfg(feature = "bake")]
fn load_equirect_from_file(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    environment: EnvironmentMap,
) -> Result<wgpu::TextureView, Box<dyn std::error::Error>> {
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
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("review_ibl_equirect"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ENV_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(&halfs),
    );
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Copy one texture subresource (mip + array layer) back to the CPU as tight
/// (row-padding-stripped) bytes. `copy_texture_to_buffer` requires a 256-byte row
/// stride, so the staging buffer is padded and the padding is removed here.
#[cfg(feature = "bake")]
fn readback_subresource(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
    size: u32,
    bpp: u32,
) -> Vec<u8> {
    let tight_row = size * bpp;
    let padded_row = tight_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("review_ibl_readback"),
        size: (padded_row * size) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("review_ibl_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver
        .recv()
        .expect("map_async callback dropped")
        .expect("IBL readback buffer mapping failed");

    let tight_row = tight_row as usize;
    let padded_row = padded_row as usize;
    let mapped = slice.get_mapped_range();
    let mut out = Vec::with_capacity(tight_row * size as usize);
    for row in 0..size as usize {
        let start = row * padded_row;
        out.extend_from_slice(&mapped[start..start + tight_row]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// Shape of a baked map: base face resolution, mip count, array layers (6 for a
/// cube, 1 for the 2D BRDF LUT) and bytes per texel. The same four numbers drive
/// the runtime upload, so the `.bin` byte layout stays in lockstep.
#[cfg(feature = "bake")]
#[derive(Clone, Copy)]
struct BakedMapLayout {
    base_size: u32,
    mips: u32,
    layers: u32,
    bpp: u32,
    /// BC6H-compress each subresource before writing (the HDR cubes). The BRDF LUT
    /// stays uncompressed (`false`), so the runtime uploads it raw.
    bc6h: bool,
}

/// Read a whole texture (all mips × layers) back to `path` in mip-major,
/// faces-contiguous order — the exact byte layout the runtime upload re-reads. The
/// HDR cubes (`layout.bc6h`) are BC6H-compressed per subresource on the way out;
/// the BRDF LUT is written as raw f16.
#[cfg(feature = "bake")]
fn readback_to_file(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layout: BakedMapLayout,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let mut data = Vec::new();
    for mip in 0..layout.mips {
        let size = layout.base_size >> mip;
        for layer in 0..layout.layers {
            let raw = readback_subresource(device, queue, texture, mip, layer, size, layout.bpp);
            if layout.bc6h {
                data.extend_from_slice(&compress_bc6h_face(size, &raw));
            } else {
                data.extend_from_slice(&raw);
            }
        }
    }
    std::fs::write(path, &data)
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

/// Offline IBL bake entry point: precompute every environment's maps on a
/// headless device and write them to `assets/ibl_baked/`. Re-exported as
/// `review_render::bake_ibl_assets` for the `bake_ibl` binary. Needs a real GPU.
#[cfg(feature = "bake")]
pub fn bake_ibl_assets() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ibl_baked");
    std::fs::create_dir_all(&out_dir)?;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or("no GPU adapter available for the IBL bake")?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("review_ibl_bake_device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None,
    ))?;

    for (index, environment) in EnvironmentMap::ALL.iter().enumerate() {
        let nn = format!("{:02}", index + 1);
        let equirect_view = load_equirect_from_file(&device, &queue, *environment)?;
        let maps = precompute_maps(&device, &queue, &equirect_view);

        let cube = |base_size, mips| BakedMapLayout {
            base_size,
            mips,
            layers: 6,
            bpp: RGBA16F_BPP,
            bc6h: true,
        };
        readback_to_file(
            &device,
            &queue,
            &maps.env_cube,
            cube(ENV_CUBE_SIZE, 1),
            &out_dir.join(format!("T_IBL_{nn}_env.bin")),
        )?;
        readback_to_file(
            &device,
            &queue,
            &maps.irradiance,
            cube(IRRADIANCE_SIZE, 1),
            &out_dir.join(format!("T_IBL_{nn}_irradiance.bin")),
        )?;
        readback_to_file(
            &device,
            &queue,
            &maps.prefilter,
            cube(PREFILTER_SIZE, PREFILTER_MIPS),
            &out_dir.join(format!("T_IBL_{nn}_prefilter.bin")),
        )?;
        // The BRDF integration LUT is environment-independent — bake it once.
        if index == 0 {
            readback_to_file(
                &device,
                &queue,
                &maps.brdf,
                BakedMapLayout {
                    base_size: BRDF_SIZE,
                    mips: 1,
                    layers: 1,
                    bpp: RG16F_BPP,
                    bc6h: false,
                },
                &out_dir.join("T_IBL_BRDF.bin"),
            )?;
        }
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

#[cfg(test)]
mod shader_tests {
    /// The IBL precompute shader must parse + validate. Like the scene-shader
    /// test (see `scene::gpu_types`), this catches type / control-flow / binding
    /// mistakes without a GPU; the real check is pipeline creation in
    /// `precompute_maps` (behind the `bake` feature).
    #[test]
    fn ibl_shader_validates() {
        let module =
            naga::front::wgsl::parse_str(super::IBL_SHADER).expect("ibl.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("ibl.wgsl should validate");
    }
}
