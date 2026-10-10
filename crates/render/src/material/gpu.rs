//! The GPU material table. Per material it holds the resolved seven texture-slot
//! textures (drawn from a path-keyed upload cache so a packed map is uploaded once)
//! and the `#[repr(C)]` [`MaterialUniform`] the shader reads from its own uniform
//! block.
//!
//! Under sokol the per-range switch is two calls with no state left behind: the
//! uniform goes up with `apply_uniforms`, and the seven textures are seven slots of
//! the one `Bindings` value the draw is given. That is why there is no `bind_shared`
//! / `set_range` pair any more — a range hands back what it needs and the caller
//! assembles one bindings value per draw.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::MaterialMode;
use crate::rhi::{Bindings, GpuResult, Sampler, Texture};
use crate::shaders::generated;
use crate::texture::{TEXTURE_SLOT_COUNT, TextureSlot};

use super::state::{MaterialState, MaterialUniform};

/// How many uploads the cache keeps beyond what the *current* material table
/// references — the same LRU bound `TexGpu` puts on the Tex viewport's cache, for
/// the same reason: a mipped 4K RGBA8 upload is ~22 MB, and without a bound a
/// session that loads several models (or re-points one slot at a series of
/// candidate files) grows VRAM for as long as it runs.
///
/// Entries the current sync touched are never evicted, so a table wider than this
/// still gets every one of its textures; the cap governs only how much of the
/// *previous* table is kept warm, which pays off across the models of one asset
/// set, where the same maps recur.
const MATERIAL_CACHE_CAP: usize = 16;

/// The view slot the first material texture binds to; the seven occupy that slot and
/// the six above it, which the generated constants confirm below.
const MATERIAL_VIEW_BASE: usize = generated::VIEW_BASE_COLOR_TEX;
const _: () = assert!(
    generated::VIEW_NORMAL_TEX == MATERIAL_VIEW_BASE + 1
        && generated::VIEW_ROUGHNESS_TEX == MATERIAL_VIEW_BASE + 2
        && generated::VIEW_METALLIC_TEX == MATERIAL_VIEW_BASE + 3
        && generated::VIEW_AO_TEX == MATERIAL_VIEW_BASE + 4
        && generated::VIEW_EMISSIVE_TEX == MATERIAL_VIEW_BASE + 5
        && generated::VIEW_OPACITY_TEX == MATERIAL_VIEW_BASE + 6,
    "the seven material texture slots are no longer consecutive in review.glsl"
);

/// One material's GPU-resolved data: its uniform + the seven slot textures (each a
/// real upload or the per-slot fallback), shared by `Arc` so a packed map is one
/// upload.
pub(crate) struct MaterialEntry {
    uniform: MaterialUniform,
    textures: [Arc<Texture>; TEXTURE_SLOT_COUNT],
}

impl MaterialEntry {
    /// The uniform block this material's draws upload.
    pub(crate) fn uniform(&self) -> &MaterialUniform {
        &self.uniform
    }

    /// Bind this material's seven texture slots into `bindings`.
    pub(crate) fn bind_textures(&self, bindings: &mut Bindings) {
        for (slot, texture) in self.textures.iter().enumerate() {
            bindings.texture(MATERIAL_VIEW_BASE + slot, texture);
        }
    }
}

/// One cached GPU upload + the source `Arc`'s pointer (the identity used to detect a
/// disk reload, which swaps in a fresh `Arc` at the same path → a re-upload), and the
/// sync that last referenced it (the LRU stamp).
struct CachedTexture {
    identity: usize,
    texture: Arc<Texture>,
    last_used: u64,
}

/// The editable material table. Synced from the effective material list when the
/// material revision or [`MaterialMode`] changes; read per draw range.
pub(crate) struct MaterialTable {
    /// Anisotropic-repeat material sampler.
    sampler: Sampler,
    /// Per-slot neutral 1×1 fallbacks, bound for unassigned slots (the shader gates
    /// them off via the slot-flags bitfield, but every declared slot must be bound).
    fallback_textures: [Arc<Texture>; TEXTURE_SLOT_COUNT],
    /// Path+srgb-keyed GPU texture cache (uploaded once, shared across materials),
    /// bounded by [`MATERIAL_CACHE_CAP`] beyond the live table.
    cache: HashMap<(PathBuf, bool), CachedTexture>,
    /// Monotonic sync counter stamped onto every entry a [`Self::sync`] references,
    /// so one stamp marks a whole table as live and the smallest marks the LRU.
    tick: u64,
    /// One entry per material, in table order.
    entries: Vec<MaterialEntry>,
    /// The all-fallback entry, used for ranges whose material is out of range, and
    /// by every draw that shares the mesh shader without sampling a material (the
    /// skybox, the grid, the overlays).
    fallback_entry: MaterialEntry,
    /// What the entries were last synced to (`None` forces the first sync).
    synced: Option<MaterialKey>,
}

/// Everything the effective material table depends on: the material edits'
/// revision, the mode, and - in Unique mode only - how many mesh parts the
/// table is hued over.
///
/// The part count is what lets one table serve the Opt workspace's two meshes.
/// Both are synced every frame, and in Unique mode they have their own part
/// counts; keyed without it, the second mesh drew with the first one's table -
/// another object's hues, or the grey fallback past the end of it. In the other
/// modes the table does not depend on the mesh at all, so the part count is left
/// out and alternating between the two meshes rebuilds nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterialKey {
    revision: u64,
    mode: MaterialMode,
    parts: usize,
}

impl MaterialKey {
    pub(crate) fn new(revision: u64, mode: MaterialMode, part_count: usize) -> Self {
        Self {
            revision,
            mode,
            parts: if mode == MaterialMode::Unique {
                part_count
            } else {
                0
            },
        }
    }
}

impl MaterialTable {
    /// Build an empty table (fallback only), before any model is loaded.
    pub(crate) fn new() -> GpuResult<Self> {
        let fallback_textures = create_fallback_textures()?;
        let fallback_entry = MaterialEntry {
            uniform: MaterialUniform::fallback(),
            textures: fallback_textures.clone(),
        };
        Ok(Self {
            sampler: Sampler::aniso_repeat()?,
            fallback_textures,
            cache: HashMap::new(),
            tick: 0,
            entries: Vec::new(),
            fallback_entry,
            synced: None,
        })
    }

    /// Whether the entries already describe `key`, so the caller need not even
    /// build the effective table - which is a clone of every material, and a
    /// steady-state frame allocates nothing (invariant 3).
    pub(crate) fn is_current(&self, key: MaterialKey) -> bool {
        self.synced == Some(key)
    }

    /// Bring the table in line with `materials` (the effective table for `key`'s
    /// mode) when `key` changes: upload any newly-referenced texture once
    /// (deduplicated by path, re-uploaded on a disk reload), then build one entry per
    /// material and age out the uploads no longer referenced. A no-op when nothing
    /// changed.
    pub(crate) fn sync(&mut self, materials: &[MaterialState], key: MaterialKey) -> GpuResult<()> {
        if self.is_current(key) {
            return Ok(());
        }
        // One stamp for the whole sync: every entry this table references shares it,
        // which is what lets the eviction below tell "live" from "left over".
        self.tick += 1;
        let tick = self.tick;

        let mut entries = Vec::with_capacity(materials.len());
        for material in materials {
            let mut textures: [Arc<Texture>; TEXTURE_SLOT_COUNT] =
                std::array::from_fn(|slot| Arc::clone(&self.fallback_textures[slot]));
            for slot in TextureSlot::ALL {
                if let Some(binding) = &material.textures[slot.index()] {
                    let srgb = slot.is_srgb();
                    let key = (binding.path.clone(), srgb);
                    let identity = Arc::as_ptr(&binding.image) as usize;
                    match self.cache.get_mut(&key) {
                        Some(cached) if cached.identity == identity => cached.last_used = tick,
                        _ => {
                            let texture = Texture::rgba8_mipped_or_white(
                                binding.image.width,
                                binding.image.height,
                                &binding.image.rgba,
                                srgb,
                                c"material texture",
                            )?;
                            self.cache.insert(
                                key.clone(),
                                CachedTexture {
                                    identity,
                                    texture: Arc::new(texture),
                                    last_used: tick,
                                },
                            );
                        }
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
        self.synced = Some(key);
        self.evict_stale(tick);
        Ok(())
    }

    /// Drop least-recently-used uploads until at most [`MATERIAL_CACHE_CAP`] entries
    /// remain that this sync (`tick`) did not reference.
    ///
    /// Skipping the live ones is what makes the cap safe: a material table wider than
    /// the cap must still hold every texture it draws with, and one of them was just
    /// handed out by `Arc` above. Evicting only the leftovers keeps the bound on what
    /// no longer has a reader — a model swap, or a slot re-pointed at another file,
    /// both of which previously kept their old ~22 MB uploads for the session.
    fn evict_stale(&mut self, tick: u64) {
        let mut stale: Vec<(u64, (PathBuf, bool))> = self
            .cache
            .iter()
            .filter(|(_, cached)| cached.last_used != tick)
            .map(|(key, cached)| (cached.last_used, key.clone()))
            .collect();
        if stale.len() <= MATERIAL_CACHE_CAP {
            return;
        }
        stale.sort_unstable_by_key(|(last_used, _)| *last_used);
        for (_, key) in stale.drain(..stale.len() - MATERIAL_CACHE_CAP) {
            self.cache.remove(&key);
        }
    }

    /// The entry a draw range uses. `material` indexes the table; an out-of-range
    /// index (`u32::MAX` for un-materialed triangles) resolves to the fallback.
    pub(crate) fn entry(&self, material: u32) -> &MaterialEntry {
        self.entries
            .get(material as usize)
            .unwrap_or(&self.fallback_entry)
    }

    /// The all-fallback entry, for the draws that share the mesh shader without
    /// sampling a real material.
    pub(crate) fn fallback(&self) -> &MaterialEntry {
        &self.fallback_entry
    }

    /// An entry that shows `texture` flat ([`MaterialUniform::image`]): in the
    /// base-color slot and, with `alpha`, the opacity slot too, every other slot
    /// its fallback. The UV viewport draws its texture and its checker with one.
    pub(crate) fn image_entry(&self, texture: Arc<Texture>, alpha: bool) -> MaterialEntry {
        let mut textures = self.fallback_textures.clone();
        textures[TextureSlot::BaseColor as usize] = Arc::clone(&texture);
        if alpha {
            textures[TextureSlot::Opacity as usize] = texture;
        }
        MaterialEntry {
            uniform: MaterialUniform::image(alpha),
            textures,
        }
    }

    /// The anisotropic material sampler.
    pub(crate) fn sampler(&self) -> &Sampler {
        &self.sampler
    }
}

/// Create the per-slot neutral 1×1 fallback textures (white base/AO/opacity,
/// `[128,128,255]` normal, mid-grey roughness/metallic, black emissive) used for
/// any unassigned slot.
fn create_fallback_textures() -> GpuResult<[Arc<Texture>; TEXTURE_SLOT_COUNT]> {
    let make = |slot: TextureSlot| -> GpuResult<Arc<Texture>> {
        let pixel = fallback_pixel(slot);
        let format = if slot.is_srgb() {
            crate::rhi::Format::Rgba8Srgb
        } else {
            crate::rhi::Format::Rgba8
        };
        Ok(Arc::new(Texture::immutable_2d(
            &pixel,
            1,
            1,
            format,
            c"material fallback",
        )?))
    };
    // Destructured so the compiler proves one texture per slot (no fallible
    // `try_into` + dead panic arm).
    let [a, b, c, d, e, f, g] = TextureSlot::ALL;
    Ok([
        make(a)?,
        make(b)?,
        make(c)?,
        make(d)?,
        make(e)?,
        make(f)?,
        make(g)?,
    ])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The Opt workspace syncs its two meshes through the one table every frame:
    /// their part counts must tell Unique tables apart, and must not make the
    /// other modes rebuild on every alternation.
    #[test]
    fn only_a_unique_table_depends_on_the_part_count() {
        let key = |mode, parts| MaterialKey::new(7, mode, parts);
        assert_ne!(key(MaterialMode::Unique, 3), key(MaterialMode::Unique, 5));
        assert_eq!(key(MaterialMode::Source, 3), key(MaterialMode::Source, 5));
        assert_eq!(
            key(MaterialMode::Standard, 3),
            key(MaterialMode::Standard, 5)
        );
        assert_ne!(
            MaterialKey::new(7, MaterialMode::Source, 0),
            MaterialKey::new(8, MaterialMode::Source, 0)
        );
    }
}
