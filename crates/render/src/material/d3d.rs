//! The Direct3D 11 material table. Per material it holds the resolved seven
//! texture-slot SRVs (`t5..t11`, drawn from a path-keyed upload cache so a packed
//! map is uploaded once) and the `#[repr(C)]` [`MaterialUniform`] the shader reads
//! from cbuffer `b1`. The uniform is rewritten per draw range into one
//! `USAGE_DYNAMIC` cbuffer via `Map(WRITE_DISCARD)`; the SRVs are bound per range
//! with one `PSSetShaderResources`.
//!
//! No `unsafe` lives here: it drives the GPU through the safe `rhi` wrappers
//! (`Texture`, `DynamicConstantBuffer`, `Sampler`) — the unsafe/COM is confined to
//! `rhi` (invariant 9 amendment).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::config::MaterialMode;
use crate::rhi::{DynamicConstantBuffer, Sampler, Texture, bind_ps_textures};
use crate::texture::{TEXTURE_SLOT_COUNT, TextureSlot};

use super::state::{MaterialState, MaterialUniform};

/// The pixel-shader SRV slot the first material texture binds to (`t5`); the seven
/// slots occupy `t5..t11`, matching the register plan in `scene.hlsl`.
const MATERIAL_SRV_BASE: u32 = 5;
/// The material cbuffer register (`b1`).
const MATERIAL_CBUFFER_SLOT: u32 = 1;
/// The material sampler register (`s2`).
const MATERIAL_SAMPLER_SLOT: u32 = 2;

/// One material's GPU-resolved data: its uniform + the seven slot SRVs (each a real
/// upload or the per-slot fallback), shared by `Arc` so a packed map is one upload.
struct MaterialEntry {
    uniform: MaterialUniform,
    textures: [Arc<Texture>; TEXTURE_SLOT_COUNT],
}

/// One cached GPU upload + the source `Arc`'s pointer (the identity used to detect a
/// disk reload, which swaps in a fresh `Arc` at the same path → a re-upload).
struct CachedTexture {
    identity: usize,
    texture: Arc<Texture>,
}

/// The editable material table (Direct3D 11). Synced from the effective material
/// list when the material revision or [`MaterialMode`] changes; bound per draw range.
pub(crate) struct MaterialTableD3d {
    /// Per-draw material uniform (cbuffer `b1`), rewritten per range.
    uniform: DynamicConstantBuffer,
    /// Anisotropic-repeat material sampler (`s2`).
    sampler: Sampler,
    /// Per-slot neutral 1×1 fallbacks, bound for unassigned slots (the shader gates
    /// them off via the slot-flags bitfield, but every slot must be bound).
    fallback_textures: [Arc<Texture>; TEXTURE_SLOT_COUNT],
    /// Path+srgb-keyed GPU texture cache (uploaded once, shared across materials).
    cache: HashMap<(PathBuf, bool), CachedTexture>,
    /// One entry per material, in table order.
    entries: Vec<MaterialEntry>,
    /// The all-fallback entry, bound for ranges whose material is out of range.
    fallback_entry: MaterialEntry,
    /// Material revision + mode the entries were last synced to (a sentinel forces
    /// the first sync).
    synced: Option<(u64, MaterialMode)>,
}

impl MaterialTableD3d {
    /// Build an empty table (fallback only), before any model is loaded.
    pub(crate) fn new(device: &ID3D11Device) -> windows::core::Result<Self> {
        let uniform = DynamicConstantBuffer::new::<MaterialUniform>(device)?;
        let sampler = Sampler::aniso_repeat(device)?;
        let fallback_textures = create_fallback_textures(device)?;
        let fallback_entry = MaterialEntry {
            uniform: MaterialUniform::fallback(),
            textures: fallback_textures.clone(),
        };
        Ok(Self {
            uniform,
            sampler,
            fallback_textures,
            cache: HashMap::new(),
            entries: Vec::new(),
            fallback_entry,
            synced: None,
        })
    }

    /// Bring the table in line with `materials` (the effective table for `mode`) when
    /// the revision or mode changes: upload any newly-referenced texture once
    /// (deduplicated by path, re-uploaded on a disk reload), then build one entry per
    /// material. A no-op when nothing changed.
    pub(crate) fn sync(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        materials: &[MaterialState],
        revision: u64,
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        if self.synced == Some((revision, mode)) {
            return Ok(());
        }
        // A new model (different material count) retires the previous model's
        // uploads; drop the cache so the new slots upload fresh on demand.
        if self.entries.len() != materials.len() {
            self.cache.clear();
        }

        let mut entries = Vec::with_capacity(materials.len());
        for material in materials {
            let mut textures: [Arc<Texture>; TEXTURE_SLOT_COUNT] =
                std::array::from_fn(|slot| Arc::clone(&self.fallback_textures[slot]));
            for slot in TextureSlot::ALL {
                if let Some(binding) = &material.textures[slot.index()] {
                    let srgb = slot.is_srgb();
                    let key = (binding.path.clone(), srgb);
                    let identity = Arc::as_ptr(&binding.image) as usize;
                    let stale = self
                        .cache
                        .get(&key)
                        .is_none_or(|cached| cached.identity != identity);
                    if stale {
                        let texture = upload_binding(device, ctx, &binding.image, srgb)?;
                        self.cache.insert(
                            key.clone(),
                            CachedTexture {
                                identity,
                                texture: Arc::new(texture),
                            },
                        );
                    }
                    textures[slot.index()] = Arc::clone(&self.cache[&key].texture);
                }
            }
            entries.push(MaterialEntry {
                uniform: MaterialUniform::from_state(material),
                textures,
            });
        }
        self.entries = entries;
        self.synced = Some((revision, mode));
        Ok(())
    }

    /// Bind the shared material state once per pass: the cbuffer (`b1`) + the sampler
    /// (`s2`). Per-range, only the cbuffer contents + the slot SRVs change.
    pub(crate) fn bind_shared(&self, ctx: &ID3D11DeviceContext) {
        self.uniform.bind_ps(ctx, MATERIAL_CBUFFER_SLOT);
        self.sampler.bind_ps(ctx, MATERIAL_SAMPLER_SLOT);
    }

    /// Bind one draw range's material: rewrite the cbuffer with its uniform and bind
    /// its seven slot SRVs at `t5`. `material` indexes the table; an out-of-range
    /// index (e.g. `u32::MAX` for un-materialed triangles) uses the fallback entry.
    pub(crate) fn set_range(
        &self,
        ctx: &ID3D11DeviceContext,
        material: u32,
    ) -> windows::core::Result<()> {
        let entry = self
            .entries
            .get(material as usize)
            .unwrap_or(&self.fallback_entry);
        self.uniform.update(ctx, &entry.uniform)?;
        let refs: [&Texture; TEXTURE_SLOT_COUNT] =
            std::array::from_fn(|slot| entry.textures[slot].as_ref());
        bind_ps_textures(ctx, MATERIAL_SRV_BASE, &refs);
        Ok(())
    }

    /// Bind the all-fallback material (for the skybox / grid / overlays, which don't
    /// sample a real material but share the pipeline's resource bindings).
    pub(crate) fn bind_fallback(&self, ctx: &ID3D11DeviceContext) -> windows::core::Result<()> {
        self.uniform.update(ctx, &self.fallback_entry.uniform)?;
        let refs: [&Texture; TEXTURE_SLOT_COUNT] =
            std::array::from_fn(|slot| self.fallback_entry.textures[slot].as_ref());
        bind_ps_textures(ctx, MATERIAL_SRV_BASE, &refs);
        Ok(())
    }
}

/// Upload one decoded image into a mip-mapped material texture. Mirrors the wgpu
/// `upload_texture` guard: a genuinely empty / mismatched buffer falls back to a 1×1
/// white texel so a decode hiccup never fails the render path.
fn upload_binding(
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    image: &crate::texture::DecodedImage,
    srgb: bool,
) -> windows::core::Result<Texture> {
    let width = image.width.max(1);
    let height = image.height.max(1);
    if image.rgba.len() < (width as usize * height as usize * 4) {
        return Texture::rgba8_single(device, 1, 1, &[255, 255, 255, 255], srgb);
    }
    Texture::rgba8_mipped(device, ctx, width, height, &image.rgba, srgb)
}

/// Create the per-slot neutral 1×1 fallback textures (white base/AO/opacity,
/// `[128,128,255]` normal, mid-grey roughness/metallic, black emissive) used for
/// any unassigned slot.
fn create_fallback_textures(
    device: &ID3D11Device,
) -> windows::core::Result<[Arc<Texture>; TEXTURE_SLOT_COUNT]> {
    let mut textures: Vec<Arc<Texture>> = Vec::with_capacity(TEXTURE_SLOT_COUNT);
    for slot in TextureSlot::ALL {
        let pixel = fallback_pixel(slot);
        let texture = Texture::rgba8_single(device, 1, 1, &pixel, slot.is_srgb())?;
        textures.push(Arc::new(texture));
    }
    Ok(textures.try_into().unwrap_or_else(|_| unreachable!()))
}

/// The neutral fallback RGBA for an unassigned slot.
fn fallback_pixel(slot: TextureSlot) -> [u8; 4] {
    match slot {
        TextureSlot::BaseColor | TextureSlot::Ao | TextureSlot::Opacity => [255, 255, 255, 255],
        TextureSlot::Normal => [128, 128, 255, 255],
        TextureSlot::Roughness | TextureSlot::Metallic => [128, 128, 128, 255],
        TextureSlot::Emissive => [0, 0, 0, 255],
    }
}
