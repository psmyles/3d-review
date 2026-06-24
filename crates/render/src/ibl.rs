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
//! These are exposed through a single bind group ([`IblResources::bind_group`])
//! the scene pipelines reference as group 2. Rebuilt only when the user switches
//! environments (`IblResources::new`); never touched per frame. The precompute
//! shaders live in `ibl.wgsl`.
//!
//! Per invariant 4 the maps use only formats the adapter renders/filters
//! (`Rgba16Float` / `Rg16Float`); [`ibl_supported`] gates the UI toggle. `new`
//! itself is infallible — a decode failure falls back to a flat grey environment
//! so the bind group (and therefore the shared scene pipeline layout) is always
//! valid.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::EnvironmentMap;

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

const ENV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const BRDF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// The largest mip index of the prefiltered specular cube, exposed to the scene
/// shader so it can map roughness → mip LOD. Kept here so the two stay in step.
pub(crate) const PREFILTER_MAX_LOD: f32 = (PREFILTER_MIPS - 1) as f32;

/// Whether the adapter can build the IBL maps: the HDR color + BRDF formats must
/// be renderable, and the color format filterable (the maps are sampled with
/// linear/trilinear filtering). `Rgba16Float` filtering is core WebGPU, so this
/// is effectively always true on a desktop adapter — but invariant 4 says gate,
/// not assume, so the UI can disable IBL rather than crash on an exotic adapter.
pub fn ibl_supported(adapter: &wgpu::Adapter) -> bool {
    let color = adapter.get_texture_format_features(ENV_FORMAT);
    let brdf = adapter.get_texture_format_features(BRDF_FORMAT);
    color
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        && color
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        && brdf
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

/// The precomputed IBL maps + the scene-facing bind group (group 2).
pub(crate) struct IblResources {
    /// Which environment these maps were built from (so the scene only rebuilds
    /// on an actual change).
    pub(crate) environment: EnvironmentMap,
    pub(crate) bind_group: wgpu::BindGroup,
    /// Diffuse irradiance cube. Kept alive for the scene bind group and also
    /// re-sampled by the post pass to re-light the diffuse ambient with the GTAO
    /// bent normal, so it is exposed via [`IblResources::irradiance_view`].
    irradiance_view: wgpu::TextureView,
    // Views are kept alive for the bind group's lifetime.
    _prefilter_view: wgpu::TextureView,
    _brdf_view: wgpu::TextureView,
    _env_cube_view: wgpu::TextureView,
}

/// Per-face / per-pass uniform fed to the precompute shaders (matches the WGSL
/// `FaceUniform`, invariant 11). `forward/right/up` are the cube face basis;
/// `params.x` is the prefilter roughness.
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
const FACE_BASES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]), // +X
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), // -X
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]), // +Y
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), // -Y
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),  // +Z
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
];

impl IblResources {
    /// The bind-group layout the scene pipelines reference as group 2. Created
    /// once (it must be stable across environment switches, since the scene
    /// pipeline layout embeds it) and shared into every [`IblResources::new`].
    ///
    /// Bindings: 0 irradiance cube, 1 prefiltered cube, 2 BRDF LUT (2D),
    /// 3 environment cube (skybox), 4 sampler.
    pub(crate) fn scene_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let cube = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::Cube,
            multisampled: false,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_ibl_scene_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: cube,
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: cube,
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: cube,
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    /// Precompute every IBL map from `environment` and assemble the scene bind
    /// group against `scene_layout`. One-time GPU work (a few dozen small passes).
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        environment: EnvironmentMap,
    ) -> Self {
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

        // A linear, clamped sampler with trilinear mip filtering, shared by the
        // precompute passes (sampling the equirect / env cube) and the scene
        // (sampling irradiance / prefiltered / BRDF maps).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_ibl_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let equirect_view = load_equirect_texture(device, queue, environment);
        let env_cube = create_cube_texture(device, "review_ibl_env_cube", ENV_CUBE_SIZE, 1);
        let irradiance = create_cube_texture(device, "review_ibl_irradiance", IRRADIANCE_SIZE, 1);
        let prefilter = create_cube_texture(
            device,
            "review_ibl_prefilter",
            PREFILTER_SIZE,
            PREFILTER_MIPS,
        );

        // --- Pass layouts + pipelines (precompute-only; dropped after `new`). ---
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
                    resource: wgpu::BindingResource::TextureView(&equirect_view),
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
        let brdf = device.create_texture(&wgpu::TextureDescriptor {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let brdf_target = brdf.create_view(&wgpu::TextureViewDescriptor::default());
        write_face_uniform(queue, &uniform_buffer, FACE_BASES[0], 0.0);
        run_pass(device, queue, &brdf_pipeline, &brdf_bind, &brdf_target);

        // --- Assemble the scene bind group (group 2). ---
        let irradiance_view = irradiance.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let prefilter_view = prefilter.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let brdf_view = brdf.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_ibl_scene_bind"),
            layout: scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&irradiance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&prefilter_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&brdf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&env_cube_sample_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Self {
            environment,
            bind_group,
            irradiance_view,
            _prefilter_view: prefilter_view,
            _brdf_view: brdf_view,
            _env_cube_view: env_cube_sample_view,
        }
    }

    /// The diffuse irradiance cube view, for the post pass's bent-normal ambient
    /// re-lighting. Rebuilt with `IblResources` on every environment switch, so the
    /// post bind group referencing it must be rebuilt too (`sync_environment`).
    pub(crate) fn irradiance_view(&self) -> &wgpu::TextureView {
        &self.irradiance_view
    }
}

/// Which source texture a precompute pass binds, so [`pass_layout`] lists the
/// matching bindings.
enum PassSource {
    /// 2D equirect source (equirect→cube).
    Tex2d,
    /// Cube source (irradiance / prefilter).
    Cube,
    /// No source texture (BRDF LUT — only the vertex uniform).
    None,
}

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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

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

fn create_cube_texture(device: &wgpu::Device, label: &str, size: u32, mips: u32) -> wgpu::Texture {
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
        format: ENV_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// A single-face, single-mip 2D view of a cube texture, used as a render target.
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

/// Decode the chosen HDR and upload it as an `Rgba16Float` equirect texture. A
/// decode failure (a packaging bug, since the HDRs are baked in) falls back to a
/// flat mid-grey so IBL still produces a valid — if dull — environment.
fn load_equirect_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    environment: EnvironmentMap,
) -> wgpu::TextureView {
    let (width, height, halfs) = decode_hdr(environment);

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
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Largest value representable as a finite `f16`. HDR suns routinely exceed this;
/// left unclamped, `f16::from_f32` rounds them to `inf`, which then propagates as
/// `NaN` through the IBL convolutions (the unclamped irradiance integral
/// especially) and the raw skybox sample — blowing out into black speckles on the
/// model and a dead spot at the sun. Clamp every channel to this ceiling so the
/// uploaded environment stays finite. (`f32::min(NaN, x)` returns `x`, so this
/// also sanitizes a stray non-finite source texel.)
const F16_MAX: f32 = 65504.0;

/// Decode an environment HDR to `(width, height, rgba_f16_bits)`. RGBA, four
/// half-floats per texel (`Rgba16Float`).
fn decode_hdr(environment: EnvironmentMap) -> (u32, u32, Vec<u16>) {
    let bytes = hdr_bytes(environment);
    match image::load_from_memory(bytes) {
        Ok(image) => {
            let rgba = image.to_rgba32f();
            let (width, height) = rgba.dimensions();
            let halfs = rgba
                .iter()
                .map(|&c| half::f16::from_f32(c.min(F16_MAX)).to_bits())
                .collect();
            (width, height, halfs)
        }
        Err(_) => {
            // 1×1 mid-grey fallback.
            let grey = half::f16::from_f32(0.25).to_bits();
            (
                1,
                1,
                vec![grey, grey, grey, half::f16::from_f32(1.0).to_bits()],
            )
        }
    }
}

fn hdr_bytes(environment: EnvironmentMap) -> &'static [u8] {
    match environment {
        EnvironmentMap::Hdr01 => include_bytes!("../../../assets/textures/T_HDR_01.hdr"),
        EnvironmentMap::Hdr02 => include_bytes!("../../../assets/textures/T_HDR_02.hdr"),
        EnvironmentMap::Hdr03 => include_bytes!("../../../assets/textures/T_HDR_03.hdr"),
        EnvironmentMap::Hdr04 => include_bytes!("../../../assets/textures/T_HDR_04.hdr"),
        EnvironmentMap::Hdr05 => include_bytes!("../../../assets/textures/T_HDR_05.hdr"),
        EnvironmentMap::Hdr06 => include_bytes!("../../../assets/textures/T_HDR_06.hdr"),
    }
}

#[cfg(test)]
mod shader_tests {
    /// The IBL precompute shader must parse + validate. Like the scene-shader
    /// test (see `scene::gpu_types`), this catches type / control-flow / binding
    /// mistakes without a GPU; the real check is pipeline creation in
    /// [`super::IblResources::new`].
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
