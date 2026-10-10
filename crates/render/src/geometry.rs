//! CPU-side vertex generation for the scene's static grid and the derived debug
//! views (wireframe, face/vertex normal lines).
//!
//! These index *into* the shared [`ModelData`] buffers (invariant 1) and produce
//! flat [`SceneVertex`](crate::scene::SceneVertex) arrays for upload; they never
//! copy or re-own geometry. Each builder takes its visual parameters explicitly
//! so the renderer can rebuild a single view live when its slider/color changes.
//!
//! Organized by purpose: [`vertex`] holds the shared emit helpers; [`hidden`] the
//! Outliner hidden-mesh filter every visibility-aware builder resolves through;
//! [`grid`] the static reference grids; [`mesh`] the shaded mesh + per-material
//! reorder; [`select`] the selection/visibility draw lists; [`wireframe`] the
//! model wireframe; [`markers`] the pivot and bounding box; [`normal_lines`] the
//! face and vertex normals; [`uv_seams`] the seam view; [`skeleton`] the bone
//! overlay; [`skin`] the weight heat map; [`uv`] the UV viewport geometry.
//!
//! [`ModelData`]: review_model::ModelData

mod audit;
pub(crate) mod deform;
mod grid;
mod hidden;
mod markers;
mod mesh;
mod normal_lines;
mod select;
mod skeleton;
mod skin;
mod uv;
mod uv_seams;
mod vertex;
mod wireframe;

pub(crate) use audit::audit_lines;
pub(crate) use grid::{scene_lines, uv_grid_lines};
pub(crate) use markers::{bounding_box_lines, model_pivot, pivot_half_extent, pivot_lines};
pub(crate) use mesh::{group_triangles, model_mesh};
pub(crate) use normal_lines::{face_normal_lines, vertex_normal_lines};
pub(crate) use select::{selected_triangle_mask, selection_geometry, visible_geometry};
pub use skeleton::{BONE_PICK_TOLERANCE_POINTS, pick_bone_shape, posed_joint_positions};
pub(crate) use skeleton::{BoneTint, skeleton_fill_triangles, skeleton_lines};
pub(crate) use skin::skin_weight_vertices;
pub(crate) use uv::{UvNodeScope, hsv_to_rgb, uv_fill_triangles, uv_wireframe_lines};
pub(crate) use uv_seams::uv_seam_lines;
pub(crate) use wireframe::wireframe_edge_indices;
