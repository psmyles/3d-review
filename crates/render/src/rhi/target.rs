//! Render-target plumbing: the depth target plus the offscreen linear-HDR color
//! targets the scene renders into. Color targets carry the scene's MSAA level: at
//! 2×/4×/8×/16× the render texture is multisampled (render-only) and resolved into a
//! single-sample texture the composite samples; at 1× the single render texture is
//! sampled directly (no resolve). The depth target matches the same sample count.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_DEPTH_STENCIL, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11DepthStencilView, ID3D11Device,
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_SAMPLE_DESC,
};
use windows::core::Result;

/// An offscreen color target: a texture with a render-target view (a pass draws into
/// it) and a shader-resource view (a later pass samples it). At `sample_count == 1`
/// these are the same single-sample texture; at `sample_count > 1` the render texture
/// is multisampled (RTV only) and a separate single-sample `resolve` texture holds
/// the SRV — [`Self::resolve`] copies the MSAA texture into it. Recreated on resize
/// or sample-count change. The scene color / ambient targets are linear-HDR
/// (`R16G16B16A16_FLOAT`) and MSAA-capable; the GTAO G-buffer reuses the HDR format
/// single-sample, and the raw / blurred AO targets are single-channel `R8_UNORM`.
pub(crate) struct ColorTarget {
    /// The texture the scene renders into (multisampled when `sample_count > 1`).
    render: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    /// The view a later pass samples: the `resolve` texture's when multisampled, else
    /// the single-sample render texture's.
    srv: ID3D11ShaderResourceView,
    /// Single-sample resolve of `render`, present only when `sample_count > 1`. The
    /// MSAA resolve writes the per-pixel average here; the SRV reads it.
    resolve: Option<ID3D11Texture2D>,
    format: DXGI_FORMAT,
    width: u32,
    height: u32,
}

impl ColorTarget {
    /// A single-sample linear-HDR (`R16G16B16A16_FLOAT`) target — the GTAO view
    /// normal/Z G-buffer.
    pub(crate) fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        Self::build(device, width, height, DXGI_FORMAT_R16G16B16A16_FLOAT, 1)
    }

    /// A linear-HDR target at `sample_count` MSAA — the scene color / ambient MRT.
    /// `sample_count == 1` is single-sample (no resolve); `> 1` is multisampled with a
    /// single-sample resolve the composite samples.
    pub(crate) fn hdr_msaa(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Result<Self> {
        Self::build(
            device,
            width,
            height,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            sample_count,
        )
    }

    /// A single-channel single-sample `R8_UNORM` target — the raw + blurred GTAO
    /// occlusion.
    pub(crate) fn r8(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        Self::build(device, width, height, DXGI_FORMAT_R8_UNORM, 1)
    }

    fn build(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        sample_count: u32,
    ) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let sample_count = sample_count.max(1);
        let multisampled = sample_count > 1;

        // The render texture: at 1× it is also sampled (SRV); at 2×+ it is render-only
        // (a multisample texture can't back a plain 2D SRV) and the resolve below
        // carries the SRV.
        let render_bind = if multisampled {
            D3D11_BIND_RENDER_TARGET.0
        } else {
            D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0
        };
        let render = create_texture(
            device,
            width,
            height,
            format,
            sample_count,
            render_bind as u32,
        )?;
        let mut rtv = None;
        // SAFETY: `render` is render-target-bindable; a default RTV desc infers the
        // (MS or 2D) view dimension from the texture. Out-param set.
        unsafe { device.CreateRenderTargetView(&render, None, Some(&mut rtv))? };

        let (srv, resolve) = if multisampled {
            // Single-sample resolve target, sampled after the MSAA resolve.
            let resolve = create_texture(
                device,
                width,
                height,
                format,
                1,
                D3D11_BIND_SHADER_RESOURCE.0 as u32,
            )?;
            (create_srv(device, &resolve)?, Some(resolve))
        } else {
            (create_srv(device, &render)?, None)
        };

        Ok(Self {
            render,
            rtv: rtv.unwrap(),
            srv,
            resolve,
            format,
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

    /// Resolve the multisampled render texture into the single-sample resolve texture
    /// (a no-op when single-sample). Call after the scene pass has drawn + the render
    /// texture is no longer bound as a render target, before the composite samples it.
    pub(crate) fn resolve(&self, ctx: &ID3D11DeviceContext) {
        if let Some(resolve) = &self.resolve {
            // SAFETY: `resolve` (single-sample) + `render` (multisample) share the same
            // format + size; neither is bound as a render target at the call site.
            unsafe { ctx.ResolveSubresource(resolve, 0, &self.render, 0, self.format) };
        }
    }

    /// Bind the sampled view to pixel-shader slot `slot` (the resolve when MSAA, the
    /// render texture otherwise).
    pub(crate) fn bind_ps_srv(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the SRV is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetShaderResources(slot, Some(&[Some(self.srv.clone())]));
        }
    }
}

/// The scene depth buffer (`D32_FLOAT`, Reversed-Z), at `sample_count` MSAA to match
/// the color MRT. Recreated alongside the color targets on resize / sample-count
/// change.
pub(crate) struct DepthTarget {
    view: ID3D11DepthStencilView,
}

impl DepthTarget {
    /// A single-sample depth target — the GTAO G-buffer depth.
    pub(crate) fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        Self::with_samples(device, width, height, 1)
    }

    /// A depth target at `sample_count` MSAA — the scene depth.
    pub(crate) fn with_samples(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Result<Self> {
        let texture = create_texture(
            device,
            width.max(1),
            height.max(1),
            DXGI_FORMAT_D32_FLOAT,
            sample_count.max(1),
            D3D11_BIND_DEPTH_STENCIL.0 as u32,
        )?;
        let mut view = None;
        // SAFETY: `texture` is depth-stencil-bindable; a default DSV desc (None)
        // matches its typed `D32_FLOAT` format + (MS or 2D) dimension. Out-param set.
        unsafe { device.CreateDepthStencilView(&texture, None, Some(&mut view))? };
        Ok(Self {
            view: view.unwrap(),
        })
    }

    pub(crate) fn view(&self) -> &ID3D11DepthStencilView {
        &self.view
    }
}

/// Create a default-usage 2D texture at the given format / sample count / bind flags
/// (no initial data — render targets are filled by drawing / resolving).
fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    sample_count: u32,
    bind_flags: u32,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: sample_count,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: `desc` is a well-formed render texture; no initial data; out-param set.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.unwrap())
}

/// Create a default (full-mip, matching-format) shader-resource view over `texture`.
fn create_srv(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView> {
    let mut srv = None;
    // SAFETY: `texture` is shader-resource-bindable; a default view desc matches its
    // typed format. Out-param set.
    unsafe { device.CreateShaderResourceView(texture, None, Some(&mut srv))? };
    Ok(srv.unwrap())
}
