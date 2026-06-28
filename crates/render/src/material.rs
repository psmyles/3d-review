//! Editable per-material state + the GPU material table (bind group 3).
//!
//! Phase 1 seeded the renderer with an editable table of per-material PBR
//! parameters drawn one index range per material ([`MaterialDrawRange`]). Phase 3
//! extends each material with **seven texture slots** (base color / normal /
//! roughness / metallic / AO / emissive / opacity) plus **packed-channel routing**:
//! one decoded image (deduplicated by path) can feed several scalar properties via
//! per-property channel selectors carried in the uniform. The GPU side keeps a
//! path-keyed texture cache so a packed map is uploaded once and shared across the
//! materials/slots that reference it; reassigning a slot rebuilds only the bind
//! groups, never the geometry. The UI edits parameters via [`MaterialEdit`] intents
//! and assigns textures via app-side decode (invariant 2).
//!
//! Organized into [`state`] (CPU/GPU data types incl. the `#[repr(C)]`
//! `MaterialUniform`), [`mode`] (the effective-table / Unique-part grouping) and
//! [`d3d`] (the Direct3D 11 material table: cbuffer `b1` + the seven texture slots
//! `t5..t11` + a path-keyed upload cache).

mod d3d;
mod mode;
mod state;

pub(crate) use d3d::MaterialTableD3d;
pub(crate) use mode::{build_part_key, effective_materials};
pub(crate) use state::MaterialDrawRange;
pub use state::{
    AlphaMode, MaterialChange, MaterialEdit, MaterialSnapshot, MaterialState, RoughnessWorkflow,
    TextureBinding,
};
