//! The indexed-colour view's CPU geometry: a vertex buffer parallel to the
//! mesh's own (as the heat maps' is), each corner coloured by the palette entry
//! its vertex colour channel names.
//!
//! A game that reads a vertex colour channel as an index into its palette
//! stores index ÷ 255 there; this shows what the game would draw. A value that
//! is no whole index, or an index past the palette's end, is drawn in
//! [`NOT_AN_INDEX`].

use review_model::ModelData;
use review_model::color::{linear_to_srgb, srgb_to_linear};

use crate::IndexedColor;
use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::vertex::push_shaded_vertex;

/// What a corner whose value is not an index is drawn in: the magenta that
/// reads as "missing" everywhere.
pub const NOT_AN_INDEX: [f32; 3] = [1.0, 0.0, 1.0];

/// The index a stored channel value holds, when it holds one: the nearest whole
/// index, kept if storing it again lands within a 16-bit step of the value —
/// the arithmetic of the audit's indexed-colour check. With `srgb`, the index
/// was painted as an sRGB colour and stored linear.
pub fn palette_index(value: f32, srgb: bool) -> Option<u8> {
    if !(0.0..=1.0).contains(&value) {
        return None;
    }
    let encoded = if srgb { linear_to_srgb(value) } else { value };
    let index = (encoded * 255.0).round();
    let exact = index / 255.0;
    let stored = if srgb { srgb_to_linear(exact) } else { exact };
    ((stored - value).abs() <= 1.0 / 65535.0 + 1.0e-7).then_some(index as u8)
}

/// Entry `index` of the built-in palette, for a model with no palette of its
/// own: hues a golden angle apart, so neighbouring indices are far apart in
/// colour, at two lightnesses alternating, and none in the magenta band
/// [`NOT_AN_INDEX`] is drawn in. Index 0 is a dark grey, as an unset channel
/// usually is.
pub fn default_palette_color(index: u8) -> [f32; 3] {
    /// The hues, in degrees, left to [`NOT_AN_INDEX`].
    const MAGENTA: std::ops::Range<f32> = 270.0..330.0;
    if index == 0 {
        return [0.2, 0.2, 0.2];
    }
    let mut degrees = (f32::from(index) * 137.507_77).rem_euclid(360.0);
    // Squeeze the full circle into the hues outside the magenta band.
    let span = MAGENTA.end - MAGENTA.start;
    degrees *= (360.0 - span) / 360.0;
    if degrees >= MAGENTA.start {
        degrees += span;
    }
    let hue = degrees.rem_euclid(360.0) / 60.0;
    let (saturation, value) = if index.is_multiple_of(2) {
        (0.75, 0.95)
    } else {
        (0.6, 0.7)
    };
    let chroma = value * saturation;
    let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let base = value - chroma;
    [r + base, g + base, b + base]
}

/// Build the indexed-colour vertex buffer: the mesh's own positions and normals,
/// each corner in its palette entry (gamma rgb, fully replacing the shaded
/// base). An empty `palette` is the built-in one.
pub(crate) fn indexed_color_vertices(
    model: &ModelData,
    lanes: &[[u32; 4]],
    view: IndexedColor,
    palette: &[[f32; 3]],
) -> Vec<SceneVertex> {
    let channel = usize::from(view.channel.min(3));
    let mut vertices = Vec::with_capacity(model.vertices.len());
    for (index, vertex) in model.vertices.iter().enumerate() {
        let rgb = match palette_index(vertex.vertex_color[channel], view.srgb) {
            Some(entry) if palette.is_empty() => default_palette_color(entry),
            Some(entry) => palette
                .get(usize::from(entry))
                .copied()
                .unwrap_or(NOT_AN_INDEX),
            None => NOT_AN_INDEX,
        };
        push_shaded_vertex(
            &mut vertices,
            vertex.position.to_array(),
            vertex.normal.to_array(),
            [rgb[0], rgb[1], rgb[2], 1.0],
            corner_deform(lanes, index),
        );
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_index_reads_back_and_a_blend_does_not() {
        assert_eq!(palette_index(3.0 / 255.0, false), Some(3));
        assert_eq!(palette_index(0.5, false), None);
        // 16 bits of index 3: within a step.
        let stored = (3.0f32 / 255.0 * 65535.0).round() / 65535.0;
        assert_eq!(palette_index(stored, false), Some(3));
        // Index 1 painted in sRGB, stored linear in 16 bits.
        let painted = (srgb_to_linear(1.0 / 255.0) * 65535.0).round() / 65535.0;
        assert_eq!(palette_index(painted, true), Some(1));
        assert_eq!(palette_index(painted, false), None);
        assert_eq!(palette_index(1.0, false), Some(255));
    }

    #[test]
    fn nothing_in_the_built_in_palette_looks_like_not_an_index() {
        for index in 0..=255u8 {
            let color = default_palette_color(index);
            let distance: f32 = (0..3).map(|k| (color[k] - NOT_AN_INDEX[k]).abs()).sum();
            assert!(distance > 0.6, "{index}: {color:?}");
        }
    }

    #[test]
    fn the_built_in_palette_tells_neighbours_apart() {
        for index in 1..=254u8 {
            let [a, b] = [
                default_palette_color(index),
                default_palette_color(index + 1),
            ];
            let distance: f32 = (0..3).map(|k| (a[k] - b[k]).abs()).sum();
            assert!(distance > 0.15, "{index}: {a:?} {b:?}");
        }
    }
}
