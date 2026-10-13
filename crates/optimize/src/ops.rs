//! Applying one stack operation to one submesh.
//!
//! Every function here takes a [`Submesh`] and rewrites it in place, so
//! [`crate::process`](mod@crate::process) can walk the stack without knowing what any individual
//! operation does. The LOD operation is the exception: it fans one submesh out
//! into several and lives in [`crate::process`](mod@crate::process) with the rest of the chain logic
//! — though the simplify itself is [`simplify`] here, which the in-place Reduce
//! operation calls the same way.

use std::collections::HashMap;

use review_model::Vertex;

use crate::OptError;
use crate::meshopt::options::simplify as simplify_options;
use crate::meshopt::{self, SimplifyAttributes};
use crate::stack::{AttributeWeights, SimplifyAlgorithm, SimplifySettings, WeldParams};
use crate::submesh::Submesh;

/// Merge vertices per [`WeldParams`].
///
/// Two paths, both routed through meshoptimizer:
///
/// * **Exact** (`attribute_tolerance == 0`) packs the compared attributes into a
///   tight byte key and asks for binary-equality merging. Packing rather than
///   casting the Rust `Vertex` is deliberate — meshoptimizer compares *every*
///   byte including padding, and `Vertex` has no guaranteed layout.
/// * **Tolerance** hands meshoptimizer the positions (which it still requires to
///   match exactly) plus a predicate that decides whether the remaining
///   attributes are close enough. This is what rejoins a seam split only by
///   float drift.
///
/// Tangents are never compared. They are a *derived* basis — regenerated after
/// any geometry change — so letting a stale tangent difference keep two
/// otherwise identical vertices apart would block welds for no benefit.
pub fn weld(submesh: &mut Submesh, params: &WeldParams) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Weld Vertices");

    if submesh.is_empty() {
        return Ok(());
    }
    let vertex_count = submesh.vertices.len();

    // Deform rows are part of a vertex's identity for every weld: two vertices
    // that skin or morph differently are different vertices however alike they
    // look, and merging them would hand one of them the other's binding.
    let row_ids = submesh.row_ids();
    let (remap, unique) = if params.attribute_tolerance > 0.0 {
        let positions = submesh.positions();
        let attributes = AttributeView::new(submesh, params);
        meshopt::generate_vertex_remap_custom(
            &submesh.indices,
            &positions,
            vertex_count,
            |a, b| {
                attributes.within_tolerance(a, b, params.attribute_tolerance)
                    && (row_ids.is_empty() || row_ids.get(a as usize) == row_ids.get(b as usize))
            },
        )?
    } else {
        let (key_bytes, stride) = weld_key(submesh, params, &row_ids);
        meshopt::generate_vertex_remap(&submesh.indices, &key_bytes, vertex_count, stride)?
    };

    submesh.indices = meshopt::remap_index_buffer(&submesh.indices, &remap)?;
    submesh.apply_vertex_remap(&remap, unique);
    // A weld blind to normals keeps one of each merged group's normals
    // arbitrarily; the survivor then describes one face rather than the surface.
    if !params.compare_normals {
        submesh.normals_stale = true;
    }
    Ok(())
}

/// Remove degenerate and duplicate triangles.
pub fn filter_triangles(submesh: &mut Submesh) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Filter Triangles");

    if submesh.is_empty() {
        return Ok(());
    }
    let positions = submesh.positions();
    let before = submesh.polygons.is_some().then(|| submesh.indices.clone());
    // A duplicate triangle is only a duplicate when it deforms the same way:
    // the deform row rides beside the position as a second stream.
    let row_ids = submesh.row_ids();
    submesh.indices = if row_ids.is_empty() {
        meshopt::filter_index_buffer(&submesh.indices, &positions, submesh.vertices.len())?
    } else {
        meshopt::filter_index_buffer_with_rows(
            &submesh.indices,
            &positions,
            &row_ids,
            submesh.vertices.len(),
        )?
    };
    if let Some(before) = before {
        submesh.reconcile_triangles(&before);
    }
    Ok(())
}

/// Remove disconnected components smaller than `error`.
pub fn prune_components(submesh: &mut Submesh, error: f32) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Prune Components");

    if submesh.is_empty() {
        return Ok(());
    }
    let positions = submesh.positions();
    let before = submesh.polygons.is_some().then(|| submesh.indices.clone());
    submesh.indices =
        meshopt::simplify_prune(&submesh.indices, &positions, submesh.vertices.len(), error)?;
    if let Some(before) = before {
        submesh.reconcile_triangles(&before);
    }
    Ok(())
}

/// Reorder triangles for the post-transform vertex cache.
pub fn optimize_vertex_cache(submesh: &mut Submesh) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Optimize Vertex Cache");

    if submesh.is_empty() {
        return Ok(());
    }
    let before = submesh.polygons.is_some().then(|| submesh.indices.clone());
    submesh.indices = meshopt::optimize_vertex_cache(&submesh.indices, submesh.vertices.len())?;
    if let Some(before) = before {
        submesh.reconcile_triangles(&before);
    }
    Ok(())
}

/// Reorder triangles front-to-back to reduce overdraw.
pub fn optimize_overdraw(submesh: &mut Submesh, threshold: f32) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Optimize Overdraw");

    if submesh.is_empty() {
        return Ok(());
    }
    let positions = submesh.positions();
    let before = submesh.polygons.is_some().then(|| submesh.indices.clone());
    submesh.indices = meshopt::optimize_overdraw(
        &submesh.indices,
        &positions,
        submesh.vertices.len(),
        threshold,
    )?;
    if let Some(before) = before {
        submesh.reconcile_triangles(&before);
    }
    Ok(())
}

/// Reorder (and compact) vertices for linear vertex-buffer reads.
pub fn optimize_vertex_fetch(submesh: &mut Submesh) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Optimize Vertex Fetch");

    if submesh.is_empty() {
        return Ok(());
    }
    let (remap, unique) =
        meshopt::optimize_vertex_fetch_remap(&submesh.indices, submesh.vertices.len())?;
    submesh.indices = meshopt::remap_index_buffer(&submesh.indices, &remap)?;
    submesh.apply_vertex_remap(&remap, unique);
    Ok(())
}

/// Simplify `submesh` toward `target_triangles`, stopping short if `target_error`
/// would be exceeded. Returns the error meshoptimizer actually achieved (in the
/// same units as `target_error`: relative to mesh extent, or world units when
/// the absolute-error flag is set).
///
/// The reduced index buffer still points into the submesh's vertex array, and
/// [`Submesh::compact_unreferenced`] tidies up at the end of the level. The
/// array keeps its length and order; only the attribute-aware simplifier
/// rewrites values in it, and only those of the vertices it keeps.
pub fn simplify(
    submesh: &mut Submesh,
    settings: &SimplifySettings,
    target_triangles: usize,
    target_error: f32,
) -> Result<f32, OptError> {
    let _z = crate::prof::zone!("Simplify");

    if submesh.is_empty() {
        return Ok(0.0);
    }

    let outcome = simplify_buffers(
        &mut submesh.vertices,
        &mut submesh.uv_channels,
        &submesh.indices,
        settings,
        target_triangles.saturating_mul(3),
        target_error,
    )?;
    submesh.indices = outcome.indices;
    // The simplifier's triangles correspond to no source face.
    submesh.clear_polygons();
    Ok(outcome.error)
}

/// Simplify one *object*: every material piece of a node at once, toward one
/// triangle target for all of them together.
///
/// The pieces are concatenated and simplified as a single mesh, then dealt back
/// out by which piece each surviving vertex came from. A material border is two
/// pieces' vertices at the same positions, which meshoptimizer reads as an
/// attribute seam and keeps intact — so nothing tears — while the budget goes
/// wherever the error is lowest *across the whole object*. Simplifying each
/// piece toward its own share instead (what this replaced) spends triangles by
/// material rather than by shape: a dense decorative trim keeps detail nobody
/// can see while a sparse panel beside it loses its silhouette.
///
/// Returns the worst error, as [`simplify`] does. Pieces whose UV-set count
/// disagrees cannot share an attribute stream and are simplified one by one.
pub fn simplify_object(
    pieces: &mut Vec<&mut Submesh>,
    settings: &SimplifySettings,
    target_triangles: usize,
    target_error: f32,
) -> Result<f32, OptError> {
    let _z = crate::prof::zone!("Simplify Object");

    pieces.retain(|piece| !piece.is_empty());
    let total: usize = pieces.iter().map(|piece| piece.triangle_count()).sum();
    if pieces.len() <= 1
        || settings.algorithm == SimplifyAlgorithm::Sloppy
        || pieces
            .iter()
            .any(|piece| piece.uv_channels.len() != pieces[0].uv_channels.len())
    {
        // One piece, or a simplifier that would invent cross-piece triangles
        // (the sloppy one clusters vertices without regard to which piece they
        // came from): each piece toward its own share of the target.
        let mut worst = 0.0f32;
        for piece in pieces.iter_mut() {
            let share = (target_triangles as f64 * piece.triangle_count() as f64
                / total.max(1) as f64)
                .round() as usize;
            worst = worst.max(simplify(piece, settings, share, target_error)?);
        }
        return Ok(worst);
    }

    // Concatenate: each piece's vertices follow the previous piece's.
    let mut starts = Vec::with_capacity(pieces.len() + 1);
    let mut vertices = Vec::new();
    let mut uv_channels: Vec<Vec<glam::Vec2>> = vec![Vec::new(); pieces[0].uv_channels.len()];
    let mut indices = Vec::with_capacity(total * 3);
    for piece in pieces.iter() {
        let base = vertices.len() as u32;
        starts.push(base);
        vertices.extend_from_slice(&piece.vertices);
        for (combined, channel) in uv_channels.iter_mut().zip(&piece.uv_channels) {
            combined.extend_from_slice(channel);
        }
        indices.extend(piece.indices.iter().map(|&index| index + base));
    }
    starts.push(vertices.len() as u32);

    let outcome = simplify_buffers(
        &mut vertices,
        &mut uv_channels,
        &indices,
        settings,
        target_triangles.saturating_mul(3),
        target_error,
    )?;

    // Deal the triangles back out. A triangle belongs to the piece its first
    // corner came from; a corner from another piece (a collapse through a
    // vertex where several pieces meet can leave one) is replaced by the owning
    // piece's own vertex at the same position, which a border vertex always has.
    let piece_of = |vertex: u32| starts.partition_point(|&start| start <= vertex) - 1;
    let mut dealt: Vec<Vec<u32>> = vec![Vec::new(); pieces.len()];
    let mut by_position: Vec<Option<HashMap<[u32; 3], u32>>> = vec![None; pieces.len()];
    for triangle in outcome.indices.as_chunks::<3>().0.iter() {
        let owner = piece_of(triangle[0]);
        let mut local = [0u32; 3];
        let mut whole = true;
        for (slot, &vertex) in local.iter_mut().zip(triangle) {
            if piece_of(vertex) == owner {
                *slot = vertex - starts[owner];
                continue;
            }
            let lookup = by_position[owner].get_or_insert_with(|| {
                let range = starts[owner] as usize..starts[owner + 1] as usize;
                vertices[range]
                    .iter()
                    .enumerate()
                    .map(|(index, vertex)| (position_key(vertex.position), index as u32))
                    .collect()
            });
            match lookup.get(&position_key(vertices[vertex as usize].position)) {
                Some(&own) => *slot = own,
                None => whole = false,
            }
        }
        if whole {
            dealt[owner].extend_from_slice(&local);
        }
    }

    for (index, piece) in pieces.iter_mut().enumerate() {
        let range = starts[index] as usize..starts[index + 1] as usize;
        piece.vertices.copy_from_slice(&vertices[range.clone()]);
        for (channel, combined) in piece.uv_channels.iter_mut().zip(&uv_channels) {
            channel.copy_from_slice(&combined[range.clone()]);
        }
        piece.indices = std::mem::take(&mut dealt[index]);
        piece.clear_polygons();
    }
    Ok(outcome.error)
}

/// A position as exact bits, `-0.0` folded into `0.0`.
fn position_key(position: glam::Vec3) -> [u32; 3] {
    [
        canonical(position.x).to_bits(),
        canonical(position.y).to_bits(),
        canonical(position.z).to_bits(),
    ]
}

/// The simplifier proper, over a vertex array (with its extra UV sets) and a
/// triangle list over it. Returns the reduced index buffer, still addressing
/// `vertices`, and the error to report for it.
///
/// [`SimplifyAlgorithm::WithAttributes`] rewrites the vertices it keeps (see
/// [`simplify_appearance`]); the other two leave them exactly as they were.
fn simplify_buffers(
    vertices: &mut [Vertex],
    uv_channels: &mut [Vec<glam::Vec2>],
    indices: &[u32],
    settings: &SimplifySettings,
    target_indices: usize,
    target_error: f32,
) -> Result<meshopt::SimplifyOutcome, OptError> {
    let vertex_count = vertices.len();
    let mut positions = Vec::with_capacity(vertex_count * 3);
    for vertex in vertices.iter() {
        positions.extend_from_slice(&vertex.position.to_array());
    }
    match settings.algorithm {
        SimplifyAlgorithm::Standard => meshopt::simplify(
            indices,
            &positions,
            vertex_count,
            &SimplifyAttributes::default(),
            target_indices,
            target_error,
            settings.flags.bits(),
        ),
        SimplifyAlgorithm::WithAttributes => simplify_appearance(
            vertices,
            uv_channels,
            indices,
            positions,
            settings,
            target_indices,
            target_error,
        ),
        // The sloppy simplifier has no options parameter: it ignores topology by
        // construction, which is what most of the flags exist to modulate.
        SimplifyAlgorithm::Sloppy => meshopt::simplify_sloppy(
            indices,
            &positions,
            vertex_count,
            target_indices,
            target_error,
        ),
    }
}

/// The attribute-aware simplify: collapses chosen by shape *and* by how far the
/// normals, UVs and colours would drift, and every survivor then moved to the
/// point that best fits the surface it now stands for, with those attributes
/// refit there ([`meshopt::simplify_with_update`]).
///
/// The refit is what keeps a simplified surface shading like the source. A
/// survivor's original normal described the few faces around it at full detail —
/// on a bevelled edge it leans into the bevel — and once the collapses have
/// stretched its faces across a whole panel that lean tilts the shading of
/// everything it reaches, which reads as dark blotches. Measured against the
/// source's own per-pixel shading normals, from eight directions over ten real
/// assets at equal triangle counts, keeping survivors as they were left a mean
/// 11.5 degrees of error; weighing attributes without moving anything, 6.0;
/// weighing and refitting, 4.7, with the silhouette tighter as well.
///
/// The error limit is a limit on the *shape*, as the settings describe it. The
/// attribute error rides in the same figure inside meshoptimizer, so a single
/// run would stop as soon as the shading had drifted "far enough" however
/// little the surface had moved — on the fixtures that held most levels well
/// short of their triangle targets. So a position-only pass decides how far the
/// limit lets this object go, and the attribute-aware pass runs to exactly that
/// triangle count with no limit of its own. Loose pieces the shape pass pruned
/// are removed before the second pass, which then prunes nothing, so the error
/// limit alone decides what is thrown away. The reported error is the shape
/// pass's: the distance a collapse to this triangle count had to move the
/// surface, in the limit's own units.
fn simplify_appearance(
    vertices: &mut [Vertex],
    uv_channels: &mut [Vec<glam::Vec2>],
    indices: &[u32],
    mut positions: Vec<f32>,
    settings: &SimplifySettings,
    target_indices: usize,
    target_error: f32,
) -> Result<meshopt::SimplifyOutcome, OptError> {
    let vertex_count = vertices.len();
    let flags = settings.flags.bits();
    let shape = meshopt::simplify(
        indices,
        &positions,
        vertex_count,
        &SimplifyAttributes::default(),
        target_indices,
        target_error,
        flags,
    )?;

    let pruning = flags & simplify_options::PRUNE != 0;
    let kept = if pruning {
        drop_pruned_components(indices, &shape.indices, vertices)
    } else {
        indices.to_vec()
    };
    if kept.is_empty() {
        return Ok(meshopt::SimplifyOutcome {
            indices: Vec::new(),
            error: shape.error,
        });
    }

    let layout = AttributeLayout::new(vertices, uv_channels, settings.attribute_weights);
    let mut attributes = layout.stream(vertices, uv_channels);
    let outcome = meshopt::simplify_with_update(
        &kept,
        &mut positions,
        vertex_count,
        &mut attributes,
        shape.indices.len().max(target_indices),
        f32::MAX,
        flags & !simplify_options::PRUNE,
    )?;
    layout.write_back(vertices, uv_channels, &positions, &attributes.stream);
    Ok(meshopt::SimplifyOutcome {
        indices: outcome.indices,
        error: shape.error,
    })
}

/// The triangles of `indices` minus every connected piece that `simplified`
/// kept none of — the loose pieces a pruning simplify threw away. Pieces are
/// connected through shared *positions*, as meshoptimizer's own components are,
/// so a piece split by a UV seam is still one piece.
fn drop_pruned_components(indices: &[u32], simplified: &[u32], vertices: &[Vertex]) -> Vec<u32> {
    // One id per distinct position, then a union-find over the triangles.
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let position_id: Vec<u32> = vertices
        .iter()
        .map(|vertex| {
            let next = ids.len() as u32;
            *ids.entry(position_key(vertex.position)).or_insert(next)
        })
        .collect();
    let mut parent: Vec<u32> = (0..ids.len() as u32).collect();
    fn root(parent: &mut [u32], mut node: u32) -> u32 {
        while parent[node as usize] != node {
            parent[node as usize] = parent[parent[node as usize] as usize];
            node = parent[node as usize];
        }
        node
    }
    for triangle in indices.as_chunks::<3>().0.iter() {
        let a = root(&mut parent, position_id[triangle[0] as usize]);
        for &corner in &triangle[1..] {
            let b = root(&mut parent, position_id[corner as usize]);
            parent[b as usize] = a;
        }
    }
    let mut alive = vec![false; parent.len()];
    for &corner in simplified {
        let component = root(&mut parent, position_id[corner as usize]);
        alive[component as usize] = true;
    }
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|triangle| alive[root(&mut parent, position_id[triangle[0] as usize]) as usize])
        .flatten()
        .copied()
        .collect()
}

/// The weight an attribute the settings leave at zero still carries in the
/// attribute-aware simplify, so that it is *refit* where a survivor moves while
/// barely touching which edges collapse. meshoptimizer neither weighs nor
/// rewrites a zero-weighted attribute, and a survivor that moves with its old
/// normal, UV or colour no longer matches the surface under it: the texture
/// swims, a baked colour slides. The refit value does not depend on the weight
/// (meshoptimizer divides it back out), so any small positive one is exact.
const REFIT_WEIGHT: f32 = 1e-3;

/// Which attributes an attribute-aware simplify carries and at what weight, in
/// the order [`AttributeLayout::stream`] packs them and
/// [`AttributeLayout::write_back`] reads them: the normal, every UV set, the
/// vertex colour.
struct AttributeLayout {
    normal: f32,
    uv: f32,
    uv_sets: usize,
    /// `None` when every vertex carries the same colour — nothing to weigh or
    /// refit, so it stays out of the quadrics.
    color: Option<f32>,
}

impl AttributeLayout {
    fn new(
        vertices: &[Vertex],
        uv_channels: &[Vec<glam::Vec2>],
        weights: AttributeWeights,
    ) -> Self {
        let uniform_color = vertices.first().is_none_or(|first| {
            vertices
                .iter()
                .all(|v| v.vertex_color == first.vertex_color)
        });
        Self {
            normal: weights.normal.max(REFIT_WEIGHT),
            uv: weights.uv.max(REFIT_WEIGHT),
            uv_sets: uv_channels.len().max(1),
            color: (!uniform_color || weights.color > 0.0).then(|| weights.color.max(REFIT_WEIGHT)),
        }
    }

    fn stride(&self) -> usize {
        3 + self.uv_sets * 2 + if self.color.is_some() { 4 } else { 0 }
    }

    fn stream(&self, vertices: &[Vertex], uv_channels: &[Vec<glam::Vec2>]) -> SimplifyAttributes {
        let mut weights = vec![self.normal; 3];
        weights.extend(std::iter::repeat_n(self.uv, self.uv_sets * 2));
        if let Some(color) = self.color {
            weights.extend_from_slice(&[color; 4]);
        }
        let mut stream = Vec::with_capacity(vertices.len() * self.stride());
        for (index, vertex) in vertices.iter().enumerate() {
            stream.extend_from_slice(&vertex.normal.to_array());
            if uv_channels.is_empty() {
                stream.extend_from_slice(&vertex.uv.to_array());
            } else {
                for channel in uv_channels {
                    stream.extend_from_slice(&channel[index].to_array());
                }
            }
            if self.color.is_some() {
                stream.extend_from_slice(&vertex.vertex_color.to_array());
            }
        }
        SimplifyAttributes { stream, weights }
    }

    /// Copy what [`meshopt::simplify_with_update`] rewrote back into the
    /// vertices. It rewrites only the vertices its result references and leaves
    /// the rest bit for bit, so copying every vertex back is exact. A refit
    /// normal is a least-squares fit, not a unit vector, so it is renormalized.
    fn write_back(
        &self,
        vertices: &mut [Vertex],
        uv_channels: &mut [Vec<glam::Vec2>],
        positions: &[f32],
        stream: &[f32],
    ) {
        let stride = self.stride();
        for (index, vertex) in vertices.iter_mut().enumerate() {
            let p = &positions[index * 3..index * 3 + 3];
            vertex.position = glam::Vec3::new(p[0], p[1], p[2]);
            let a = &stream[index * stride..(index + 1) * stride];
            vertex.normal = glam::Vec3::new(a[0], a[1], a[2]).normalize_or(vertex.normal);
            let uv = &a[3..3 + self.uv_sets * 2];
            if uv_channels.is_empty() {
                vertex.uv = glam::Vec2::new(uv[0], uv[1]);
            } else {
                for (set, channel) in uv_channels.iter_mut().enumerate() {
                    channel[index] = glam::Vec2::new(uv[set * 2], uv[set * 2 + 1]);
                }
                vertex.uv = uv_channels[0][index];
            }
            if self.color.is_some() {
                let c = &a[3 + self.uv_sets * 2..];
                vertex.vertex_color = glam::Vec4::new(c[0], c[1], c[2], c[3]);
            }
        }
    }
}

/// Pack the attributes a weld compares into a tight byte key, one run of
/// `stride` bytes per vertex.
///
/// Positions are always included. `-0.0` is folded to `0.0` and every NaN to one
/// canonical NaN, because the comparison is bitwise: without that, two vertices
/// at visually identical positions could fail to merge over a sign bit no one
/// can see.
fn weld_key(submesh: &Submesh, params: &WeldParams, row_ids: &[u32]) -> (Vec<u8>, usize) {
    let uv_sets = weld_uv_sets(submesh, params);
    let components = 3
        + if params.compare_normals { 3 } else { 0 }
        + uv_sets * 2
        + if params.compare_colors { 4 } else { 0 }
        + usize::from(!row_ids.is_empty());
    let stride = components * size_of::<f32>();

    let mut bytes = Vec::with_capacity(submesh.vertices.len() * stride);
    let push = |value: f32, bytes: &mut Vec<u8>| {
        bytes.extend_from_slice(&canonical(value).to_ne_bytes());
    };

    for (index, vertex) in submesh.vertices.iter().enumerate() {
        push(vertex.position.x, &mut bytes);
        push(vertex.position.y, &mut bytes);
        push(vertex.position.z, &mut bytes);
        if params.compare_normals {
            push(vertex.normal.x, &mut bytes);
            push(vertex.normal.y, &mut bytes);
            push(vertex.normal.z, &mut bytes);
        }
        if params.compare_uvs {
            if submesh.uv_channels.is_empty() {
                push(vertex.uv.x, &mut bytes);
                push(vertex.uv.y, &mut bytes);
            } else {
                for channel in &submesh.uv_channels {
                    let uv = channel.get(index).copied().unwrap_or_default();
                    push(uv.x, &mut bytes);
                    push(uv.y, &mut bytes);
                }
            }
        }
        if params.compare_colors {
            push(vertex.vertex_color.x, &mut bytes);
            push(vertex.vertex_color.y, &mut bytes);
            push(vertex.vertex_color.z, &mut bytes);
            push(vertex.vertex_color.w, &mut bytes);
        }
        if let Some(&row) = row_ids.get(index) {
            bytes.extend_from_slice(&row.to_ne_bytes());
        }
    }

    (bytes, stride)
}

/// How many UV sets a weld key carries: none when UVs are not compared, every
/// stored channel on a multi-set model, otherwise the single set that lives in
/// [`Vertex::uv`].
fn weld_uv_sets(submesh: &Submesh, params: &WeldParams) -> usize {
    if !params.compare_uvs {
        0
    } else if submesh.uv_channels.is_empty() {
        1
    } else {
        submesh.uv_channels.len()
    }
}

/// Fold `-0.0` to `0.0` and every NaN to one bit pattern, so bitwise equality
/// matches numeric equality for the values a mesh actually contains.
fn canonical(value: f32) -> f32 {
    if value.is_nan() {
        f32::NAN
    } else if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Borrowed view of the attributes a tolerance weld compares, so the predicate
/// handed to meshoptimizer does no allocation per call.
struct AttributeView<'a> {
    vertices: &'a [Vertex],
    uv_channels: &'a [Vec<glam::Vec2>],
    compare_normals: bool,
    compare_uvs: bool,
    compare_colors: bool,
}

impl<'a> AttributeView<'a> {
    fn new(submesh: &'a Submesh, params: &WeldParams) -> Self {
        Self {
            vertices: &submesh.vertices,
            uv_channels: &submesh.uv_channels,
            compare_normals: params.compare_normals,
            compare_uvs: params.compare_uvs,
            compare_colors: params.compare_colors,
        }
    }

    /// Whether vertices `a` and `b` agree on every compared attribute to within
    /// `tolerance`.
    ///
    /// Runs on meshoptimizer's stack, so it must never panic: every lookup goes
    /// through `get`, and an out-of-range index reports "not equivalent" rather
    /// than unwinding across the FFI boundary.
    fn within_tolerance(&self, a: u32, b: u32, tolerance: f32) -> bool {
        let (Some(va), Some(vb)) = (self.vertices.get(a as usize), self.vertices.get(b as usize))
        else {
            return false;
        };

        if self.compare_normals && !close(va.normal.to_array(), vb.normal.to_array(), tolerance) {
            return false;
        }
        if self.compare_colors
            && !close(
                va.vertex_color.to_array(),
                vb.vertex_color.to_array(),
                tolerance,
            )
        {
            return false;
        }
        if self.compare_uvs {
            if self.uv_channels.is_empty() {
                if !close(va.uv.to_array(), vb.uv.to_array(), tolerance) {
                    return false;
                }
            } else {
                for channel in self.uv_channels {
                    let (Some(ua), Some(ub)) = (channel.get(a as usize), channel.get(b as usize))
                    else {
                        return false;
                    };
                    if !close(ua.to_array(), ub.to_array(), tolerance) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Componentwise absolute comparison.
fn close<const N: usize>(a: [f32; N], b: [f32; N], tolerance: f32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(&a, &b)| (a - b).abs() <= tolerance)
}

#[cfg(all(test, has_meshopt))]
mod tests {
    use std::collections::HashMap;

    use glam::{Vec2, Vec3};
    use review_model::Vertex;

    use super::*;
    use crate::stack::SimplifyFlags;
    use crate::submesh::VertexRows;

    fn piece(material: u32, vertices: Vec<Vertex>, indices: Vec<u32>) -> Submesh {
        Submesh {
            node: 0,
            material,
            vertices,
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices,
            polygons: None,
            skin: VertexRows::default(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
            normals_stale: false,
        }
    }

    /// An indexed `cells` x `cells` grid over `x0..x1` by `-1..1`, each vertex
    /// placed and given a normal by `at(x, y)`, with its UV the grid position.
    fn grid(
        material: u32,
        x0: f32,
        x1: f32,
        cells: u32,
        at: impl Fn(f32, f32) -> (Vec3, Vec3),
    ) -> Submesh {
        grid_of(material, x0, x1, (cells, cells), at)
    }

    /// [`grid`] with its own cell count across and down.
    fn grid_of(
        material: u32,
        x0: f32,
        x1: f32,
        (cells, rows): (u32, u32),
        at: impl Fn(f32, f32) -> (Vec3, Vec3),
    ) -> Submesh {
        let mut vertices = Vec::new();
        for j in 0..=rows {
            for i in 0..=cells {
                let (u, v) = (i as f32 / cells as f32, j as f32 / rows as f32);
                let (position, normal) = at(x0 + (x1 - x0) * u, -1.0 + 2.0 * v);
                vertices.push(Vertex {
                    position,
                    normal,
                    uv: Vec2::new(u, v),
                    ..Vertex::default()
                });
            }
        }
        let row = cells + 1;
        let mut indices = Vec::new();
        for j in 0..rows {
            for i in 0..cells {
                let a = j * row + i;
                indices.extend_from_slice(&[a, a + 1, a + row + 1, a, a + row + 1, a + row]);
            }
        }
        piece(material, vertices, indices)
    }

    fn flat(x: f32, y: f32) -> (Vec3, Vec3) {
        (Vec3::new(x, y, 0.0), Vec3::Z)
    }

    fn settings(algorithm: SimplifyAlgorithm) -> SimplifySettings {
        SimplifySettings {
            algorithm,
            ..SimplifySettings::default()
        }
    }

    /// The area-weighted mean angle, in degrees, between `expected` and the
    /// normal each surviving triangle shades its middle with.
    fn shading_error(piece: &Submesh, expected: Vec3) -> f32 {
        let (mut sum, mut area) = (0.0f32, 0.0f32);
        for triangle in piece.indices.as_chunks::<3>().0.iter() {
            let [a, b, c] = [0, 1, 2].map(|k| piece.vertices[triangle[k] as usize]);
            let weight = (b.position - a.position)
                .cross(c.position - a.position)
                .length();
            let normal = (a.normal + b.normal + c.normal).normalize_or_zero();
            sum += weight * normal.dot(expected).clamp(-1.0, 1.0).acos().to_degrees();
            area += weight;
        }
        sum / area.max(f32::MIN_POSITIVE)
    }

    /// The case that started this: a panel whose rim vertices carry normals
    /// leaning into a bevel, as a smoothed edge leaves them. Keeping survivors as
    /// they were stretches that lean across the whole panel once the interior
    /// is collapsed away — dark blotches over a flat face. The attribute-aware
    /// simplify refits it.
    #[test]
    fn a_simplified_panel_shades_flat_where_the_source_did() {
        let leaning = |x: f32, y: f32| {
            let rim = x.abs() > 0.999 || y.abs() > 0.999;
            let normal = if rim {
                (Vec3::new(
                    x.signum() * f32::from(x.abs() > 0.999),
                    y.signum() * f32::from(y.abs() > 0.999),
                    0.0,
                )
                .normalize_or_zero()
                    + Vec3::Z)
                    .normalize()
            } else {
                Vec3::Z
            };
            (Vec3::new(x, y, 0.0), normal)
        };
        let error_after = |algorithm| {
            let mut panel = grid(0, -1.0, 1.0, 30, leaning);
            let target = panel.triangle_count() / 20;
            simplify(&mut panel, &settings(algorithm), target, 1.0).unwrap();
            shading_error(&panel, Vec3::Z)
        };
        let kept = error_after(SimplifyAlgorithm::Standard);
        let refit = error_after(SimplifyAlgorithm::WithAttributes);
        assert!(
            refit * 2.0 < kept,
            "refitting should at least halve the panel's shading error: {refit:.2} deg against {kept:.2} deg"
        );
    }

    /// Two materials of one object: a dense flat grid and a sparse curved
    /// strip. Per material, each would keep a quarter of its own triangles and
    /// the strip would lose its curve; per object, the flat grid — which costs
    /// nothing to collapse — pays for nearly all of it.
    #[test]
    fn an_object_spends_its_budget_where_the_shape_needs_it() {
        let mut flat_grid = grid(0, -1.0, 1.0, 40, flat);
        let mut strip = grid_of(1, 3.0, 5.0, (64, 1), |x, y| {
            let angle = (x - 3.0) / 2.0 * std::f32::consts::PI;
            let normal = Vec3::new(angle.cos(), 0.0, angle.sin());
            (Vec3::new(4.0, y, 0.0) + normal, normal)
        });
        let strip_before = strip.triangle_count();
        let total = flat_grid.triangle_count() + strip_before;
        let mut pieces = vec![&mut flat_grid, &mut strip];
        simplify_object(
            &mut pieces,
            &settings(SimplifyAlgorithm::Standard),
            total / 4,
            1.0,
        )
        .unwrap();
        assert!(
            strip.triangle_count() * 10 >= strip_before * 9,
            "the curved piece kept {} of {strip_before} triangles",
            strip.triangle_count()
        );
        assert!(flat_grid.triangle_count() + strip.triangle_count() <= total / 4 + 2);
    }

    /// One bumpy surface split down the middle into two materials. Simplified
    /// as one object the border between them is an attribute seam — both sides
    /// keep it, and any vertex that moves moves on both — so welded by exact
    /// position the result has no edge used once except on its outer rim.
    #[test]
    fn a_material_border_does_not_open() {
        let bumpy = |x: f32, y: f32| {
            let z = 0.15 * (3.0 * x).sin() * (2.0 * y).cos();
            (Vec3::new(x, y, z), Vec3::Z)
        };
        let mut left = grid(0, -1.0, 0.0, 16, bumpy);
        let mut right = grid(1, 0.0, 1.0, 16, bumpy);
        let total = left.triangle_count() + right.triangle_count();
        let mut pieces = vec![&mut left, &mut right];
        simplify_object(
            &mut pieces,
            &settings(SimplifyAlgorithm::WithAttributes),
            total / 5,
            1.0,
        )
        .unwrap();

        let mut uses: HashMap<[[u32; 3]; 2], u32> = HashMap::new();
        for part in [&left, &right] {
            for triangle in part.indices.as_chunks::<3>().0.iter() {
                for k in 0..3 {
                    let a = position_key(part.vertices[triangle[k] as usize].position);
                    let b = position_key(part.vertices[triangle[(k + 1) % 3] as usize].position);
                    *uses.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
                }
            }
        }
        let on_rim = |key: [u32; 3]| {
            let (x, y) = (f32::from_bits(key[0]), f32::from_bits(key[1]));
            x.abs() > 0.999 || y.abs() > 0.999
        };
        let opened: Vec<_> = uses
            .iter()
            .filter(|(edge, count)| **count == 1 && !(on_rim(edge[0]) && on_rim(edge[1])))
            .collect();
        assert!(
            opened.is_empty(),
            "{} edges inside the surface opened up",
            opened.len()
        );
    }

    /// The error limit is a limit on how far the surface moves. A flat grid
    /// with scattered normals has a great deal of attribute error and no shape
    /// error at all, so it must reach the same count under the attribute-aware
    /// simplifier as under the position-only one — where a single run that let
    /// the shading spend the limit would stop almost at once.
    #[test]
    fn the_error_limit_bounds_the_shape_not_the_shading() {
        let scattered = |x: f32, y: f32| {
            let tilt = Vec3::new((x * 37.0).sin(), (y * 53.0).cos(), 0.0) * 0.5;
            (Vec3::new(x, y, 0.0), (Vec3::Z + tilt).normalize())
        };
        let reached = |algorithm| {
            let mut panel = grid(0, -1.0, 1.0, 24, scattered);
            let target = panel.triangle_count() / 10;
            simplify(&mut panel, &settings(algorithm), target, 0.001).unwrap();
            panel.triangle_count()
        };
        let shape_only = reached(SimplifyAlgorithm::Standard);
        let with_attributes = reached(SimplifyAlgorithm::WithAttributes);
        assert!(
            with_attributes <= shape_only + 2,
            "the attribute-aware run stopped at {with_attributes} triangles, the shape-only run at {shape_only}"
        );
    }

    /// With pruning on, the shape pass decides which loose pieces go: a speck
    /// far smaller than the error limit is removed, a piece the limit cannot
    /// cover is kept.
    #[test]
    fn pruning_removes_what_the_error_limit_covers() {
        let mut panel = grid(0, -1.0, 1.0, 20, flat);
        let speck = |x: f32, y: f32| (Vec3::new(3.0 + x * 0.001, y * 0.001, 0.0), Vec3::Z);
        let block = |x: f32, y: f32| (Vec3::new(5.0 + x * 0.5, y * 0.5, 0.0), Vec3::Z);
        for extra in [grid(0, -1.0, 1.0, 2, speck), grid(0, -1.0, 1.0, 2, block)] {
            let base = panel.vertices.len() as u32;
            panel.vertices.extend_from_slice(&extra.vertices);
            panel
                .indices
                .extend(extra.indices.iter().map(|&index| index + base));
        }
        let mut with_pruning = settings(SimplifyAlgorithm::WithAttributes);
        with_pruning.flags = SimplifyFlags {
            prune: true,
            ..SimplifyFlags::default()
        };
        simplify(&mut panel, &with_pruning, 4, 0.01).unwrap();
        let referenced = |range: std::ops::Range<f32>| {
            panel
                .indices
                .iter()
                .any(|&index| range.contains(&panel.vertices[index as usize].position.x))
        };
        assert!(!referenced(2.9..3.1), "the speck survived");
        assert!(referenced(4.4..5.6), "the block was pruned");
    }
}
