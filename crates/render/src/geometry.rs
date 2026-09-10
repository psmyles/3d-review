//! CPU-side vertex generation for the scene's static grid and the derived debug
//! views (wireframe, face/vertex normal lines).
//!
//! These index *into* the shared [`ModelData`] buffers (invariant 1) and produce
//! flat [`SceneVertex`] arrays for upload; they never copy or re-own geometry.
//! Each builder takes its visual parameters explicitly so the renderer can
//! rebuild a single view live when its slider/color changes.
//!
//! Organized by purpose: [`vertex`] holds the shared emit helpers; [`hidden`] the
//! Outliner hidden-mesh filter every visibility-aware builder resolves through;
//! [`grid`] the static reference grids; [`mesh`] the shaded mesh + per-material
//! reorder; [`select`] the selection/visibility draw lists; [`debug_lines`] the 3D
//! debug line views; [`skeleton`] the bone overlay; [`skin`] the weight heat map;
//! [`uv`] the UV viewport geometry.
//!
//! [`ModelData`]: review_model::ModelData

mod debug_lines;
pub(crate) mod deform;
mod grid;
mod hidden;
mod mesh;
mod select;
mod skeleton;
mod skin;
mod uv;
mod vertex;

pub(crate) use debug_lines::{
    bounding_box_lines, face_normal_lines, model_pivot, pivot_half_extent, pivot_lines,
    uv_seam_lines, vertex_normal_lines, wireframe_edge_indices,
};
pub(crate) use grid::{scene_lines, uv_grid_lines};
pub(crate) use mesh::model_mesh;
pub(crate) use select::{selected_triangle_mask, selection_geometry, visible_geometry};
pub(crate) use skeleton::{skeleton_fill_triangles, skeleton_lines};
pub(crate) use skin::skin_weight_vertices;
pub(crate) use uv::{uv_fill_triangles, uv_wireframe_lines};
