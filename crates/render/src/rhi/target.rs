//! Render-target plumbing. Phase 1 needs only a depth target (the scene draws
//! straight to the swapchain backbuffer); the offscreen linear-HDR MRT color
//! targets + MSAA resolves join this module in Phases 2/5.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_DEPTH_STENCIL, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11DepthStencilView,
    ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_D32_FLOAT, DXGI_SAMPLE_DESC};
use windows::core::Result;

/// The scene depth buffer (`D32_FLOAT`, Reversed-Z). Recreated when the
/// framebuffer size changes; single-sample in Phase 1 (MSAA lands in Phase 5).
pub(crate) struct DepthTarget {
    view: ID3D11DepthStencilView,
    width: u32,
    height: u32,
}

impl DepthTarget {
    pub(crate) fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_D32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DEPTH_STENCIL.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: `desc` is a well-formed depth texture; the out-param is set.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.unwrap();
        let mut view = None;
        // SAFETY: `texture` is a depth-stencil-bindable texture; a default view desc
        // (None) matches its typed `D32_FLOAT` format. Out-param set.
        unsafe { device.CreateDepthStencilView(&texture, None, Some(&mut view))? };
        Ok(Self {
            view: view.unwrap(),
            width,
            height,
        })
    }

    pub(crate) fn view(&self) -> &ID3D11DepthStencilView {
        &self.view
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
