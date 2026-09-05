//! GPU texture plumbing for the scene path: immutable block-compressed cube maps
//! (the baked BC6H IBL cubes), immutable raw 2D maps (the BRDF LUT + the per-slot
//! fallbacks + the checker), and RGBA8 2D textures with a runtime-generated mip
//! chain (the material slots). Each owns a single shader-resource view the pixel
//! shaders sample.
//!
//! All D3D11 `unsafe`/COM lives here (and the sibling `rhi` modules); the material /
//! IBL paths drive these through safe methods only (invariant 9 amendment).

use std::ffi::c_void;

use windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_TEXTURECUBE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_FLAG,
    D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_RESOURCE_MISC_TEXTURECUBE,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXCUBE_SRV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE,
    ID3D11ShaderResourceView,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

use super::{Format, Gpu, GpuError, GpuResult, ResourceContext, ResourceKind, out_param};

/// A sampled GPU texture: a 2D or cube texture plus the shader-resource view a
/// pixel shader reads it through. The underlying texture is kept alive by the SRV's
/// internal reference, so only the view is stored.
pub(crate) struct Texture {
    srv: ID3D11ShaderResourceView,
}

impl Texture {
    /// Create an immutable block-compressed **cube** texture (the baked BC6H IBL
    /// cubes) from mip-major, faces-contiguous block bytes — exactly the layout the
    /// bake tool wrote. `size` is the base face resolution; the block size comes
    /// from `format` itself, so a caller can no longer disagree with it. The D3D11
    /// subresource order is `face * mips + mip`, so the mip-major source is
    /// re-indexed accordingly.
    pub(crate) fn cube_block_compressed(
        gpu: &Gpu,
        size: u32,
        mips: u32,
        format: Format,
        data: &[u8],
    ) -> GpuResult<Self> {
        let device = gpu.device();
        debug_assert!(
            format.is_block_compressed(),
            "cube_block_compressed needs a block-compressed format"
        );
        let block_bytes = format.block_bytes();
        // Validate the payload against the computed subresource layout *before*
        // handing pointers to the driver: a truncated baked asset would otherwise
        // make D3D11 read past the end of `data` (out of bounds, not an error).
        let total_len: usize = (0..mips)
            .map(|mip| {
                let blocks = (size >> mip).max(1).div_ceil(4) as usize;
                blocks * blocks * block_bytes as usize * 6
            })
            .sum();
        if data.len() < total_len {
            return Err(GpuError::invalid_arg(
                "block-compressed cube payload is shorter than its subresource layout",
            ));
        }

        let mut subdata = vec![D3D11_SUBRESOURCE_DATA::default(); (6 * mips) as usize];
        let mut offset = 0usize;
        for mip in 0..mips {
            let face_size = (size >> mip).max(1);
            // Block-compressed faces stride by whole rows of 4×4 blocks.
            let blocks = face_size.div_ceil(4);
            let row_pitch = blocks * block_bytes;
            let len = (row_pitch * blocks) as usize;
            for face in 0..6 {
                let index = (face * mips + mip) as usize;
                subdata[index] = D3D11_SUBRESOURCE_DATA {
                    pSysMem: data[offset..].as_ptr() as *const c_void,
                    SysMemPitch: row_pitch,
                    SysMemSlicePitch: len as u32,
                };
                offset += len;
            }
        }

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
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_TEXTURECUBE.0 as u32,
        };
        let mut texture = None;
        // SAFETY: `desc` describes an immutable cube; `subdata` (alive for the call)
        // holds the 6×mips initial-data entries pointing into `data` (also alive).
        unsafe { device.CreateTexture2D(&desc, Some(subdata.as_ptr()), Some(&mut texture)) }
            .resource(ResourceKind::Texture, "block-compressed cube")?;
        let texture = out_param(texture);

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
        unsafe { device.CreateShaderResourceView(&texture, Some(&srv_desc), Some(&mut srv)) }
            .resource(ResourceKind::Texture, "block-compressed cube view")?;
        Ok(Self {
            srv: out_param(srv),
        })
    }

    /// Create an immutable single-mip 2D texture from tightly-packed `data`
    /// (`row_pitch` bytes per row) — the raw [`Format::Rg16F`] BRDF LUT.
    pub(crate) fn immutable_2d(
        gpu: &Gpu,
        width: u32,
        height: u32,
        format: Format,
        row_pitch: u32,
        data: &[u8],
    ) -> GpuResult<Self> {
        let device = gpu.device();
        // The driver reads `row_pitch` bytes per row for `height` rows; a short
        // payload would be an out-of-bounds read, so reject it here.
        if data.len() < row_pitch as usize * height as usize {
            return Err(GpuError::invalid_arg(
                "2D texture payload is shorter than pitch × height",
            ));
        }
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const c_void,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: 0,
        };
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
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: immutable 2D texture with full initial data alive for the call.
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut texture)) }
            .resource(ResourceKind::Texture, "immutable 2D")?;
        let texture = out_param(texture);
        let mut srv = None;
        // SAFETY: `texture` is shader-resource-bindable; a default view desc matches.
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv)) }
            .resource(ResourceKind::Texture, "immutable 2D view")?;
        Ok(Self {
            srv: out_param(srv),
        })
    }

    /// Create an immutable single-mip RGBA8 2D texture (sRGB or linear) — the
    /// per-slot 1×1 material fallbacks and the UV-checker textures (no mips needed).
    pub(crate) fn rgba8_single(
        gpu: &Gpu,
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
    ) -> GpuResult<Self> {
        Self::immutable_2d(gpu, width, height, rgba8_format(srgb), width * 4, rgba)
    }

    /// Create an RGBA8 2D texture (sRGB or linear) with a full, runtime-generated
    /// mip chain — the material texture slots. Mip 0 is uploaded via
    /// `UpdateSubresource`, then `GenerateMips` fills the rest, so anisotropic
    /// minification has levels to filter.
    pub(crate) fn rgba8_mipped(
        gpu: &Gpu,
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
    ) -> GpuResult<Self> {
        let (device, ctx) = (gpu.device(), gpu.context());
        // Enforced here (not just documented at the call sites): the mip-0 upload
        // below reads `width*height*4` bytes.
        if rgba.len() < width as usize * height as usize * 4 {
            return Err(GpuError::invalid_arg(
                "RGBA8 payload is shorter than width × height × 4",
            ));
        }
        let format = rgba8_format(srgb);
        let mips = mip_level_count(width, height);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: mips,
            ArraySize: 1,
            Format: format.dxgi(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            // GenerateMips needs both render-target + shader-resource binding.
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
        };
        let mut texture = None;
        // SAFETY: a default-usage mip-mapped 2D texture; mip0 is filled below and the
        // rest generated, so no initial data is supplied.
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .resource(ResourceKind::Texture, "mipped RGBA8")?;
        let texture = out_param(texture);

        // SAFETY: writing mip 0 of `texture`; `rgba` (>= width*height*4 bytes,
        // checked above) is alive for the call, `width*4` is its row pitch.
        unsafe {
            ctx.UpdateSubresource(
                &texture,
                0,
                None,
                rgba.as_ptr() as *const c_void,
                width * 4,
                0,
            );
        }
        let mut srv = None;
        // SAFETY: `texture` is shader-resource-bindable across all mips; default desc.
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv)) }
            .resource(ResourceKind::Texture, "mipped RGBA8 view")?;
        let srv = out_param(srv);
        // SAFETY: the SRV covers the full mip chain of a render-target-capable
        // texture, so GenerateMips can downsample mip0 into the rest.
        unsafe { ctx.GenerateMips(&srv) };
        Ok(Self { srv })
    }

    /// Upload a decoded RGBA8 image as a mip-mapped texture, falling back to a
    /// 1×1 white texel when the pixel buffer is degenerate / mismatched — a
    /// decode hiccup must never fail the render path. Shared by the material
    /// slots (sRGB per slot) and the Tex viewport (raw).
    pub(crate) fn rgba8_mipped_or_white(
        gpu: &Gpu,
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
    ) -> GpuResult<Self> {
        let width = width.max(1);
        let height = height.max(1);
        if rgba.len() < (width as usize * height as usize * 4) {
            return Self::rgba8_single(gpu, 1, 1, &[255, 255, 255, 255], srgb);
        }
        Self::rgba8_mipped(gpu, width, height, rgba, srgb)
    }

    /// Bind this texture's SRV to pixel-shader slot `slot`.
    pub(crate) fn bind_ps(&self, gpu: &Gpu, slot: u32) {
        super::bind_ps_srv(gpu, slot, &self.srv);
    }
}

/// Bind `textures` to consecutive pixel-shader SRV slots starting at `slot` in one
/// call (the seven material slots → `t5..t11`). Fixed-size so the per-draw-range
/// rebind allocates nothing.
pub(crate) fn bind_ps_textures<const N: usize>(gpu: &Gpu, slot: u32, textures: &[&Texture; N]) {
    let srvs: [Option<ID3D11ShaderResourceView>; N] =
        std::array::from_fn(|index| Some(textures[index].srv.clone()));
    // SAFETY: every SRV is live; the local array outlives the call.
    unsafe {
        gpu.context().PSSetShaderResources(slot, Some(&srvs));
    }
}

const fn rgba8_format(srgb: bool) -> Format {
    if srgb {
        Format::Rgba8Srgb
    } else {
        Format::Rgba8
    }
}

/// Full mip-chain length for a `width`×`height` texture: `floor(log2(max)) + 1`.
fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}
