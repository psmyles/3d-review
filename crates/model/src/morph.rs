//! Blend shapes: the channels, their keyframes, and the per-logical-vertex
//! offset CSR.

use glam::Vec3;

/// Blend-shape (morph target) data, stored **per logical source vertex** as a
/// compressed sparse-row table like [`SkinData`]: logical vertex `v`'s shape
/// offsets are `shape[offsets[v]..offsets[v+1]]` paired with `position[..]` /
/// `normal[..]`. Offsets are already rotated into the baked world orientation of
/// their mesh node (the importer applies the mesh's `geometry_to_world` linear
/// part), so adding `weight × offset` to a baked vertex *before* skinning is
/// exact — skinning is linear.
///
/// A channel is the artist-facing slider; it blends between its keyframes'
/// shapes by the ufbx in-between rule (see [`anim::channel_effective_weights`]).
/// In the common case a channel has exactly one keyframe at target weight 1.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphData {
    pub channels: Vec<MorphChannel>,
    pub shapes: Vec<MorphShape>,
    /// CSR row starts over the logical vertices, `logical_vertex_count + 1` long.
    pub offsets: Vec<u32>,
    /// Flat per-entry shape index (into [`MorphData::shapes`]), parallel to
    /// [`MorphData::position`] / [`MorphData::normal`].
    pub shape: Vec<u32>,
    /// Flat per-entry position offsets, baked-world oriented.
    pub position: Vec<Vec3>,
    /// Flat per-entry normal offsets (zero when the file declared none).
    pub normal: Vec<Vec3>,
}

impl MorphData {
    pub fn logical_vertex_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// The slice range into the flat entry arrays holding `logical`'s offsets.
    /// Empty for an out-of-range vertex.
    pub fn entry_range(&self, logical: usize) -> std::ops::Range<usize> {
        match (self.offsets.get(logical), self.offsets.get(logical + 1)) {
            (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
            _ => 0..0,
        }
    }

    /// The import-funnel guard, like [`SkinData::validate`].
    pub fn validate(&self, logical_count: usize, node_count: usize) -> Result<(), String> {
        if self.offsets.len() != logical_count + 1 {
            return Err(format!(
                "morph offsets has {} entries, expected {} (logical vertices + 1)",
                self.offsets.len(),
                logical_count + 1
            ));
        }
        if let Some(window) = self.offsets.windows(2).find(|window| window[0] > window[1]) {
            return Err(format!(
                "morph offsets are not monotonic: {} then {}",
                window[0], window[1]
            ));
        }
        let tail = self.offsets.last().copied().unwrap_or(0) as usize;
        if tail != self.shape.len() {
            return Err(format!(
                "morph offsets end at {tail} but there are {} entries",
                self.shape.len()
            ));
        }
        if self.position.len() != self.shape.len() || self.normal.len() != self.shape.len() {
            return Err(format!(
                "morph entry arrays disagree: {} shapes, {} positions, {} normals",
                self.shape.len(),
                self.position.len(),
                self.normal.len()
            ));
        }
        if let Some(&shape) = self
            .shape
            .iter()
            .find(|&&shape| shape as usize >= self.shapes.len())
        {
            return Err(format!(
                "morph entry references shape {shape} of {}",
                self.shapes.len()
            ));
        }
        if self
            .position
            .iter()
            .chain(&self.normal)
            .any(|offset| !offset.is_finite())
        {
            return Err("morph offset is not finite".to_owned());
        }
        for (index, channel) in self.channels.iter().enumerate() {
            if channel.mesh_node as usize >= node_count {
                return Err(format!(
                    "morph channel {index} references node {} of {node_count}",
                    channel.mesh_node
                ));
            }
            if !channel.rest_weight.is_finite() {
                return Err(format!(
                    "morph channel {index} has a non-finite rest weight"
                ));
            }
            for key in &channel.keyframes {
                if key.shape as usize >= self.shapes.len() {
                    return Err(format!(
                        "morph channel {index} references shape {} of {}",
                        key.shape,
                        self.shapes.len()
                    ));
                }
                if !key.target_weight.is_finite() {
                    return Err(format!(
                        "morph channel {index} has a non-finite target weight"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MorphChannel {
    pub name: String,
    /// The mesh node this channel deforms, indexing [`ModelData::nodes`].
    pub mesh_node: u32,
    /// The channel's weight at the file's default pose, in `0..=1`.
    pub rest_weight: f32,
    /// The channel's targets in ascending `target_weight` order.
    pub keyframes: Vec<MorphKeyframe>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphKeyframe {
    /// Index into [`MorphData::shapes`].
    pub shape: u32,
    /// The channel weight at which this shape applies at full strength.
    pub target_weight: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphShape {
    pub name: String,
}
