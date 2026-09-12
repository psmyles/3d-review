//! The vendored-ufbx side of the importer: the C bridge, and the marshalling
//! that turns what it writes into `review_model` types.
//!
//! The whole module is `#[cfg(has_ufbx)]` — without `third_party/ufbx` the crate
//! still builds and import reports [`crate::ImportError::UfbxUnavailable`].
//!
//! Layered so the `unsafe` is a leaf rather than a theme (invariant 9):
//!
//! * [`raw_scene`] / [`raw_extras`] — the `#[repr(C)]` mirrors. Data only.
//! * [`bridge`] — the one call into C, and the free protocol around it.
//! * [`raw`] — pointer to slice, pointer to string. The last of the `unsafe`.
//! * [`marshal_model`] / [`marshal_extras`] — safe code over checked slices.
//!
//! The two marshalling modules are the majority of this crate by line count and
//! contain no `unsafe` at all, which is what the `deny` at the top of each says.

mod bridge;
mod marshal_extras;
mod marshal_model;
mod raw;
mod raw_extras;
mod raw_scene;

pub(crate) use bridge::{ExtrasHandle, load_fbx};
