//! Texture sampler plumbing: the composite / IBL linear-clamp sampler (`s1`), the
//! UV-checker repeat sampler (`s0`), and the material anisotropic-repeat sampler
//! (`s2`).

use windows::Win32::Graphics::Direct3D11::{
    D3D11_COMPARISON_NEVER, D3D11_FILTER, D3D11_FILTER_ANISOTROPIC,
    D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_FLOAT32_MAX, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_MODE, D3D11_TEXTURE_ADDRESS_WRAP,
    ID3D11SamplerState,
};

use super::{Gpu, GpuResult, ResourceContext, ResourceKind, out_param};

/// Anisotropic-filter sample count for the material sampler — 16× is the common
/// hardware ceiling.
const MATERIAL_ANISOTROPY: u32 = 16;

/// A texture sampler state.
pub(crate) struct Sampler {
    state: ID3D11SamplerState,
}

impl Sampler {
    /// A trilinear sampler with clamp addressing — the composite reads its
    /// full-resolution (1:1) inputs, so addressing is moot, but clamp avoids edge
    /// wrap on any future sub-rect sampling.
    pub(crate) fn linear_clamp(gpu: &Gpu) -> GpuResult<Self> {
        Self::create(
            gpu,
            D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            D3D11_TEXTURE_ADDRESS_CLAMP,
            1,
            "linear-clamp",
        )
    }

    /// A trilinear sampler with **repeat** addressing — the UV-checker sampler
    /// (`s0`), whose tiled UVs run past the 0..1 range.
    pub(crate) fn linear_repeat(gpu: &Gpu) -> GpuResult<Self> {
        Self::create(
            gpu,
            D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            D3D11_TEXTURE_ADDRESS_WRAP,
            1,
            "linear-repeat",
        )
    }

    /// A point (nearest), clamp-addressed sampler — the GTAO sampler (`s0` of the
    /// GTAO passes). The occlusion + bilateral blur read view normals / depths from
    /// the G-buffer, which must **not** be linearly blended across geometry edges
    /// (that would bleed occlusion); clamp keeps border samples from wrapping.
    pub(crate) fn point_clamp(gpu: &Gpu) -> GpuResult<Self> {
        Self::create(
            gpu,
            D3D11_FILTER_MIN_MAG_MIP_POINT,
            D3D11_TEXTURE_ADDRESS_CLAMP,
            1,
            "point-clamp",
        )
    }

    /// A linear-minify / point-magnify, clamp-addressed sampler — the Tex viewport
    /// sampler. Crisp texels when zoomed in (nearest magnify), smooth when zoomed out
    /// (linear minify over the mip chain); clamp keeps the border from wrapping.
    pub(crate) fn tex_view(gpu: &Gpu) -> GpuResult<Self> {
        Self::create(
            gpu,
            D3D11_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR,
            D3D11_TEXTURE_ADDRESS_CLAMP,
            1,
            "tex-view",
        )
    }

    /// An anisotropic, repeat-addressed sampler — the material sampler (`s2`).
    /// Combined with the per-texture mip chain this removes grazing-angle shimmer.
    pub(crate) fn aniso_repeat(gpu: &Gpu) -> GpuResult<Self> {
        Self::create(
            gpu,
            D3D11_FILTER_ANISOTROPIC,
            D3D11_TEXTURE_ADDRESS_WRAP,
            MATERIAL_ANISOTROPY,
            "aniso-repeat",
        )
    }

    /// The one place a sampler is actually created. Every sampler the renderer uses
    /// differs only in filter, address mode and anisotropy — the rest of the
    /// description (no comparison, no LOD bias, the full mip range) is the same for
    /// all five, so it is written once here rather than five times above.
    fn create(
        gpu: &Gpu,
        filter: D3D11_FILTER,
        address: D3D11_TEXTURE_ADDRESS_MODE,
        max_anisotropy: u32,
        label: &str,
    ) -> GpuResult<Self> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: filter,
            AddressU: address,
            AddressV: address,
            AddressW: address,
            MipLODBias: 0.0,
            MaxAnisotropy: max_anisotropy,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: D3D11_FLOAT32_MAX,
        };
        let mut state = None;
        // SAFETY: `desc` is a well-formed sampler description; the out-param is set.
        unsafe { gpu.device().CreateSamplerState(&desc, Some(&mut state)) }
            .resource(ResourceKind::Sampler, label)?;
        Ok(Self {
            state: out_param(state),
        })
    }

    /// Bind this sampler to pixel-shader sampler slot `slot`.
    pub(crate) fn bind_ps(&self, gpu: &Gpu, slot: u32) {
        // SAFETY: the sampler is live; the one-element array outlives the call.
        unsafe {
            gpu.context()
                .PSSetSamplers(slot, Some(&[Some(self.state.clone())]));
        }
    }
}
