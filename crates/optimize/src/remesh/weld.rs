//! Welding by exact position: what every stage that has to ask "are these the
//! same point" means by it.
//!
//! The proxy rejoins the corner splits import made ([`super::proxy`]), the
//! distance field's extraction rejoins crossings that landed on one point
//! (`crate::shrinkwrap::mc`), and the voxel method welds the soup the library
//! hands back (`crate::shrinkwrap::remesher`). Each had its own key, and only
//! the last folded `-0.0` onto `+0.0` - so a point on an axis plane, which
//! exporters write with either sign, could be one vertex in a voxel shell and
//! two in the proxy, and the rebuild would see a seam that was not there.
//!
//! Exact bits rather than a tolerance because a corner split *is* exact: the
//! copies of one authored vertex share their position bit for bit, so this
//! rejoins every seam import made and nothing the artist authored apart.
//!
//! [`super::layout`]'s canonical order is deliberately *not* this key. A sort
//! key has to tell every distinct bit pattern apart or ties fall back to the
//! order the faces were emitted in, which is the thing it exists to remove.

use std::collections::HashMap;

use glam::Vec3;

/// A position as the identity a weld compares, with `-0.0` read as `+0.0`.
pub(crate) fn position_key(position: Vec3) -> [u32; 3] {
    // `-0.0 == 0.0`, so this rewrites exactly the one value with two spellings.
    let bits = |value: f32| if value == 0.0 { 0 } else { value.to_bits() };
    [bits(position.x), bits(position.y), bits(position.z)]
}

/// Merge `positions` that share a [`position_key`]: the distinct points in order
/// of first appearance, and each input's slot among them.
pub(crate) fn weld(positions: &[Vec3]) -> (Vec<Vec3>, Vec<u32>) {
    let mut slot_of: HashMap<[u32; 3], u32> = HashMap::with_capacity(positions.len());
    let mut unique: Vec<Vec3> = Vec::with_capacity(positions.len());
    let remap = positions
        .iter()
        .map(|&position| {
            let next = unique.len() as u32;
            *slot_of.entry(position_key(position)).or_insert_with(|| {
                unique.push(position);
                next
            })
        })
        .collect();
    (unique, remap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_zeros_are_one_point() {
        assert_eq!(
            position_key(Vec3::new(-0.0, 1.0, 0.0)),
            position_key(Vec3::new(0.0, 1.0, -0.0))
        );
        assert_ne!(
            position_key(Vec3::new(1.0e-30, 0.0, 0.0)),
            position_key(Vec3::ZERO),
            "anything that is not zero keeps its own bits"
        );
    }

    #[test]
    fn a_weld_keeps_first_appearance_order() {
        let points = [
            Vec3::X,
            Vec3::Y,
            Vec3::X,
            Vec3::new(-0.0, 1.0, 0.0),
            Vec3::Z,
        ];

        let (unique, remap) = weld(&points);

        assert_eq!(unique, vec![Vec3::X, Vec3::Y, Vec3::Z]);
        assert_eq!(remap, vec![0, 1, 0, 1, 2]);
    }
}
