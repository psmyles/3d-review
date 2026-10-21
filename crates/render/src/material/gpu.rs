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
use std::sync::{Arc, Weak};

use crate::config::MaterialMode;
use crate::rhi::{Bindings, GpuResult, Sampler, Texture};
use crate::shaders::generated;
use crate::texture::{DecodedImage, TEXTURE_SLOT_COUNT, TextureSlot};

use super::mode::unmaterialed_material;
use super::state::{MaterialState, MaterialUniform, TextureBinding};

/// How many uploads the cache keeps beyond what the *current* material table
/// references — the same LRU bound `TexGpu` puts on the Tex viewport's cache, for
/// the same reason: a mipped 4K RGBA8 upload is ~22 MB, and without a bound a
/// session that re-points one slot at a series of candidate files grows VRAM for
/// as long as it runs.
///
/// Entries the current sync touched are never evicted, so a table wider than this
/// still gets every one of its textures; the cap governs only how much of the
/// *previous* table is kept warm — what a Standard or Unique round trip comes back
/// to, or a slot re-pointed back at the file it had. Only while `app` still holds
/// the decoded image, though: a model load clears its decode cache and decodes
/// every map afresh, so the last model's uploads can never be matched again, and
/// they go at the next sync rather than sitting in VRAM ([`MaterialTable::sync`]).
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

/// One cached GPU upload, the decoded image it was made from, and the sync that
/// last referenced it (the LRU stamp).
///
/// The image is what says whether the upload is still the file's: a disk reload
/// swaps a fresh `Arc` in at the same path, which must re-upload. It is held as a
/// `Weak` rather than as the `Arc`'s address alone. An address is not an
/// identity once its allocation is freed — the next decode at that path can land
/// on it and be shown the stale upload — and a `Weak` keeps the allocation, so
/// the address cannot be reused while the entry exists, without keeping the
/// pixels: those go the moment `app` drops the image, and the entry with them
/// ([`MaterialTable::evict_stale`]), since nothing can ever match it again.
struct CachedTexture {
    image: Weak<DecodedImage>,
    texture: Arc<Texture>,
    last_used: u64,
}

impl CachedTexture {
    /// Whether this upload was made from `image` — the very same decode.
    fn is_of(&self, image: &Arc<DecodedImage>) -> bool {
        same_decode(&self.image, image)
    }
}

/// Whether `held` and `image` are one decode. The address comparison is sound
/// only because `held` keeps the allocation it points at: no other image can be
/// placed there while it exists.
fn same_decode(held: &Weak<DecodedImage>, image: &Arc<DecodedImage>) -> bool {
    std::ptr::eq(held.as_ptr(), Arc::as_ptr(image))
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
    /// The all-fallback entry, used by every draw that shares the mesh shader
    /// without sampling a material (the skybox, the grid, the overlays).
    fallback_entry: MaterialEntry,
    /// What a range whose material is out of range draws with — the triangles
    /// import tagged as having no material (`u32::MAX`). The neutral fallback in
    /// Source mode, and the standard grey every other entry has in Standard and
    /// Unique, which is what those modes promise for the whole model.
    unmaterialed_entry: MaterialEntry,
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
        let plain = |uniform| MaterialEntry {
            uniform,
            textures: fallback_textures.clone(),
        };
        let fallback_entry = plain(MaterialUniform::fallback());
        let unmaterialed_entry = plain(MaterialUniform::fallback());
        Ok(Self {
            sampler: Sampler::aniso_repeat()?,
            fallback_textures,
            cache: HashMap::new(),
            tick: 0,
            entries: Vec::new(),
            fallback_entry,
            unmaterialed_entry,
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
    ///
    /// An upload the device refuses does not fail the sync. It used to, before the
    /// table was marked synced or the upload cached, so the render returned the
    /// error and the next frame rebuilt the whole mip chain to fail again — a dead
    /// viewport that cost a full decode's worth of work per redraw. The slot draws
    /// its neutral fallback instead, the warning is logged once, and the failure is
    /// cached against that decoded image like any upload, so it is retried only
    /// when the image changes (a reload, or another file).
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
                    textures[slot.index()] = self.upload(slot, binding, tick);
                }
            }
            entries.push(MaterialEntry {
                uniform: MaterialUniform::from_state(material),
                textures,
            });
        }
        self.entries = entries;
        self.unmaterialed_entry = MaterialEntry {
            uniform: unmaterialed_material(key.mode)
                .map_or_else(MaterialUniform::fallback, |state| {
                    MaterialUniform::from_state(&state)
                }),
            textures: self.fallback_textures.clone(),
        };
        self.synced = Some(key);
        self.evict_stale(tick);
        Ok(())
    }

    /// The texture `binding` draws with in `slot`: the cached upload of its decoded
    /// image, or a new one, stamped live for this sync (`tick`). A refused upload
    /// is the slot's fallback, cached in its place (see [`Self::sync`]).
    fn upload(&mut self, slot: TextureSlot, binding: &TextureBinding, tick: u64) -> Arc<Texture> {
        let srgb = slot.is_srgb();
        let key = (binding.path.clone(), srgb);
        if let Some(cached) = self.cache.get_mut(&key)
            && cached.is_of(&binding.image)
        {
            cached.last_used = tick;
            return Arc::clone(&cached.texture);
        }
        let texture = match Texture::rgba8_mipped_or_white(
            binding.image.width,
            binding.image.height,
            &binding.image.rgba,
            srgb,
            c"material texture",
        ) {
            Ok(texture) => Arc::new(texture),
            Err(error) => {
                log::warn!(
                    "{} could not be uploaded, so its {} slot draws without it: {error}",
                    binding.path.display(),
                    slot.label(),
                );
                Arc::clone(&self.fallback_textures[slot.index()])
            }
        };
        self.cache.insert(
            key,
            CachedTexture {
                image: Arc::downgrade(&binding.image),
                texture: Arc::clone(&texture),
                last_used: tick,
            },
        );
        texture
    }

    /// Drop every upload whose decoded image is gone, then least-recently-used
    /// ones until at most [`MATERIAL_CACHE_CAP`] entries remain that this sync
    /// (`tick`) did not reference.
    ///
    /// An upload whose image is gone can never be matched again — `app` makes a
    /// fresh image for every decode — so keeping it would only hold its VRAM; a
    /// model load, which clears `app`'s decode cache, retires the last model's
    /// whole set here. Skipping the live ones is what makes the cap safe: a
    /// material table wider than the cap must still hold every texture it draws
    /// with, and one of them was just handed out by `Arc` above. Evicting only the
    /// leftovers keeps the bound on what no longer has a reader — a slot
    /// re-pointed at another file, which previously kept its old ~22 MB upload
    /// for the session.
    fn evict_stale(&mut self, tick: u64) {
        self.cache
            .retain(|_, cached| cached.last_used == tick || cached.image.strong_count() > 0);
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
    /// index (`u32::MAX` for un-materialed triangles) resolves to what the mode
    /// draws a triangle with no material as — the neutral fallback in Source mode,
    /// the standard grey in Standard and Unique.
    pub(crate) fn entry(&self, material: u32) -> &MaterialEntry {
        self.entries
            .get(material as usize)
            .unwrap_or(&self.unmaterialed_entry)
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

    fn decoded() -> Arc<DecodedImage> {
        Arc::new(DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![9, 9, 9, 255],
            source_channels: 4,
            source_bit_depth: 8,
        })
    }

    /// An upload matches the decode it was made from and nothing else: not a
    /// fresh decode of the same pixels (what a reload, or the next model's load,
    /// hands over), and the held reference frees the pixels with the last reader
    /// while still keeping the address from being given to another image — which
    /// is what an address-only key could not do.
    #[test]
    fn an_upload_matches_only_its_own_decode() {
        let image = decoded();
        let held = Arc::downgrade(&image);
        assert!(same_decode(&held, &image));

        let reloaded = decoded();
        assert_eq!(*reloaded, *image);
        assert!(!same_decode(&held, &reloaded));

        drop(image);
        // Gone, so the eviction retires it; and no decode after it can match.
        assert_eq!(held.strong_count(), 0);
        let next = decoded();
        assert!(!same_decode(&held, &next));
    }
}
