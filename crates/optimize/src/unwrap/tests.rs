//! The pieces of the unwrap that are pure maths.

use glam::{Vec2, Vec3};

use super::layout::stretch;

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
