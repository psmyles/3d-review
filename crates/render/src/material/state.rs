//! CPU + GPU per-material data types: the editable [`MaterialState`] (seeded from
//! import defaults, edited via [`MaterialEdit`] intents), its alpha / workflow
//! modes and texture bindings, the app→UI [`MaterialSnapshot`], the per-material
//! [`MaterialDrawRange`], and the `#[repr(C)]` [`MaterialUniform`] that must match
//! the HLSL per-material cbuffer (`b1`) in `scene.hlsl` exactly (invariant 11).

use std::path::PathBuf;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::texture::{ChannelSelect, DecodedImage, TEXTURE_SLOT_COUNT};

/// One per-material draw: a contiguous run of the reordered mesh index buffer
/// whose triangles share a single material slot. `material` indexes the table, or
/// `u32::MAX` for triangles that carried no material (drawn with the fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaterialDrawRange {
    pub material: u32,
    pub first_index: u32,
    pub index_count: u32,
}

/// How a material's alpha is composited. Driven by an assigned opacity map (or the
/// base color alpha): `Blend` straight-alpha blends, `Clip` does a hard cutout at
/// [`MaterialState::alpha_cutoff`]. `Opaque` ignores alpha entirely.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Blend,
    Clip,
}

/// How a material's roughness is authored — the Inspector's **Workflow** dropdown.
/// `Roughness` is the native metallic-roughness convention (the value/map *is*
/// roughness). `Smoothness` is the Unity-style inverse: the slider reads as
/// smoothness and a bound map is a smoothness map (an inverted roughness map), so
/// the shader inverts the sampled value before using it as roughness. Internally
/// the material always stores roughness; the workflow only changes how it is
/// displayed and how a bound map is interpreted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoughnessWorkflow {
    #[default]
    Roughness,
    Smoothness,
}

impl RoughnessWorkflow {
    pub const ALL: [RoughnessWorkflow; 2] =
        [RoughnessWorkflow::Roughness, RoughnessWorkflow::Smoothness];

    /// Value the shader branches on (`flags.x`): 0 roughness, 1 smoothness (invert
    /// the bound map).
    fn shader_value(self) -> f32 {
        match self {
            RoughnessWorkflow::Roughness => 0.0,
            RoughnessWorkflow::Smoothness => 1.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RoughnessWorkflow::Roughness => "Roughness",
            RoughnessWorkflow::Smoothness => "Smoothness",
        }
    }
}

impl AlphaMode {
    pub const ALL: [AlphaMode; 3] = [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Clip];

    /// Value the shader branches on (`params.w`).
    fn shader_value(self) -> f32 {
        match self {
            AlphaMode::Opaque => 0.0,
            AlphaMode::Blend => 1.0,
            AlphaMode::Clip => 2.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AlphaMode::Opaque => "Opaque",
            AlphaMode::Blend => "Blend",
            AlphaMode::Clip => "Clip",
        }
    }
}

/// One assigned texture slot: the source path (cache + watcher key), the decoded
/// RGBA8 pixels shared by `Arc` (so the per-frame callback clones a refcount, not
/// megabytes), and which channel(s) feed the property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureBinding {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    pub channel: ChannelSelect,
}

/// Editable per-material PBR parameters — app-side state seeded from the import
/// defaults. `base_color` and `emissive` are linear RGB. The UI edits the scalars
/// via [`MaterialEdit`] intents and assigns the [`TextureBinding`]s via app-side
/// decode; the renderer uploads them into the GPU material table when the material
/// revision changes.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialState {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// The seven texture slots, indexed by [`TextureSlot::index`]. `None` slots use
    /// the shader's neutral per-slot fallback.
    pub textures: [Option<TextureBinding>; TEXTURE_SLOT_COUNT],
    pub alpha_mode: AlphaMode,
    /// Alpha-clip threshold in `0.0..=1.0` (used only in [`AlphaMode::Clip`]).
    pub alpha_cutoff: f32,
    /// How `roughness` (and a bound roughness map) is authored / displayed. Storage
    /// is always roughness; [`RoughnessWorkflow::Smoothness`] only flips the UI and
    /// inverts a bound map in the shader.
    pub workflow: RoughnessWorkflow,
}

impl Default for MaterialState {
    fn default() -> Self {
        Self {
            base_color: Vec3::ONE,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
            textures: Default::default(),
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            workflow: RoughnessWorkflow::Roughness,
        }
    }
}

/// A named editable material for the app→UI snapshot (invariant 2: a plain value
/// the UI reads, never a handle into renderer state).
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSnapshot {
    pub name: String,
    pub state: MaterialState,
}

/// Which scalar/color/routing parameter a material edit changes. Colors are linear
/// RGB. Texture *assignment* (which needs a decoded image) flows separately
/// through the app-side decode path, not this enum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaterialChange {
    BaseColor([f32; 3]),
    Metallic(f32),
    Roughness(f32),
    Emissive([f32; 3]),
    /// Re-route an already-assigned slot's channel (the Inspector dropdown). The
    /// `usize` is the [`TextureSlot::index`].
    Channel(usize, ChannelSelect),
    AlphaMode(AlphaMode),
    AlphaCutoff(f32),
    /// The roughness/smoothness authoring workflow (display + map interpretation).
    Workflow(RoughnessWorkflow),
}

/// A UI edit intent: change one parameter of the material at `index`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialEdit {
    pub index: usize,
    pub change: MaterialChange,
}

/// GPU-side per-material uniform (cbuffer `b1` in `scene.hlsl`). `#[repr(C)]` +
/// `Pod` to match that HLSL cbuffer layout exactly (invariant 11).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct MaterialUniform {
    pub base_color: [f32; 4],
    pub emissive: [f32; 4],
    /// `x` metallic, `y` roughness, `z` slot_flags bitfield (bit `i` = slot `i`
    /// bound), `w` alpha mode (0 opaque, 1 blend, 2 clip).
    pub params: [f32; 4],
    /// Channel-select index (0 R … 3 A) for slots 0,1,2,3.
    pub channels0: [f32; 4],
    /// Channel-select index for slots 4,5,6 in `x,y,z`; `w` = alpha cutoff.
    pub channels1: [f32; 4],
    /// Material flags. `x` = roughness workflow (0 roughness, 1 smoothness: invert
    /// the bound roughness map); `y,z,w` reserved.
    pub flags: [f32; 4],
}

// Byte-size lock against `scene.hlsl`'s `b1` (invariant 11). The cbuffer is sized
// from `size_of::<T>()` and an upload is rejected only when it is *larger* than
// the buffer, so a field added on one side alone grows both and uploads happily
// while the shader keeps reading the old offsets — wrong pixels, not an error.
const _: () = assert!(std::mem::size_of::<MaterialUniform>() == 96);
const _: () = assert!(
    std::mem::size_of::<MaterialUniform>()
        == std::mem::size_of::<crate::shaders::generated::Material>()
);

impl MaterialUniform {
    pub(super) fn from_state(state: &MaterialState) -> Self {
        let mut slot_flags = 0u32;
        let mut channel = [0.0f32; TEXTURE_SLOT_COUNT];
        for (index, binding) in state.textures.iter().enumerate() {
            if let Some(binding) = binding {
                slot_flags |= 1 << index;
                channel[index] = binding.channel.shader_index();
            }
        }
        Self {
            base_color: [
                state.base_color.x,
                state.base_color.y,
                state.base_color.z,
                1.0,
            ],
            emissive: [state.emissive.x, state.emissive.y, state.emissive.z, 0.0],
            params: [
                state.metallic,
                state.roughness,
                slot_flags as f32,
                state.alpha_mode.shader_value(),
            ],
            channels0: [channel[0], channel[1], channel[2], channel[3]],
            channels1: [
                channel[4],
                channel[5],
                channel[6],
                state.alpha_cutoff.clamp(0.0, 1.0),
            ],
            flags: [state.workflow.shader_value(), 0.0, 0.0, 0.0],
        }
    }

    /// Neutral dielectric grey, bound for unmaterialed triangles, overlays, the
    /// grid, skybox and UV draws (none of which sample it). No texture slots bound.
    pub(super) fn fallback() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8, 1.0],
            emissive: [0.0; 4],
            params: [0.0, 0.5, 0.0, 0.0],
            channels0: [0.0; 4],
            channels1: [0.0; 4],
            flags: [0.0; 4],
        }
    }
}
