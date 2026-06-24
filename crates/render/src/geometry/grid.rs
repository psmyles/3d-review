//! The static reference grids: the 3D world-space floor + colored axes, and the
//! 2D UV viewport's 0..1 unit-square grid. Neither depends on the model.

use crate::scene::SceneVertex;

use super::vertex::push_line;

/// The static reference grid + colored X/Z axis lines, in world space. A 2 m
/// square floor (`±GRID_HALF_EXTENT`) ruled in 10 cm cells, with a stronger line
/// every 0.5 m. Iterates integer cell indices to avoid float drift.
pub(crate) fn scene_lines() -> Vec<SceneVertex> {
    let mut vertices = Vec::new();
    let extent = crate::GRID_HALF_EXTENT;
    // 10 cm cells across the half-extent, so ±10 lines for a ±1 m grid.
    const CELL: f32 = 0.1;
    let lines_per_half = (extent / CELL).round() as i32;

    for line in -lines_per_half..=lines_per_half {
        let coord = line as f32 * CELL;
        // Emphasize every 0.5 m (every 5th 10 cm line).
        let strong = line % 5 == 0;
        let color = if strong {
            [0.42, 0.49, 0.54, 0.46]
        } else {
            [0.33, 0.38, 0.42, 0.28]
        };
        push_line(
            &mut vertices,
            [coord, 0.0, -extent],
            [coord, 0.0, extent],
            color,
        );
        push_line(
            &mut vertices,
            [-extent, 0.0, coord],
            [extent, 0.0, coord],
            color,
        );
    }

    push_line(
        &mut vertices,
        [-extent, 0.002, 0.0],
        [extent, 0.002, 0.0],
        [0.94, 0.23, 0.28, 1.0],
    );
    push_line(
        &mut vertices,
        [0.0, 0.004, -extent],
        [0.0, 0.004, extent],
        [0.18, 0.53, 1.0, 1.0],
    );
    vertices
}

/// Color of the UV unit-square border (the 0..1 outline), gamma-space (the line
/// shader returns the vertex color directly into the gamma framebuffer).
const UV_GRID_BORDER_COLOR: [f32; 4] = [0.42, 0.49, 0.54, 0.7];
/// Color of the interior UV grid subdivisions (every 0.1 of the unit square).
const UV_GRID_CELL_COLOR: [f32; 4] = [0.33, 0.38, 0.42, 0.32];

/// The 0..1 reference grid for the UV viewport, in UV-plane coordinates
/// (positions are `(u, v, 0)`). Ten subdivisions per axis plus a stronger
/// border at 0 and 1, matching the look of the 3D floor grid.
pub(crate) fn uv_grid_lines() -> Vec<SceneVertex> {
    const CELLS: i32 = 10;
    let mut vertices = Vec::with_capacity(((CELLS + 1) * 2 * 2) as usize);

    for line in 0..=CELLS {
        let coord = line as f32 / CELLS as f32;
        // The 0 and 1 lines form the unit-square border; the rest are cells.
        let color = if line == 0 || line == CELLS {
            UV_GRID_BORDER_COLOR
        } else {
            UV_GRID_CELL_COLOR
        };
        // Vertical line (constant u) and horizontal line (constant v).
        push_line(&mut vertices, [coord, 0.0, 0.0], [coord, 1.0, 0.0], color);
        push_line(&mut vertices, [0.0, coord, 0.0], [1.0, coord, 0.0], color);
    }

    vertices
}
