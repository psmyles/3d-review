//! Editable per-material state + the GPU material table (the `material` uniform
//! block and texture bindings 5..11).
//!
//! Each material is an editable set of PBR parameters, drawn one index range per
//! material ([`MaterialDrawRange`]), with **seven texture slots** (base color / normal /
//! roughness / metallic / AO / emissive / opacity) plus **packed-channel routing**:
//! one decoded image (deduplicated by path) can feed several scalar properties via
//! per-property channel selectors carried in the uniform. The GPU side keeps a
//! path-keyed texture cache so a packed map is uploaded once and shared across the
//! materials/slots that reference it; reassigning a slot re-resolves only that
//! material's textures, never the geometry. The UI edits parameters via
//! [`MaterialEdit`] intents and assigns textures via app-side decode (invariant 2).
//!
//! Organized into [`state`] (CPU/GPU data types incl. the `#[repr(C)]`
//! `MaterialUniform`), [`mode`] (the effective-table / Unique-part grouping) and
//! [`gpu`] (the uploaded table: one entry per material, its uniform and its seven
//! texture slots, over a path-keyed LRU upload cache).

mod gpu;
mod mode;
mod state;

pub(crate) use gpu::{MaterialEntry, MaterialKey, MaterialTable};
pub(crate) use mode::{build_part_key, effective_materials};
pub(crate) use state::MaterialDrawRange;
pub use state::{
    AlphaMode, MaterialChange, MaterialEdit, MaterialSnapshot, MaterialState, RoughnessWorkflow,
    TextureBinding,
};
