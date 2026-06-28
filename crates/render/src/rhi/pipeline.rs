//! Graphics pipeline plumbing: shader creation from DXBC bytecode, the input
//! layout, and the bundled D3D11 state objects (rasterizer / depth-stencil /
//! blend) that a wgpu `RenderPipeline` rolled into one. [`Pipeline::bind`] does
//! the `IASet*` / `VSSetShader` / `PSSetShader` / `RSSetState` /
//! `OMSetDepthStencilState` / `OMSetBlendState` that wgpu's `set_pipeline` did in
//! one call.
//!
//! All D3D11 `unsafe`/COM lives here (and the sibling `rhi` modules); the scene
//! path drives these through safe methods only (invariant 9 amendment).

use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY, D3D_PRIMITIVE_TOPOLOGY_LINELIST, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_APPEND_ALIGNED_ELEMENT, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_COMPARISON_ALWAYS, D3D11_COMPARISON_GREATER_EQUAL, D3D11_CULL_BACK, D3D11_CULL_FRONT,
    D3D11_CULL_NONE, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL,
    D3D11_DEPTH_WRITE_MASK_ZERO, D3D11_FILL_SOLID, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    ID3D11BlendState, ID3D11DepthStencilState, ID3D11Device, ID3D11DeviceContext,
    ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT,
    DXGI_FORMAT_R32G32B32A32_FLOAT,
};
use windows::core::{BOOL, PCSTR, Result};

/// Vertex-attribute element formats the scene buffers use, mapped to DXGI.
#[derive(Clone, Copy)]
pub(crate) enum VertexFormat {
    Float2,
    Float3,
    Float4,
}

impl VertexFormat {
    fn dxgi(self) -> DXGI_FORMAT {
        match self {
            VertexFormat::Float2 => DXGI_FORMAT_R32G32_FLOAT,
            VertexFormat::Float3 => DXGI_FORMAT_R32G32B32_FLOAT,
            VertexFormat::Float4 => DXGI_FORMAT_R32G32B32A32_FLOAT,
        }
    }
}

/// One input-layout element. Offsets are auto-computed (`APPEND_ALIGNED`), so the
/// caller only names the HLSL semantic + its format, in vertex-struct order.
#[derive(Clone, Copy)]
pub(crate) struct InputElement {
    /// HLSL semantic name (without the trailing index), e.g. `"POSITION"`.
    pub(crate) semantic: &'static str,
    pub(crate) semantic_index: u32,
    pub(crate) format: VertexFormat,
}

impl InputElement {
    pub(crate) const fn new(
        semantic: &'static str,
        semantic_index: u32,
        format: VertexFormat,
    ) -> Self {
        Self {
            semantic,
            semantic_index,
            format,
        }
    }
}

/// Primitive topology for a pipeline.
// `TriangleList` is the mesh/skybox/UV-fill topology, wired in Phase 2.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum Topology {
    TriangleList,
    LineList,
}

impl Topology {
    fn d3d(self) -> D3D_PRIMITIVE_TOPOLOGY {
        match self {
            Topology::TriangleList => D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            Topology::LineList => D3D_PRIMITIVE_TOPOLOGY_LINELIST,
        }
    }
}

/// Face culling mode. Front faces are counter-clockwise (matching the wgpu
/// `FrontFace::Ccw` + glam's `_rh` projection), so the rasterizer sets
/// `FrontCounterClockwise = TRUE`.
// `Back`/`Front` culling is used by the mesh pipeline (Phase 2); Phase 1 lines
// don't cull.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum Cull {
    None,
    Back,
    Front,
}

/// Depth comparison: Reversed-Z geometry uses `GreaterEqual`; the skybox draws
/// with `Always` (Phase 2).
#[allow(dead_code)] // `Always` is the skybox depth func (Phase 2).
#[derive(Clone, Copy)]
pub(crate) enum DepthCompare {
    GreaterEqual,
    Always,
}

/// Depth-stencil configuration. Reversed-Z: depth clears to 0, the mesh writes +
/// compares `GreaterEqual`; line overlays test but don't write.
#[derive(Clone, Copy)]
pub(crate) struct DepthState {
    pub(crate) test: bool,
    pub(crate) write: bool,
    pub(crate) compare: DepthCompare,
}

/// Color blend mode for the (single) render target.
// `Opaque` is the skybox/G-buffer blend (Phase 2); Phase 1 lines alpha-blend.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum BlendMode {
    /// No blending (opaque overwrite).
    Opaque,
    /// Standard straight-alpha blend (`src.a` over), matching wgpu's
    /// `BlendState::ALPHA_BLENDING`.
    AlphaBlend,
}

/// Slope-scaled depth bias (real depth-buffer units). The mesh pushes its surface
/// back so coplanar line overlays win the Reversed-Z test (Phase 2); lines use
/// zero bias.
#[derive(Clone, Copy, Default)]
pub(crate) struct DepthBias {
    pub(crate) constant: i32,
    pub(crate) slope_scaled: f32,
}

/// Everything needed to build a [`Pipeline`]: the compiled shader blobs, the input
/// layout, and the fixed-function state. An rhi-owned description so the scene
/// path never touches D3D11 descs directly.
pub(crate) struct PipelineDesc<'a> {
    pub(crate) vs: &'a [u8],
    pub(crate) ps: &'a [u8],
    pub(crate) input: &'a [InputElement],
    pub(crate) topology: Topology,
    pub(crate) cull: Cull,
    pub(crate) depth: DepthState,
    pub(crate) blend: BlendMode,
    pub(crate) depth_bias: DepthBias,
    /// MSAA sample count the pipeline renders at (1 = single-sample). Drives the
    /// rasterizer's `MultisampleEnable`.
    pub(crate) sample_count: u32,
}

/// A bundled graphics pipeline: the VS + PS, the input layout, and the three
/// fixed-function state objects + topology. [`Self::bind`] sets them all on the
/// immediate context (the moral equivalent of wgpu's `set_pipeline`).
pub(crate) struct Pipeline {
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    /// `None` for vertex-buffer-less passes (the fullscreen composite, which builds
    /// its vertices from `SV_VertexID`).
    input_layout: Option<ID3D11InputLayout>,
    rasterizer: ID3D11RasterizerState,
    depth_stencil: ID3D11DepthStencilState,
    blend: ID3D11BlendState,
    topology: D3D_PRIMITIVE_TOPOLOGY,
}

impl Pipeline {
    pub(crate) fn new(device: &ID3D11Device, desc: &PipelineDesc) -> Result<Self> {
        let vertex_shader = create_vertex_shader(device, desc.vs)?;
        let pixel_shader = create_pixel_shader(device, desc.ps)?;
        let input_layout = if desc.input.is_empty() {
            None
        } else {
            Some(create_input_layout(device, desc.input, desc.vs)?)
        };
        let rasterizer = create_rasterizer(device, desc.cull, desc.depth_bias, desc.sample_count)?;
        let depth_stencil = create_depth_stencil(device, desc.depth)?;
        let blend = create_blend(device, desc.blend)?;
        Ok(Self {
            vertex_shader,
            pixel_shader,
            input_layout,
            rasterizer,
            depth_stencil,
            blend,
            topology: desc.topology.d3d(),
        })
    }

    /// Bind this pipeline's shaders + state to the immediate context. The blend
    /// factor is unused by both blend modes, so it's left at zero.
    pub(crate) fn bind(&self, ctx: &ID3D11DeviceContext) {
        // SAFETY: all handles are live for `self`'s lifetime; the immediate context
        // owns them for the duration of these state-setting calls.
        unsafe {
            match &self.input_layout {
                Some(layout) => ctx.IASetInputLayout(layout),
                None => ctx.IASetInputLayout(None),
            }
            ctx.IASetPrimitiveTopology(self.topology);
            ctx.VSSetShader(&self.vertex_shader, None);
            ctx.PSSetShader(&self.pixel_shader, None);
            ctx.RSSetState(&self.rasterizer);
            ctx.OMSetDepthStencilState(&self.depth_stencil, 0);
            ctx.OMSetBlendState(&self.blend, Some(&[0.0_f32; 4]), u32::MAX);
        }
    }
}

fn create_vertex_shader(device: &ID3D11Device, dxbc: &[u8]) -> Result<ID3D11VertexShader> {
    let mut shader = None;
    // SAFETY: `dxbc` is a valid compiled-shader blob; the out-param is populated by
    // the call.
    unsafe { device.CreateVertexShader(dxbc, None, Some(&mut shader))? };
    Ok(shader.unwrap())
}

fn create_pixel_shader(device: &ID3D11Device, dxbc: &[u8]) -> Result<ID3D11PixelShader> {
    let mut shader = None;
    // SAFETY: `dxbc` is a valid compiled-shader blob; the out-param is populated.
    unsafe { device.CreatePixelShader(dxbc, None, Some(&mut shader))? };
    Ok(shader.unwrap())
}

/// Build the input layout from rhi [`InputElement`]s, validated against the vertex
/// shader's signature (`vs_dxbc`). The null-terminated semantic names are kept
/// alive in `names` for the duration of the `CreateInputLayout` call.
fn create_input_layout(
    device: &ID3D11Device,
    elements: &[InputElement],
    vs_dxbc: &[u8],
) -> Result<ID3D11InputLayout> {
    let names: Vec<Vec<u8>> = elements
        .iter()
        .map(|element| {
            let mut bytes = element.semantic.as_bytes().to_vec();
            bytes.push(0);
            bytes
        })
        .collect();
    let descs: Vec<D3D11_INPUT_ELEMENT_DESC> = elements
        .iter()
        .zip(&names)
        .map(|(element, name)| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(name.as_ptr()),
            SemanticIndex: element.semantic_index,
            Format: element.format.dxgi(),
            InputSlot: 0,
            AlignedByteOffset: D3D11_APPEND_ALIGNED_ELEMENT,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        })
        .collect();
    let mut layout = None;
    // SAFETY: `descs` (and the `names` they point into) outlive the call; `vs_dxbc`
    // is the matching compiled vertex shader.
    unsafe { device.CreateInputLayout(&descs, vs_dxbc, Some(&mut layout))? };
    Ok(layout.unwrap())
}

fn create_rasterizer(
    device: &ID3D11Device,
    cull: Cull,
    bias: DepthBias,
    sample_count: u32,
) -> Result<ID3D11RasterizerState> {
    let cull_mode = match cull {
        Cull::None => D3D11_CULL_NONE,
        Cull::Back => D3D11_CULL_BACK,
        Cull::Front => D3D11_CULL_FRONT,
    };
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: cull_mode,
        // Front faces are CCW (glam `_rh` projection + the framebuffer winding).
        FrontCounterClockwise: BOOL(1),
        DepthBias: bias.constant,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: bias.slope_scaled,
        DepthClipEnable: BOOL(1),
        ScissorEnable: BOOL(0),
        MultisampleEnable: BOOL((sample_count > 1) as i32),
        AntialiasedLineEnable: BOOL(0),
    };
    let mut state = None;
    // SAFETY: `desc` is a well-formed rasterizer description; the out-param is set.
    unsafe { device.CreateRasterizerState(&desc, Some(&mut state))? };
    Ok(state.unwrap())
}

fn create_depth_stencil(
    device: &ID3D11Device,
    depth: DepthState,
) -> Result<ID3D11DepthStencilState> {
    let compare = match depth.compare {
        DepthCompare::GreaterEqual => D3D11_COMPARISON_GREATER_EQUAL,
        DepthCompare::Always => D3D11_COMPARISON_ALWAYS,
    };
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: BOOL(depth.test as i32),
        DepthWriteMask: if depth.write {
            D3D11_DEPTH_WRITE_MASK_ALL
        } else {
            D3D11_DEPTH_WRITE_MASK_ZERO
        },
        DepthFunc: compare,
        StencilEnable: BOOL(0),
        ..Default::default()
    };
    let mut state = None;
    // SAFETY: `desc` is a well-formed depth-stencil description; out-param set.
    unsafe { device.CreateDepthStencilState(&desc, Some(&mut state))? };
    Ok(state.unwrap())
}

fn create_blend(device: &ID3D11Device, mode: BlendMode) -> Result<ID3D11BlendState> {
    let target = match mode {
        BlendMode::Opaque => D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: BOOL(0),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_ZERO,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_ZERO,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        },
        BlendMode::AlphaBlend => D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: BOOL(1),
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        },
    };
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0] = target;
    let mut state = None;
    // SAFETY: `desc` is a well-formed blend description; out-param set.
    unsafe { device.CreateBlendState(&desc, Some(&mut state))? };
    Ok(state.unwrap())
}
