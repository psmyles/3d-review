//! Applying one stack operation to one submesh.
//!
//! Every function here takes a [`Submesh`] and rewrites it in place, so
//! [`crate::process`] can walk the stack without knowing what any individual
//! operation does. The LOD operation is the exception: it fans one submesh out
//! into several and lives in [`crate::process`] with the rest of the chain logic
//! — though the simplify itself is [`simplify`] here, which the in-place Reduce
//! operation calls the same way.

use review_model::Vertex;

use crate::OptError;
use crate::meshopt::{self, SimplifyAttributes};
use crate::stack::{SimplifyAlgorithm, SimplifySettings, WeldParams};
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
/// The submesh's vertex array is left alone — the reduced index buffer still
/// points into it, and [`Submesh::compact_unreferenced`] tidies up at the end of
/// the level.
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

    let positions = submesh.positions();
    let vertex_count = submesh.vertices.len();
    let target_indices = target_triangles.saturating_mul(3);

    let outcome = match settings.algorithm {
        SimplifyAlgorithm::Standard => meshopt::simplify(
            &submesh.indices,
            &positions,
            vertex_count,
            &SimplifyAttributes::default(),
            target_indices,
            target_error,
            settings.flags.bits(),
        )?,
        SimplifyAlgorithm::WithAttributes => {
            let attributes = simplify_attribute_stream(submesh, settings);
            meshopt::simplify(
                &submesh.indices,
                &positions,
                vertex_count,
                &attributes,
                target_indices,
                target_error,
                settings.flags.bits(),
            )?
        }
        // The sloppy simplifier has no options parameter: it ignores topology by
        // construction, which is what most of the flags exist to modulate.
        SimplifyAlgorithm::Sloppy => meshopt::simplify_sloppy(
            &submesh.indices,
            &positions,
            vertex_count,
            target_indices,
            target_error,
        )?,
    };

    submesh.indices = outcome.indices;
    // The simplifier's triangles correspond to no source face.
    submesh.clear_polygons();
    Ok(outcome.error)
}

/// Build the packed attribute stream and per-component weights for
/// [`SimplifyAlgorithm::WithAttributes`]. Components with a zero weight are left
/// out entirely rather than passed with a zero weight — a shorter stream is less
/// work for the simplifier's quadric to carry.
fn simplify_attribute_stream(submesh: &Submesh, settings: &SimplifySettings) -> SimplifyAttributes {
    let weights = settings.attribute_weights;
    let use_normal = weights.normal > 0.0;
    let use_uv = weights.uv > 0.0;
    let use_color = weights.color > 0.0;

    let mut component_weights = Vec::new();
    if use_normal {
        component_weights.extend_from_slice(&[weights.normal; 3]);
    }
    if use_uv {
        component_weights.extend_from_slice(&[weights.uv; 2]);
    }
    if use_color {
        component_weights.extend_from_slice(&[weights.color; 4]);
    }
    if component_weights.is_empty() {
        return SimplifyAttributes::default();
    }

    let mut stream = Vec::with_capacity(submesh.vertices.len() * component_weights.len());
    for vertex in &submesh.vertices {
        if use_normal {
            stream.extend_from_slice(&[vertex.normal.x, vertex.normal.y, vertex.normal.z]);
        }
        if use_uv {
            stream.extend_from_slice(&[vertex.uv.x, vertex.uv.y]);
        }
        if use_color {
            stream.extend_from_slice(&[
                vertex.vertex_color.x,
                vertex.vertex_color.y,
                vertex.vertex_color.z,
                vertex.vertex_color.w,
            ]);
        }
    }

    SimplifyAttributes {
        stream,
        weights: component_weights,
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
