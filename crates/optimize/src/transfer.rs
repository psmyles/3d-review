//! Baking on the model as it came in and carrying the values to each level
//! (`BakeSource::Original`).
//!
//! The bake runs, unchanged, on a fresh partition of the input model, with
//! every instance of the operation (its overrides too) turned to write plain
//! linear alpha; each level vertex then takes the value at the nearest point of
//! the same object's baked surface, interpolated across the triangle there, and
//! writes it to the channel the settings name. Objects set to bake their own
//! level do so as before; while those bake, the transferred objects are
//! excluded — which for AO means they still occlude.

use glam::Vec4;
use review_model::{Bvh, ModelData};

use crate::process::{RunContext, is_excluded, resolve_op};
use crate::stack::{AoTarget, BakeSource, NodeOverride, OpInstance, OpKind, OptStack};
use crate::submesh::Submesh;

/// The bake's settings' source, target and sRGB flag.
fn source_of(kind: &OpKind) -> Option<(BakeSource, AoTarget, bool)> {
    match kind {
        OpKind::BakeAo(params) => Some((params.source, params.target, params.srgb)),
        OpKind::BakeWear(params) => Some((params.source, params.target, params.srgb)),
        _ => None,
    }
}

/// `kind`, baking its own mesh into linear alpha: what the original's bake runs.
fn scratch(kind: &OpKind) -> OpKind {
    let mut kind = kind.clone();
    match &mut kind {
        OpKind::BakeAo(params) => {
            params.source = BakeSource::Level;
            params.target = AoTarget::Alpha;
            params.srgb = false;
        }
        OpKind::BakeWear(params) => {
            params.source = BakeSource::Level;
            params.target = AoTarget::Alpha;
            params.srgb = false;
        }
        _ => {}
    }
    kind
}

/// Run a bake (`direct`, the operation's own function) over `submeshes`, each
/// object from the mesh its settings name.
pub(crate) fn bake(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    hidden_nodes: &[u32],
    run: &RunContext<'_>,
    direct: impl Fn(&mut [Submesh], &OpInstance, &OptStack),
) {
    let mut original: Vec<u32> = Vec::new();
    let mut own: Vec<u32> = Vec::new();
    for piece in submeshes.iter() {
        if original.contains(&piece.node)
            || own.contains(&piece.node)
            || is_excluded(stack, piece.node)
        {
            continue;
        }
        match source_of(resolve_op(stack, op, piece.node)) {
            Some((BakeSource::Original, ..)) => original.push(piece.node),
            _ => own.push(piece.node),
        }
    }

    if !own.is_empty() {
        if original.is_empty() {
            direct(submeshes, op, stack);
        } else {
            let mut levels = stack.clone();
            for &node in &original {
                exclude(&mut levels, node);
            }
            direct(submeshes, op, &levels);
        }
    }
    if original.is_empty() || run.cancelled() {
        return;
    }

    // The original, baked into alpha.
    let mut scratch_stack = stack.clone();
    for instance in &mut scratch_stack.ops {
        if instance.id == op.id {
            instance.kind = scratch(&instance.kind);
        }
    }
    for entry in &mut scratch_stack.overrides {
        for instance in &mut entry.ops {
            if instance.id == op.id {
                instance.kind = scratch(&instance.kind);
            }
        }
    }
    let scratch_op = OpInstance {
        kind: scratch(&op.kind),
        ..op.clone()
    };
    // Every object but the transferred ones is left out of the original's
    // bake (still occluding, for AO): only those are read back.
    for piece_node in own {
        exclude(&mut scratch_stack, piece_node);
    }
    let (mut baked, _) = crate::submesh::partition(model, None);
    for piece in &mut baked {
        for vertex in &mut piece.vertices {
            vertex.vertex_color = Vec4::ONE;
        }
    }
    direct(&mut baked, &scratch_op, &scratch_stack);
    if run.cancelled() {
        return;
    }

    for node in original {
        if hidden_nodes.contains(&node) {
            continue;
        }
        // The object's baked surface, and its values.
        let mut surface = ModelData::default();
        let mut values: Vec<f32> = Vec::new();
        for piece in baked.iter().filter(|piece| piece.node == node) {
            let base = surface.vertices.len() as u32;
            surface.vertices.extend_from_slice(&piece.vertices);
            surface
                .indices
                .extend(piece.indices.iter().map(|&index| index + base));
            values.extend(piece.vertices.iter().map(|vertex| vertex.vertex_color.w));
        }
        if surface.indices.is_empty() {
            continue;
        }
        let bvh = Bvh::build(&surface);
        let Some((_, target, srgb)) = source_of(resolve_op(stack, op, node)) else {
            continue;
        };
        for piece in submeshes.iter_mut().filter(|piece| piece.node == node) {
            for vertex in &mut piece.vertices {
                let Some(hit) = bvh.closest_point(&surface, vertex.position, f32::INFINITY) else {
                    continue;
                };
                let corners =
                    &surface.indices[hit.triangle as usize * 3..hit.triangle as usize * 3 + 3];
                let value = hit.barycentric.x * values[corners[0] as usize]
                    + hit.barycentric.y * values[corners[1] as usize]
                    + hit.barycentric.z * values[corners[2] as usize];
                crate::ao::write_target(
                    &mut vertex.vertex_color,
                    value.clamp(0.0, 1.0),
                    target,
                    srgb,
                );
            }
        }
    }
}

/// Leave `node` out of `stack`'s run.
fn exclude(stack: &mut OptStack, node: u32) {
    match stack
        .overrides
        .iter_mut()
        .find(|entry| entry.node == node as usize)
    {
        Some(entry) => entry.exclude = true,
        None => stack.overrides.push(NodeOverride {
            node: node as usize,
            exclude: true,
            ..NodeOverride::default()
        }),
    }
}
