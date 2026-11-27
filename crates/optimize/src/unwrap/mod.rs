//! The Unwrap operation: rebuild a UV set the way an unwrap in Blender or
//! Maya does — cut the surface into islands (**seams**), flatten each
//! (**unfold**), and pack them into the 0-1 square or tile them at a texel
//! density (**layout**).
//!
//! ## Per object
//!
//! An object's pieces (one per material) are unwrapped in one call, so an atlas
//! holds the whole object; materials are always apart, each piece's islands
//! numbered on from the last's. Within a piece, [`islands`] cuts the triangles
//! into islands along the seams the settings name (a seam attribute such as
//! `_CAD_FACE`, the reference UVs' own seams, sharp edges) and along open and
//! non-manifold edges, and numbers the cut surface's vertices.
//!
//! [`charts`] then makes each island into as few charts as flatten within the
//! stretch limit: the island whole where it can be, otherwise small, nearly
//! flat pieces of it merged back greedily. Each chart is flattened by [`lscm`]
//! and, for minimum stretch, refined by [`arap`], and comes out in model units.
//!
//! [`layout`] places the charts. **Atlas** (Pack into 0-1): xatlas's packer,
//! scaled into the square. **Tiled**: each chart at the texel density, turned
//! by orthogonal Procrustes to lie as its reference UVs did (a mirror allowed)
//! and centred where they were, the reference UVs kept instead where they
//! stretch less; with no reference to follow, packed apart at the density.
//!
//! The new UVs are per corner; the piece's vertices are split wherever two
//! corners of one vertex got different UVs ([`Submesh::split_by_corner`]), with
//! everything else they carry copied.
//!
//! ## Determinism
//!
//! The segmentation and flattening are this crate's own: `f64` with bounded
//! iterations, the merge's order and maps all ordered. xatlas only packs, and
//! is compiled single-threaded with its transcendentals on `libm`
//! (`third_party/xatlas/NOTICE.txt`). The objects run
//! in parallel, each on its own, and land in slots merged in object order.
//! Island, chart and vertex numbering follow triangle order.

use glam::{Vec2, Vec3};
use review_model::ModelData;

use crate::Warnings;
use crate::notice::OptWarning;
use crate::process::{RunContext, is_excluded, resolve_op};
use crate::stack::{OpInstance, OpKind, OptStack, SeamSource, UnwrapParams};
use crate::submesh::Submesh;

mod arap;
mod charts;
mod element;
mod islands;
mod layout;
mod lscm;
mod solve;
#[cfg(test)]
mod tests;

/// The least time between two previews of an unwrap in progress.
const PREVIEW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// Where a piece's seam ids are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SeamKey {
    None,
    /// Words `offset..offset + width` of each vertex's custom row.
    Custom {
        offset: usize,
        width: usize,
    },
    /// The U of UV set `set`.
    UvSetU(usize),
}

/// The seam key the settings name, resolved against the model's custom
/// attribute layout (`submesh::partition`'s), or why it cannot be.
fn seam_key(seams: &SeamSource, model: &ModelData) -> Result<SeamKey, String> {
    match seams {
        SeamSource::None => Ok(SeamKey::None),
        SeamSource::UvSetU(set) => Ok(SeamKey::UvSetU(*set as usize)),
        SeamSource::Attribute(name) => {
            let mut offset = 0;
            for attribute in model
                .custom_attributes
                .iter()
                .filter(|attribute| attribute.vertex_count() == model.vertices.len())
            {
                if &attribute.name == name {
                    return Ok(SeamKey::Custom {
                        offset,
                        width: attribute.width(),
                    });
                }
                offset += attribute.width();
            }
            Err(name.clone())
        }
    }
}

/// Run the Unwrap operation over every piece it applies to.
pub(crate) fn unwrap_submeshes(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    warnings: &mut Warnings,
    run: &RunContext<'_>,
) {
    let _z = crate::prof::zone!("Unwrap");
    let OpKind::Unwrap(global) = &op.kind else {
        return;
    };
    if !crate::xatlas::available() {
        warnings.push(OptWarning::OperationFailed {
            operation: op.kind.clone(),
            detail: crate::OptError::Unavailable.to_string(),
        });
        return;
    }

    // One job per object: its pieces (indices into `submeshes`, in order), the
    // settings that apply to it, and its seam key.
    let mut jobs: Vec<(Vec<usize>, UnwrapParams, SeamKey)> = Vec::new();
    for (at, piece) in submeshes.iter().enumerate() {
        if piece.is_empty() || is_excluded(stack, piece.node) {
            continue;
        }
        if let Some(job) = jobs
            .iter_mut()
            .find(|(members, _, _)| submeshes[members[0]].node == piece.node)
        {
            job.0.push(at);
            continue;
        }
        let params = match resolve_op(stack, op, piece.node) {
            OpKind::Unwrap(params) => params.clone(),
            _ => global.clone(),
        };
        let key = match seam_key(&params.seams, model) {
            Ok(key) => key,
            Err(name) => {
                warnings.push(OptWarning::UnwrapSeamAttributeMissing {
                    object: crate::remesh::node_name(model, piece.node).to_string(),
                    attribute: name,
                });
                SeamKey::None
            }
        };
        jobs.push((vec![at], params, key));
    }

    let uv_sets = model.stats.uv_set_count;
    let pieces: &[Submesh] = submeshes;
    let mut results: Vec<Option<Result<Vec<Submesh>, String>>> =
        (0..jobs.len()).map(|_| None).collect();
    // While someone watches, the scene is shown with each object's new UVs as
    // it lands — at most every `PREVIEW_INTERVAL`, as a preview costs an
    // assemble of the level. Only what is shown depends on the timing.
    let mut last_preview: Option<std::time::Instant> = None;
    let mut landed = 0;
    let token = run.token();
    crate::parallel::solve_nodes(
        &jobs,
        token,
        |(members, params, key), _: &dyn Fn(())| {
            let own: Vec<&Submesh> = members.iter().map(|&at| &pieces[at]).collect();
            unwrap_node(&own, params, *key, uv_sets, token)
        },
        |index, result| {
            results[index] = Some(result);
            landed += 1;
            let due = last_preview.is_none_or(|shown| shown.elapsed() >= PREVIEW_INTERVAL);
            // The last object's landing is the result itself, a moment later.
            if run.wants_preview() && due && landed < jobs.len() {
                last_preview = Some(std::time::Instant::now());
                let mut scene: Vec<&Submesh> = pieces.iter().collect();
                for ((members, _, _), landed) in jobs.iter().zip(&results) {
                    if let Some(Ok(unwrapped)) = landed {
                        for (&at, piece) in members.iter().zip(unwrapped) {
                            scene[at] = piece;
                        }
                    }
                }
                run.preview(&scene);
            }
        },
        |_, ()| {},
    );
    // A cancelled run is discarded whole: nothing it says is worth a warning.
    if run.cancelled() {
        return;
    }
    for ((members, _, _), result) in jobs.iter().zip(results) {
        match result {
            Some(Ok(unwrapped)) => {
                for (&at, piece) in members.iter().zip(unwrapped) {
                    submeshes[at] = piece;
                }
            }
            Some(Err(detail)) => warnings.push(OptWarning::UnwrapFailed {
                object: crate::remesh::node_name(model, submeshes[members[0]].node).to_string(),
                detail,
            }),
            // Cancelled before it ran: the run is discarded whole.
            None => {}
        }
    }
}

/// The reference UVs of `piece`'s vertices, when the settings name a set the
/// model has.
fn reference_uvs(piece: &Submesh, set: Option<u32>, uv_sets: usize) -> Option<Vec<Vec2>> {
    let set = set? as usize;
    if set >= uv_sets {
        return None;
    }
    Some(match piece.uv_channels.get(set) {
        Some(channel) => channel.clone(),
        None if set == 0 => piece.vertices.iter().map(|vertex| vertex.uv).collect(),
        None => return None,
    })
}

/// One object's pieces, unwrapped together: copies of them with the target set
/// rewritten. Together, so that an atlas is one per object (its materials side
/// by side in one square, as one lightmap or bake texture wants them); the
/// island ids keep every chart inside one piece.
pub(crate) fn unwrap_node(
    pieces: &[&Submesh],
    params: &UnwrapParams,
    key: SeamKey,
    uv_sets: usize,
    cancel: Option<&crate::cancel::CancelToken>,
) -> Result<Vec<Submesh>, String> {
    let mut out: Vec<Submesh> = pieces.iter().map(|&piece| piece.clone()).collect();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut island: Vec<u32> = Vec::new();
    let mut cut_vertex: Vec<u32> = Vec::new();
    let mut cut_count: u32 = 0;
    let mut reference: Option<Vec<Vec2>> = Some(Vec::new());
    // Each piece's corners in the combined index buffer.
    let mut ranges = Vec::with_capacity(out.len());
    for piece in &out {
        let base = positions.len() as u32;
        let island_base = island.iter().max().map_or(0, |&last| last + 1);
        let own_positions: Vec<Vec3> = piece
            .vertices
            .iter()
            .map(|vertex| vertex.position)
            .collect();
        let own_reference = reference_uvs(piece, params.reference_set, uv_sets);
        let own = islands::islands(
            piece,
            &own_positions,
            own_reference.as_deref(),
            key,
            params,
            uv_sets,
        );
        reference = match (reference, own_reference) {
            (Some(mut all), Some(own)) => {
                all.extend(own);
                Some(all)
            }
            _ => None,
        };
        let first = indices.len();
        indices.extend(piece.indices.iter().map(|&index| index + base));
        island.extend(own.island.iter().map(|&id| id + island_base));
        cut_vertex.extend(own.vertex.iter().map(|&vertex| vertex + cut_count));
        cut_count += own.vertex_count;
        positions.extend(own_positions);
        ranges.push(first..indices.len());
    }
    if indices.is_empty() {
        return Ok(out);
    }
    let count = indices.len() / 3;

    // The cut surface, its charts, and their layout.
    let mut cut_positions = vec![Vec3::ZERO; cut_count as usize];
    for (corner, &vertex) in cut_vertex.iter().enumerate() {
        cut_positions[vertex as usize] = positions[indices[corner] as usize];
    }
    let triangles: Vec<[u32; 3]> = cut_vertex.as_chunks::<3>().0.to_vec();
    let mesh = charts::CutMesh {
        positions: &cut_positions,
        triangles: &triangles,
        island: &island,
    };
    let settings = charts::Settings {
        flattening: params.flattening,
        max_stretch: params.max_stretch,
    };
    let charts = charts::charts(&mesh, settings, cancel).ok_or_else(|| "cancelled".to_owned())?;
    let corner_uv = layout::lay_out(
        &charts,
        count,
        &positions,
        &indices,
        reference.as_deref(),
        params,
    )?;
    for (piece, range) in out.iter_mut().zip(ranges) {
        write_set(
            piece,
            &corner_uv[range],
            params.target_set as usize,
            uv_sets,
        );
    }
    Ok(out)
}

/// Give every corner its UV in set `set`, splitting vertices whose corners
/// disagree, and add the set (and any before it) when the piece lacks it.
fn write_set(piece: &mut Submesh, corner_uv: &[Vec2], set: usize, uv_sets: usize) {
    // Compare by bits: two corners share a vertex only if their UVs are the
    // same number exactly.
    let keyed: Vec<[u32; 2]> = corner_uv
        .iter()
        .map(|uv| [uv.x.to_bits(), uv.y.to_bits()])
        .collect();
    let values = piece.split_by_corner(&keyed);

    // The sets the piece will carry: its own, and up to `set`. A table is kept
    // when there is more than one, or when the set is new — a model that had
    // no UVs at all must come out counting the one it now has.
    let had = uv_sets.max(piece.uv_channels.len());
    let wanted = had.max(set + 1);
    if !piece.uv_channels.is_empty() || wanted > 1 || had < wanted {
        if piece.uv_channels.is_empty() {
            // Set 0 joins the table, mirrored from the vertices.
            piece
                .uv_channels
                .push(piece.vertices.iter().map(|vertex| vertex.uv).collect());
        }
        while piece.uv_channels.len() < wanted {
            piece
                .uv_channels
                .push(vec![Vec2::ZERO; piece.vertices.len()]);
        }
    }
    for (vertex, value) in values.into_iter().enumerate() {
        let Some([u, v]) = value else {
            continue;
        };
        let uv = Vec2::new(f32::from_bits(u), f32::from_bits(v));
        if set == 0 {
            piece.vertices[vertex].uv = uv;
        }
        if let Some(channel) = piece.uv_channels.get_mut(set) {
            channel[vertex] = uv;
        }
    }
}
