//! Texture sampler plumbing. Phase 2 needs the composite's linear-clamp sampler;
//! the material aniso-wrap sampler joins in Phase 2b.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_FLOAT32_MAX, D3D11_SAMPLER_DESC,
    D3D11_TEXTURE_ADDRESS_CLAMP, ID3D11Device, ID3D11DeviceContext, ID3D11SamplerState,
};
use windows::core::Result;

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

    /// Bind this sampler to pixel-shader sampler slot `slot`.
    pub(crate) fn bind_ps(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the sampler is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetSamplers(slot, Some(&[Some(self.state.clone())]));
        }
    }
}
