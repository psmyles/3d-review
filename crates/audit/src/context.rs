//! The per-model tables several checks share, built once per run on first use.

use std::borrow::Cow;
use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use review_model::extras::SourceExtras;
use review_model::topology::logical_ids;
use review_model::{Bounds, CancelToken, ModelData, NodeKind};

/// What one run reads.
#[derive(Clone, Copy)]
pub struct AuditInput<'a> {
    pub model: &'a ModelData,
    /// The source-property capture, when the load produced one. Rules that need
    /// it report `NotEvaluated(SourcePropertiesUnavailable)` without it.
    pub extras: Option<&'a SourceExtras>,
}

pub(crate) struct Context<'a> {
    pub model: &'a ModelData,
    pub extras: Option<&'a SourceExtras>,
    pub cancel: Option<&'a CancelToken>,
    pub threads: usize,
    logical: OnceLock<Cow<'a, [u32]>>,
    node_triangles: OnceLock<Vec<Vec<u32>>>,
    node_faces: OnceLock<Vec<Vec<u32>>>,
    node_bounds: OnceLock<Vec<Bounds>>,
    logical_corner: OnceLock<Vec<u32>>,
    corner_node: OnceLock<Vec<u32>>,
    synthetic: OnceLock<Vec<bool>>,
}

impl<'a> Context<'a> {
    pub fn new(input: AuditInput<'a>, cancel: Option<&'a CancelToken>, threads: usize) -> Self {
        Self {
            model: input.model,
            extras: input.extras,
            cancel,
            threads: threads.max(1),
            logical: OnceLock::new(),
            node_triangles: OnceLock::new(),
            node_faces: OnceLock::new(),
            node_bounds: OnceLock::new(),
            logical_corner: OnceLock::new(),
            corner_node: OnceLock::new(),
            synthetic: OnceLock::new(),
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled)
    }

    /// Per render corner, the logical vertex it came from.
    pub fn logical(&self) -> &[u32] {
        self.logical.get_or_init(|| logical_ids(self.model))
    }

    /// Per node, the triangles it owns.
    pub fn node_triangles(&self) -> &[Vec<u32>] {
        self.node_triangles
            .get_or_init(|| review_model::measure::node_triangles(self.model))
    }

    /// Per node, the polygons it owns — triangles standing in for a model with
    /// no face topology.
    pub fn node_faces(&self) -> &[Vec<u32>] {
        self.node_faces.get_or_init(|| {
            let model = self.model;
            if model.faces.is_empty() {
                return self.node_triangles().to_vec();
            }
            let mut lists = vec![Vec::new(); model.nodes.len()];
            if let Some(face_nodes) = model.face_nodes() {
                for (face, node) in face_nodes.into_iter().enumerate() {
                    if let Some(list) = lists.get_mut(node as usize) {
                        list.push(face as u32);
                    }
                }
            }
            lists
        })
    }

    /// Per node, the bounds of the triangles it owns.
    pub fn node_bounds(&self) -> &[Bounds] {
        self.node_bounds
            .get_or_init(|| review_model::measure::node_bounds(self.model))
    }

    /// Per logical vertex, one render corner that carries it (`u32::MAX` for a
    /// vertex no corner references).
    pub fn logical_corner(&self) -> &[u32] {
        self.logical_corner.get_or_init(|| {
            let logical = self.logical();
            let count = logical
                .iter()
                .filter(|&&id| id != u32::MAX)
                .map(|&id| id as usize + 1)
                .max()
                .unwrap_or(0)
                .max(self.model.stats.vertex_count);
            let mut corners = vec![u32::MAX; count];
            for (corner, &id) in logical.iter().enumerate() {
                if let Some(slot) = corners.get_mut(id as usize)
                    && *slot == u32::MAX
                {
                    *slot = corner as u32;
                }
            }
            corners
        })
    }

    /// Per render corner, the node of a triangle that uses it.
    pub fn corner_node(&self) -> &[u32] {
        self.corner_node.get_or_init(|| {
            let model = self.model;
            let mut nodes = vec![u32::MAX; model.vertices.len()];
            let triangle_count = model.indices.len() / 3;
            if model.triangles.node.len() == triangle_count {
                for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
                    for &corner in corners {
                        if let Some(slot) = nodes.get_mut(corner as usize) {
                            *slot = model.triangles.node[triangle];
                        }
                    }
                }
            }
            nodes
        })
    }

    /// Per node, whether the importer made it up rather than the file
    /// authoring it.
    pub fn synthetic(&self) -> &[bool] {
        self.synthetic.get_or_init(|| {
            (0..self.model.nodes.len())
                .map(|index| review_model::hierarchy::is_synthetic(self.model, self.extras, index))
                .collect()
        })
    }

    /// The authored nodes, in index order.
    pub fn authored_nodes(&self) -> impl Iterator<Item = usize> + '_ {
        let synthetic = self.synthetic();
        (0..self.model.nodes.len()).filter(move |&index| !synthetic[index])
    }

    /// The transform that took node `index`'s geometry into the (bind-pose)
    /// world the vertex buffer holds.
    pub fn geometry_to_world(&self, index: usize) -> Mat4 {
        let node = self
            .model
            .nodes
            .get(index)
            .map_or(Mat4::IDENTITY, |node| node.transform);
        let geometry = self
            .extras
            .and_then(|extras| extras.nodes.get(index))
            .map_or(Mat4::IDENTITY, |extras| extras.geometry_to_node);
        node * geometry
    }

    /// The world position of node `index`'s pivot.
    pub fn node_position(&self, index: usize) -> Vec3 {
        self.model
            .nodes
            .get(index)
            .map_or(Vec3::ZERO, |node| node.transform.w_axis.truncate())
    }

    /// Whether node `index` carries a skin: a skinned mesh, as opposed to a
    /// static one.
    pub fn is_skinned(&self, index: usize) -> bool {
        self.model.skin.as_ref().is_some_and(|skin| {
            skin.deformers
                .iter()
                .any(|deformer| deformer.mesh_node as usize == index)
        })
    }

    /// The kind of node `index`.
    pub fn kind(&self, index: usize) -> NodeKind {
        self.model
            .nodes
            .get(index)
            .map_or(NodeKind::Other, |node| node.kind)
    }

    /// Map `f` over every node index in parallel, keeping node order — the
    /// shape every per-object check takes. `None` when the run was cancelled
    /// part way.
    pub fn par_nodes<T: Send>(&self, f: impl Fn(usize) -> T + Sync) -> Option<Vec<T>> {
        let count = self.model.nodes.len();
        if self.threads <= 1 || count < 2 {
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                if self.cancelled() {
                    return None;
                }
                values.push(f(index));
            }
            return Some(values);
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let chunks = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..self.threads.min(count) {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        if self.cancelled() {
                            break;
                        }
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if index >= count {
                            break;
                        }
                        local.push((index, f(index)));
                    }
                    if let Ok(mut chunks) = chunks.lock() {
                        chunks.push(local);
                    }
                });
            }
        });
        let mut slots: Vec<Option<T>> = (0..count).map(|_| None).collect();
        for chunk in chunks.into_inner().ok()? {
            for (index, value) in chunk {
                slots[index] = Some(value);
            }
        }
        slots.into_iter().collect()
    }
}
