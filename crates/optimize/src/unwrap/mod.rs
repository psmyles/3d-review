//! The Unwrap operation: rebuild a UV set by cutting the surface into islands,
//! flattening each with xatlas, and laying them out tiled at a texel density or
//! packed into an atlas.
//!
//! ## Per object
//!
//! An object's pieces (one per material) are unwrapped in one call, so an atlas
//! holds the whole object; materials are always apart, each piece's islands
//! numbered on from the last's. Within a piece, [`islands`] cuts the triangles into islands along the seams the settings
//! name (a seam attribute such as `_CAD_FACE`, the reference UVs' own seams,
//! sharp edges) and along open and non-manifold edges. xatlas then charts each
//! island — never across two, since each is a "material" of its own to it — and
//! flattens each chart: by orthogonal projection where it is flat, by LSCM
//! where not, split further only where a flattening would fold over itself.
//!
//! [`layout`] then places each chart. **Tiled**: scaled to the texel density by
//! its own area (`sqrt(3D area / UV area) * density`), turned by orthogonal
//! Procrustes to lie as its reference UVs did (a mirror allowed) and centred
//! where they were; and, per chart, the reference UVs scaled to the density
//! about their centre are kept instead where their area-weighted mean stretch
//! is lower. **Atlas**: xatlas's packing, scaled into the 0-1 square.
//!
//! The new UVs are per corner; the piece's vertices are split wherever two
//! corners of one vertex got different UVs ([`Submesh::split_by_corner`]), with
//! everything else they carry copied.
//!
//! ## Determinism
//!
//! xatlas is compiled single-threaded with its transcendentals on `libm`
//! (`third_party/xatlas/NOTICE.txt`); the pieces run in parallel, each in an
//! atlas of its own, and land in slots merged in piece order. Island and chart
//! numbering follow triangle order.

use glam::{Vec2, Vec3};
use review_model::ModelData;

use crate::Warnings;
use crate::notice::OptWarning;
use crate::process::{RunContext, is_excluded, resolve_op};
use crate::stack::{ChartGrowth, OpInstance, OpKind, OptStack, SeamSource, UnwrapParams, UvLayout};
use crate::submesh::Submesh;

mod arap;
mod islands;
mod layout;
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
    crate::parallel::solve_nodes(
        &jobs,
        run.token(),
        |(members, params, key), _: &dyn Fn(())| {
            let own: Vec<&Submesh> = members.iter().map(|&at| &pieces[at]).collect();
            unwrap_node(&own, params, *key, uv_sets)
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
) -> Result<Vec<Submesh>, String> {
    let mut out: Vec<Submesh> = pieces.iter().map(|&piece| piece.clone()).collect();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut island: Vec<u32> = Vec::new();
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
        island.extend(own.iter().map(|&id| id + island_base));
        normals.extend(
            piece
                .vertices
                .iter()
                .flat_map(|vertex| vertex.normal.to_array()),
        );
        positions.extend(own_positions);
        ranges.push(first..indices.len());
    }
    if indices.is_empty() {
        return Ok(out);
    }

    let flat: Vec<f32> = positions.iter().flat_map(|p| p.to_array()).collect();
    let input = crate::xatlas::Input {
        positions: &flat,
        normals: &normals,
        indices: &indices,
        face_group: &island,
    };
    let charts = layout::chart_options(params.charts);
    let pack = layout::pack_options(params, &positions, &indices);
    // Whole islands may curve as far as they like (patch P3).
    let whole_islands = params.charts == ChartGrowth::Islands;
    let atlas = crate::xatlas::atlas(&input, &charts, &pack, whole_islands)?;

    let corner_uv = match params.layout {
        UvLayout::Atlas => layout::atlas_uvs(&atlas),
        UvLayout::Tiled => {
            let tiled = |atlas: &crate::xatlas::Output| {
                layout::tiled_uvs(
                    atlas,
                    &positions,
                    &indices,
                    reference.as_deref(),
                    params,
                    pack.texels_per_unit,
                )
            };
            let mut uvs = tiled(&atlas);
            // An island that does not flatten in one piece — a band all the way
            // round a cylinder, with no seam inside its face, flattens only as a
            // ring — is cut where xatlas's own charting would cut it, where that
            // stretches it clearly less. Only then: a whole island is the plan.
            if whole_islands {
                let before = layout::island_stretch(&positions, &indices, &island, &uvs);
                let limit = params.island_stretch_limit;
                if before.iter().any(|&stretch| stretch > limit) {
                    let automatic = layout::chart_options(ChartGrowth::Automatic);
                    let cut = tiled(&crate::xatlas::atlas(&input, &automatic, &pack, false)?);
                    let after = layout::island_stretch(&positions, &indices, &island, &cut);
                    for (face, &own) in island.iter().enumerate() {
                        let own = own as usize;
                        if before[own] > limit && after[own] + layout::CLEARLY_LESS < before[own] {
                            uvs[face * 3..face * 3 + 3]
                                .copy_from_slice(&cut[face * 3..face * 3 + 3]);
                        }
                    }
                }
            }
            uvs
        }
    };
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
