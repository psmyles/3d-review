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
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_SAMPLE_DESC,
};
use windows::core::Result;

/// A sampled GPU texture: a 2D or cube texture plus the shader-resource view a
/// pixel shader reads it through. The underlying texture is kept alive by the SRV's
/// internal reference, so only the view is stored.
pub(crate) struct Texture {
    srv: ID3D11ShaderResourceView,
}

impl Texture {
    /// Create an immutable block-compressed **cube** texture (the baked BC6H IBL
    /// cubes) from mip-major, faces-contiguous block bytes — exactly the layout the
    /// bake tool wrote and the wgpu `upload_cube` re-read. `size` is the base face
    /// resolution; `block_bytes` is one block's size (16 for BC6H). The D3D11
    /// subresource order is `face * mips + mip`, so the mip-major source is
    /// re-indexed accordingly.
    pub(crate) fn cube_block_compressed(
        device: &ID3D11Device,
        size: u32,
        mips: u32,
        format: DXGI_FORMAT,
        block_bytes: u32,
        data: &[u8],
    ) -> Result<Self> {
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
            Format: format,
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
        unsafe { device.CreateTexture2D(&desc, Some(subdata.as_ptr()), Some(&mut texture))? };
        let texture = texture.unwrap();

        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: format,
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
        Ok(Self { srv: srv.unwrap() })
    }

    /// Create an immutable single-mip 2D texture from tightly-packed `data`
    /// (`row_pitch` bytes per row) — the raw `Rg16Float` BRDF LUT.
    pub(crate) fn immutable_2d(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        row_pitch: u32,
        data: &[u8],
    ) -> Result<Self> {
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
            Format: format,
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
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut texture))? };
        let texture = texture.unwrap();
        let mut srv = None;
        // SAFETY: `texture` is shader-resource-bindable; a default view desc matches.
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
        Ok(Self { srv: srv.unwrap() })
    }

    /// Create an immutable single-mip RGBA8 2D texture (sRGB or linear) — the
    /// per-slot 1×1 material fallbacks and the UV-checker textures, which the wgpu
    /// path also stored without mips.
    pub(crate) fn rgba8_single(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
    ) -> Result<Self> {
        let format = rgba8_format(srgb);
        Self::immutable_2d(device, width, height, format, width * 4, rgba)
    }

    /// Create an RGBA8 2D texture (sRGB or linear) with a full, runtime-generated
    /// mip chain — the material texture slots. Mip 0 is uploaded via
    /// `UpdateSubresource`, then `GenerateMips` fills the rest (replacing the wgpu
    /// render-pass downsample), so anisotropic minification has levels to filter.
    pub(crate) fn rgba8_mipped(
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
    ) -> Result<Self> {
        let format = rgba8_format(srgb);
        let mips = mip_level_count(width, height);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: mips,
            ArraySize: 1,
            Format: format,
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
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.unwrap();

        // SAFETY: writing mip 0 of `texture`; `rgba` (>= width*height*4 bytes,
        // checked by the caller) is alive for the call, `width*4` is its row pitch.
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
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
        let srv = srv.unwrap();
        // SAFETY: the SRV covers the full mip chain of a render-target-capable
        // texture, so GenerateMips can downsample mip0 into the rest.
        unsafe { ctx.GenerateMips(&srv) };
        Ok(Self { srv })
    }

    /// Bind this texture's SRV to pixel-shader slot `slot`.
    pub(crate) fn bind_ps(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the SRV is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetShaderResources(slot, Some(&[Some(self.srv.clone())]));
        }
    }
}

/// Bind `textures` to consecutive pixel-shader SRV slots starting at `slot` in one
/// call (the seven material slots → `t5..t11`).
pub(crate) fn bind_ps_textures(ctx: &ID3D11DeviceContext, slot: u32, textures: &[&Texture]) {
    let srvs: Vec<Option<ID3D11ShaderResourceView>> =
        textures.iter().map(|t| Some(t.srv.clone())).collect();
    // SAFETY: every SRV is live; the local array outlives the call.
    unsafe {
        ctx.PSSetShaderResources(slot, Some(&srvs));
    }
}

fn rgba8_format(srgb: bool) -> DXGI_FORMAT {
    if srgb {
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
    } else {
        DXGI_FORMAT_R8G8B8A8_UNORM
    }
}

/// Full mip-chain length for a `width`×`height` texture: `floor(log2(max)) + 1`.
fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}
