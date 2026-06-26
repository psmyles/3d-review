//! Render-target plumbing: the depth target plus the offscreen linear-HDR color
//! targets the scene renders into (Phase 2). MSAA-resolve variants join in Phase 5.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_DEPTH_STENCIL, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11DepthStencilView, ID3D11Device,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_SAMPLE_DESC,
};
use windows::core::Result;

/// An offscreen color target: a texture with both a render-target view (a pass
/// draws into it) and a shader-resource view (a later pass samples it). Single-
/// sample; the MSAA-resolved variant lands in Phase 5. Recreated on resize. The
/// scene color / ambient targets are linear-HDR (`R16G16B16A16_FLOAT`); the GTAO
/// G-buffer reuses that HDR format (view normal + Z), and the raw / blurred AO
/// targets are single-channel `R8_UNORM`.
pub(crate) struct ColorTarget {
    rtv: ID3D11RenderTargetView,
    srv: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
}

impl ColorTarget {
    /// A linear-HDR (`R16G16B16A16_FLOAT`) target — the scene color / ambient MRT
    /// and the GTAO view normal/Z G-buffer.
    pub(crate) fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        Self::with_format(device, width, height, DXGI_FORMAT_R16G16B16A16_FLOAT)
    }

    /// A single-channel `R8_UNORM` target — the raw + blurred GTAO occlusion.
    pub(crate) fn r8(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        Self::with_format(device, width, height, DXGI_FORMAT_R8_UNORM)
    }

    fn with_format(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
    ) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: `desc` is a well-formed render texture; the out-param is set.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.unwrap();
        let mut rtv = None;
        let mut srv = None;
        // SAFETY: `texture` is render-target- and shader-resource-bindable; default
        // view descs (None) match its typed format. Out-params set.
        unsafe {
            device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
            device.CreateShaderResourceView(&texture, None, Some(&mut srv))?;
        }
        Ok(Self {
            rtv: rtv.unwrap(),
            srv: srv.unwrap(),
            width,
            height,
        })
    }

    pub(crate) fn rtv(&self) -> &ID3D11RenderTargetView {
        &self.rtv
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Bind this target's shader-resource view to pixel-shader slot `slot` (for the
    /// composite to sample).
    pub(crate) fn bind_ps_srv(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the SRV is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetShaderResources(slot, Some(&[Some(self.srv.clone())]));
        }
    }
}

/// The scene depth buffer (`D32_FLOAT`, Reversed-Z). Recreated alongside the
/// color targets when the framebuffer resizes; single-sample in Phase 2 (MSAA
/// lands in Phase 5).
pub(crate) struct DepthTarget {
    view: ID3D11DepthStencilView,
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
        })
    }

    pub(crate) fn view(&self) -> &ID3D11DepthStencilView {
        &self.view
    }
}
