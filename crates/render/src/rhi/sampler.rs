//! Texture sampler plumbing: the composite / IBL linear-clamp sampler (`s1`), the
//! UV-checker repeat sampler (`s0`), and the material anisotropic-repeat sampler
//! (`s2`).

use windows::Win32::Graphics::Direct3D11::{
    D3D11_COMPARISON_NEVER, D3D11_FILTER_ANISOTROPIC, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_FLOAT32_MAX, D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP,
    ID3D11Device, ID3D11DeviceContext, ID3D11SamplerState,
};
use windows::core::Result;

/// Anisotropic-filter sample count for the material sampler — 16× is the common
/// hardware ceiling (mirrors the wgpu material table's `MATERIAL_ANISOTROPY`).
const MATERIAL_ANISOTROPY: u32 = 16;

/// A texture sampler state.
pub(crate) struct Sampler {
    state: ID3D11SamplerState,
}

impl Sampler {
    /// A trilinear sampler with clamp addressing — the composite reads its
    /// full-resolution (1:1) inputs, so addressing is moot, but clamp avoids edge
    /// wrap on any future sub-rect sampling.
    pub(crate) fn linear_clamp(device: &ID3D11Device) -> Result<Self> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: D3D11_FLOAT32_MAX,
        };
        let mut state = None;
        // SAFETY: `desc` is a well-formed sampler description; the out-param is set.
        unsafe { device.CreateSamplerState(&desc, Some(&mut state))? };
        Ok(Self {
            state: state.unwrap(),
        })
    }

    /// A trilinear sampler with **repeat** addressing — the UV-checker sampler
    /// (`s0`), whose tiled UVs run past the 0..1 range.
    pub(crate) fn linear_repeat(device: &ID3D11Device) -> Result<Self> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: D3D11_FLOAT32_MAX,
        };
        let mut state = None;
        // SAFETY: `desc` is a well-formed sampler description; the out-param is set.
        unsafe { device.CreateSamplerState(&desc, Some(&mut state))? };
        Ok(Self {
            state: state.unwrap(),
        })
    }

    /// An anisotropic, repeat-addressed sampler — the material sampler (`s2`).
    /// Combined with the per-texture mip chain this removes grazing-angle shimmer.
    pub(crate) fn aniso_repeat(device: &ID3D11Device) -> Result<Self> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_ANISOTROPIC,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MipLODBias: 0.0,
            MaxAnisotropy: MATERIAL_ANISOTROPY,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: D3D11_FLOAT32_MAX,
        };
        let mut state = None;
        // SAFETY: `desc` is a well-formed sampler description; the out-param is set.
        unsafe { device.CreateSamplerState(&desc, Some(&mut state))? };
        Ok(Self {
            state: state.unwrap(),
        })
    }

    /// Bind this sampler to pixel-shader sampler slot `slot`.
    pub(crate) fn bind_ps(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the sampler is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetSamplers(slot, Some(&[Some(self.state.clone())]));
        }
    }
}
