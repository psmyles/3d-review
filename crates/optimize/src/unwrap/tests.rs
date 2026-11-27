//! The pieces of the unwrap that are pure maths.

use glam::{Vec2, Vec3};

use super::charts::Chart;
use super::layout::{lay_out, stretch};
use crate::stack::{UnwrapParams, UvLayout};

/// A tiled chart spanning two pieces of its reference UVs — a region of two
/// CAD faces, each with parameters of its own — stays one piece, lying as its
/// larger face's reference did. Before, both reference pieces were fitted at
/// once (a compromise angle), and the reference, unstretched on every
/// triangle, was "kept" in its place and cut the chart back into the faces.
#[test]
fn a_chart_over_two_reference_pieces_lies_as_the_larger_and_stays_whole() {
    // Two rectangles side by side in the plane: A is 2 x 1, B is 1 x 1, with
    // vertices of their own along the edge they share (x = 2).
    let positions: Vec<Vec3> = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (0.0, 1.0),
        (2.0, 0.0),
        (3.0, 0.0),
        (3.0, 1.0),
        (2.0, 1.0),
    ]
    .map(|(x, y)| Vec3::new(x, y, 0.0))
    .to_vec();
    let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    // A's reference runs along x; B's is a quarter turn round and elsewhere.
    let reference: Vec<Vec2> = positions
        .iter()
        .enumerate()
        .map(|(vertex, p)| {
            if vertex < 4 {
                Vec2::new(p.x, p.y)
            } else {
                Vec2::new(10.0 + p.y, 5.0 - (p.x - 2.0))
            }
        })
        .collect();
    // The chart as flattened: the plane itself, at 30 degrees.
    let turn = Vec2::from_angle(30f32.to_radians());
    let chart = Chart {
        triangles: vec![0, 1, 2, 3],
        uvs: indices
            .chunks(3)
            .map(|corners| {
                [0, 1, 2].map(|k| {
                    let p = positions[corners[k] as usize];
                    turn.rotate(Vec2::new(p.x, p.y))
                })
            })
            .collect(),
    };
    let params = UnwrapParams {
        layout: UvLayout::Tiled,
        align_to_reference: true,
        keep_better: true,
        density: 1.0,
        ..UnwrapParams::default()
    };
    let uv = lay_out(&[chart], 4, &positions, &indices, Some(&reference), &params)
        .expect("a tiled layout");

    // Whole: A's and B's corners at x = 2 have one UV each.
    let corner_of = |vertex: u32| indices.iter().position(|&at| at == vertex).unwrap();
    for (a, b) in [(1, 4), (2, 7)] {
        assert!(
            uv[corner_of(a)].distance(uv[corner_of(b)]) < 1e-4,
            "the chart was cut at x = 2: {} against {}",
            uv[corner_of(a)],
            uv[corner_of(b)]
        );
    }
    // A lies exactly as its reference: the same up to a shift.
    let shift = uv[0] - reference[0];
    for corner in 0..6 {
        let expected = reference[indices[corner] as usize] + shift;
        assert!(
            uv[corner].distance(expected) < 1e-4,
            "A is not turned to its reference: {} against {expected}",
            uv[corner]
        );
    }
}

#[test]
fn a_scaled_copy_does_not_stretch_and_a_squash_does() {
    let points = [
        Vec3::ZERO,
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    let copy = [Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(0.0, 2.0)];
    assert!((stretch(points, copy).unwrap() - 1.0).abs() < 1e-5);
    let squashed = [Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(0.0, 1.0)];
    assert!((stretch(points, squashed).unwrap() - 2.0).abs() < 1e-5);
    let flat = [Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(8.0, 0.0)];
    assert_eq!(
        stretch(points, flat),
        Some(50.0),
        "a collapsed triangle is capped"
    );
    assert_eq!(stretch([Vec3::ZERO; 3], copy), None);
}

/// The packing may turn a chart to fit it, a quarter turn at a time; turned,
/// it still winds the way it was flattened (patch P5: upstream transposed it,
/// a mirror).
#[test]
fn packing_turns_charts_without_mirroring_them() {
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut chart: Vec<u32> = Vec::new();
    // Long strips of different sizes, laid along U: the packing turns some.
    for at in 0..24u32 {
        let base = uvs.len() as u32;
        let (width, height) = (20.0 + (at * 7 % 50) as f32, 2.0 + (at % 4) as f32);
        uvs.extend([[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        chart.extend([at, at]);
    }
    let options = crate::xatlas::PackOptions {
        resolution: 0,
        texels_per_unit: 1.0,
        padding: 1,
        bilinear: true,
        rotate_charts: true,
        rotate_charts_to_axis: true,
        create_image: false,
        ..crate::xatlas::PackOptions::default()
    };
    let packed = crate::xatlas::pack(&uvs, &indices, &chart, &options).expect("packed");
    let mut turned = 0;
    for corners in indices.chunks(6) {
        let at = |index: u32| Vec2::from_array(packed.uvs[index as usize].expect("in a chart"));
        for triangle in corners.chunks(3) {
            let [a, b, c] = [at(triangle[0]), at(triangle[1]), at(triangle[2])];
            assert!((b - a).perp_dot(c - a) > 0.0, "a chart was mirrored");
        }
        // Laid along U, a strip that ends up taller than wide was turned.
        let (low, high) = corners
            .iter()
            .fold((Vec2::MAX, Vec2::MIN), |(low, high), &index| {
                (low.min(at(index)), high.max(at(index)))
            });
        if (high - low).y > (high - low).x {
            turned += 1;
        }
    }
    assert!(turned > 0, "nothing was turned, so nothing was tested");
}
