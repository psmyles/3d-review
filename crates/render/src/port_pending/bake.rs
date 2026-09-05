//! Offline IBL bake plumbing (only compiled with the `bake` feature).
//!
//! A headless Direct3D 11 device (no swapchain) plus the render targets the IBL
//! precompute draws into and reads back: a render-target **cube** (`Rgba16Float`,
//! 6 faces × mips, a per-face/mip RTV plus a cube SRV the convolution passes
//! sample) and a plain 2D color target (the BRDF LUT). Readback copies one
//! subresource into a staging texture and maps it.
//!
//! All D3D11 `unsafe`/COM stays here (and the sibling `rhi` modules; invariant 9
//! amendment), so the bake path in `ibl.rs` calls only safe methods. The runtime
//! renderer never touches this module — it loads the committed baked maps via
//! `IblD3d::from_baked` (a pure upload).

use super::{Format, Gpu, GpuResult, ResourceContext, ResourceKind, out_param};
use windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURECUBE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0,
    D3D11_RESOURCE_MISC_TEXTURECUBE, D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_TEX2D_ARRAY_RTV,
    D3D11_TEXCUBE_SRV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

/// A headless Direct3D 11 device + immediate context (no swapchain) for the offline
/// IBL bake, with safe wrappers for the fullscreen passes + subresource readback.
pub(crate) struct Baker {
    /// A headless [`Gpu`] — device + immediate context, no swapchain. Every bake
    /// resource is created from it exactly as the runtime's are, which is what lets
    /// `ibl.rs` share `Pipeline` / `Sampler` / `Texture` with the render path.
    gpu: Gpu,
}

impl Baker {
    /// Create the headless device + immediate context.
    pub(crate) fn new() -> GpuResult<Self> {
        Ok(Self {
            gpu: Gpu::headless()?,
        })
    }

    /// The headless device the bake's pipelines, buffers and targets are built from.
    pub(crate) fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    fn device(&self) -> &ID3D11Device {
        self.gpu.device()
    }

    fn context(&self) -> &ID3D11DeviceContext {
        self.gpu.context()
    }

    /// Begin a single-target, depth-less fullscreen pass into `rtv`: bind it as the
    /// only render target (no DSV), clear it to opaque black, and set a full
    /// `size`×`size` viewport. The fullscreen triangle then overwrites every texel.
    pub(crate) fn begin_target(&self, rtv: &ID3D11RenderTargetView, size: u32) {
        // SAFETY: `rtv` is live; the local arrays / viewport outlive the calls. The
        // immediate context owns the bound target for their duration.
        unsafe {
            self.context()
                .OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            self.context()
                .ClearRenderTargetView(rtv, &[0.0_f32, 0.0, 0.0, 1.0]);
            self.context()
                .RSSetViewports(Some(&[super::viewport(size, size)]));
        }
    }

    /// Draw the fullscreen triangle (the bake VS builds its vertices from
    /// `SV_VertexID`, so no vertex buffer is bound). The pipeline + cbuffer +
    /// SRVs/sampler must already be bound.
    pub(crate) fn draw_fullscreen(&self) {
        // SAFETY: a plain 3-vertex draw on the immediate context; bound state is the
        // caller's responsibility.
        unsafe { self.context().Draw(3, 0) };
    }

    /// Unbind all render targets — call before reading a just-rendered texture back
    /// (a resource can't be both a bound render target and a copy source).
    pub(crate) fn unbind_targets(&self) {
        // SAFETY: clearing the render-target slots with a single null view.
        unsafe { self.context().OMSetRenderTargets(Some(&[None]), None) };
    }

    /// Unbind the first two pixel-shader SRV slots — call before re-rendering the env
    /// cube for the next environment (it's still bound as the convolution source from
    /// the previous one, which would otherwise be an RTV/SRV conflict).
    pub(crate) fn unbind_srvs(&self) {
        // SAFETY: clearing SRV slots 0..2 with null views; the local array outlives.
        unsafe { self.context().PSSetShaderResources(0, Some(&[None, None])) };
    }

    /// Copy one subresource (`mip + face*mip_count` for a cube; 0 for a 2D target) of
    /// `source` back to the CPU as tight (row-padding-stripped) bytes. A matching
    /// single-subresource staging texture is created, the region copied into it, then
    /// mapped — D3D11 chooses the map row pitch, so the padding is stripped here.
    pub(crate) fn readback_subresource(
        &self,
        source: &ID3D11Texture2D,
        subresource: u32,
        width: u32,
        height: u32,
        format: Format,
        bpp: u32,
    ) -> GpuResult<Vec<u8>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format.dxgi(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        // SAFETY: a CPU-readable staging texture with no initial data; out-param set.
        unsafe {
            self.device()
                .CreateTexture2D(&desc, None, Some(&mut staging))
        }
        .resource(ResourceKind::Texture, "readback staging")?;
        let staging = out_param(staging);

        // SAFETY: `source` is not currently bound as a render target (callers unbind
        // first); the single source subresource matches the staging texture's only
        // subresource in format + size. A null box copies the whole subresource.
        unsafe {
            self.context()
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, source, subresource, None);
        }

        let tight_row = (width * bpp) as usize;
        let mut out = Vec::with_capacity(tight_row * height as usize);
        // SAFETY: `staging` is a mappable staging texture; the mapped range is at
        // least `RowPitch * height` bytes, so each tight-row copy stays in bounds.
        // `Unmap` is paired with the `Map`.
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context()
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let base = mapped.pData as *const u8;
            let row_pitch = mapped.RowPitch as usize;
            for row in 0..height as usize {
                let src = base.add(row * row_pitch);
                out.extend_from_slice(std::slice::from_raw_parts(src, tight_row));
            }
            self.context().Unmap(&staging, 0);
        }
        Ok(out)
    }
}

/// A render-target cubemap (`size`×`size`, 6 faces × `mips`) for the IBL precompute:
/// each face/mip is addressable as an RTV (a render target), and the whole texture as
/// a cube SRV (the convolution passes sample it). `Rgba16Float`, default usage.
pub(crate) struct CubeTarget {
    texture: ID3D11Texture2D,
    /// One RTV per (mip, face), indexed `mip * 6 + face`.
    rtvs: Vec<ID3D11RenderTargetView>,
    srv: ID3D11ShaderResourceView,
}

impl CubeTarget {
    pub(crate) fn new(gpu: &Gpu, size: u32, mips: u32, format: Format) -> GpuResult<Self> {
        let device = gpu.device();
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size,
            Height: size,
            MipLevels: mips,
            ArraySize: 6,
            Format: format.dxgi(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_TEXTURECUBE.0 as u32,
        };
        let mut texture = None;
        // SAFETY: a render-target-capable cube texture with no initial data (filled by
        // drawing); the out-param is set.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = out_param(texture);

        let mut rtvs = Vec::with_capacity((mips * 6) as usize);
        for mip in 0..mips {
            for face in 0..6 {
                let rtv_desc = D3D11_RENDER_TARGET_VIEW_DESC {
                    Format: format.dxgi(),
                    ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
                    Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                        Texture2DArray: D3D11_TEX2D_ARRAY_RTV {
                            MipSlice: mip,
                            FirstArraySlice: face,
                            ArraySize: 1,
                        },
                    },
                };
                let mut rtv = None;
                // SAFETY: `texture` is render-target-bindable; `rtv_desc` selects one
                // valid (mip, face) slice. Out-param set.
                unsafe {
                    device.CreateRenderTargetView(&texture, Some(&rtv_desc), Some(&mut rtv))?
                };
                rtvs.push(out_param(rtv));
            }
        }

        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: format.dxgi(),
            ViewDimension: D3D_SRV_DIMENSION_TEXTURECUBE,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                TextureCube: D3D11_TEXCUBE_SRV {
                    MostDetailedMip: 0,
                    MipLevels: mips,
                },
            },
        };
        let mut srv = None;
        // SAFETY: `texture` is shader-resource-bindable; `srv_desc` matches its cube
        // format + mip count. Out-param set.
        unsafe { device.CreateShaderResourceView(&texture, Some(&srv_desc), Some(&mut srv))? };

        Ok(Self {
            texture,
            rtvs,
            srv: out_param(srv),
        })
    }

    /// The render-target view for `face` (0..6) at `mip`. The cube has 6 faces per
    /// mip stored mip-major (see the build loop), so the view index is `mip*6 + face`.
    pub(crate) fn rtv(&self, face: u32, mip: u32) -> &ID3D11RenderTargetView {
        debug_assert!(face < 6, "cube face {face} out of range (0..6)");
        let index = (mip * 6 + face) as usize;
        debug_assert!(
            index < self.rtvs.len(),
            "cube rtv (mip {mip}, face {face}) out of range ({index} >= {})",
            self.rtvs.len()
        );
        &self.rtvs[index]
    }

    /// Bind the whole cube as a sampled shader resource at pixel-shader slot `slot`.
    pub(crate) fn bind_ps_srv(&self, gpu: &Gpu, slot: u32) {
        super::bind_ps_srv(gpu, slot, &self.srv);
    }

    /// The underlying texture, for [`Baker::readback_subresource`].
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }
}

/// A plain single-mip 2D render target (the BRDF integration LUT) — rendered into,
/// then read back. No SRV is needed (nothing samples it).
pub(crate) struct Target2D {
    texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
}

impl Target2D {
    pub(crate) fn new(gpu: &Gpu, width: u32, height: u32, format: Format) -> GpuResult<Self> {
        let device = gpu.device();
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format.dxgi(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: a render-target-capable 2D texture with no initial data; out-param set.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = out_param(texture);
        let mut rtv = None;
        // SAFETY: `texture` is render-target-bindable; a default RTV desc matches its
        // typed format. Out-param set.
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv))? };
        Ok(Self {
            texture,
            rtv: out_param(rtv),
        })
    }

    pub(crate) fn rtv(&self) -> &ID3D11RenderTargetView {
        &self.rtv
    }

    /// The underlying texture, for [`Baker::readback_subresource`].
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }
}
