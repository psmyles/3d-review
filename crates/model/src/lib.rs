use glam::{Vec2, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    /// Resolved material base color (carried through import), used as the surface
    /// color in the normal shaded/unlit views.
    pub color: Vec4,
    pub tangent: Vec4,
    /// Per-vertex RGBA color from the mesh's vertex-color attribute (the DCC
    /// color set), distinct from [`Vertex::color`]. White when the mesh carries
    /// no vertex-color layer. Visualized by the vertex-color debug view.
    pub vertex_color: Vec4,
    /// Resolved material smoothness in `0.0..=1.0` (glossiness, i.e.
    /// `1 - roughness`), carried through import like [`Vertex::color`] so the
    /// shaded view can drive a specular highlight. `0.5` when the source
    /// material declares neither glossiness nor roughness.
    pub smoothness: f32,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            color: Vec4::ONE,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            vertex_color: Vec4::ONE,
            smoothness: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(self) -> Vec3 {
        self.max - self.min
    }

    pub fn radius(self) -> f32 {
        self.size().length() * 0.5
    }

    pub fn is_empty(self) -> bool {
        self.min.x.is_infinite() || self.max.x.is_infinite()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialInfo {
    pub name: String,
    pub draw_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelWarning {
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopologyFace {
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModelStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    pub vertex_count: usize,
    pub uv_set_count: usize,
    pub material_count: usize,
    pub draw_count: usize,
    /// The model's authored world unit, in meters per source unit, as recorded
    /// in the file (e.g. `0.01` for a centimeter file like a Maya export). This
    /// is the *original* unit before import normalizes everything to meters, so
    /// the stats panel can show what the file claimed. `0.0` means the source
    /// declared no unit (e.g. the built-in demo, or a file missing the metadata).
    pub source_unit_meters: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelData {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub faces: Vec<TopologyFace>,
    pub tri_to_face: Vec<u32>,
    /// Per-vertex UV coordinates for every UV set the model carries, one inner
    /// vector per channel (each `vertices.len()` long). Only populated when the
    /// model has **more than one** UV set; single-set models leave this empty
    /// and use [`Vertex::uv`] (which always holds channel 0). Channel 0 is thus
    /// mirrored here for multi-set models — a deliberate trade so the renderer
    /// can pick a channel by index without special-casing channel 0.
    pub uv_channels: Vec<Vec<Vec2>>,
    pub bounds: Option<Bounds>,
    pub stats: ModelStats,
    pub materials: Vec<MaterialInfo>,
    pub warnings: Vec<ModelWarning>,
}

impl ModelData {
    /// UV coordinates for `vertex_index` in `channel`, falling back to the
    /// vertex's own [`Vertex::uv`] when the requested channel isn't stored
    /// (single-set models, or an out-of-range channel).
    pub fn uv_for_channel(&self, vertex_index: usize, channel: usize) -> Vec2 {
        self.uv_channels
            .get(channel)
            .and_then(|channel_uvs| channel_uvs.get(vertex_index).copied())
            .unwrap_or_else(|| {
                self.vertices
                    .get(vertex_index)
                    .map(|vertex| vertex.uv)
                    .unwrap_or(Vec2::ZERO)
            })
    }

    pub fn recompute_bounds(&mut self) {
        let mut bounds = Bounds::EMPTY;

        for vertex in &self.vertices {
            bounds.include_point(vertex.position);
        }

        self.bounds = (!bounds.is_empty()).then_some(bounds);
    }
}

pub fn demo_cube_model() -> ModelData {
    let mut model = ModelData {
        name: "Demo Cube".to_owned(),
        vertices: demo_cube_vertices(),
        indices: demo_cube_indices(),
        faces: (0..6)
            .map(|face_index| TopologyFace {
                first_index: face_index * 4,
                index_count: 4,
            })
            .collect(),
        tri_to_face: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
        stats: ModelStats {
            polygon_count: 6,
            triangle_count: 12,
            vertex_count: 24,
            uv_set_count: 1,
            material_count: 1,
            draw_count: 1,
            source_unit_meters: 1.0,
        },
        materials: vec![MaterialInfo {
            name: "Default".to_owned(),
            draw_count: 1,
        }],
        warnings: Vec::new(),
        ..Default::default()
    };
    model.recompute_bounds();
    model
}

fn demo_cube_vertices() -> Vec<Vertex> {
    let s = 0.5;
    let y0 = 0.03;
    let y1 = 1.03;

    let faces = [
        (
            [[-s, y0, s], [s, y0, s], [s, y1, s], [-s, y1, s]],
            Vec3::Z,
            Vec4::new(0.30, 0.58, 0.86, 0.92),
        ),
        (
            [[s, y0, -s], [-s, y0, -s], [-s, y1, -s], [s, y1, -s]],
            Vec3::NEG_Z,
            Vec4::new(0.20, 0.37, 0.56, 0.92),
        ),
        (
            [[-s, y0, -s], [-s, y0, s], [-s, y1, s], [-s, y1, -s]],
            Vec3::NEG_X,
            Vec4::new(0.22, 0.47, 0.73, 0.92),
        ),
        (
            [[s, y0, s], [s, y0, -s], [s, y1, -s], [s, y1, s]],
            Vec3::X,
            Vec4::new(0.40, 0.68, 0.92, 0.92),
        ),
        (
            [[-s, y1, s], [s, y1, s], [s, y1, -s], [-s, y1, -s]],
            Vec3::Y,
            Vec4::new(0.62, 0.79, 0.96, 0.96),
        ),
        (
            [[-s, y0, -s], [s, y0, -s], [s, y0, s], [-s, y0, s]],
            Vec3::NEG_Y,
            Vec4::new(0.14, 0.25, 0.35, 0.92),
        ),
    ];

    let uvs = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];

    // Distinct per-corner vertex colors so the vertex-color debug view has
    // something to show on the built-in demo: an R/G/B/yellow ring with a
    // 0.25→1.0 alpha ramp to exercise the alpha / RGB+A modes too.
    let vertex_colors = [
        Vec4::new(1.0, 0.0, 0.0, 0.25),
        Vec4::new(0.0, 1.0, 0.0, 0.50),
        Vec4::new(0.0, 0.0, 1.0, 0.75),
        Vec4::new(1.0, 1.0, 0.0, 1.00),
    ];

    let mut vertices = Vec::with_capacity(24);
    for (positions, normal, color) in faces {
        for (index, position) in positions.into_iter().enumerate() {
            vertices.push(Vertex {
                position: Vec3::from_array(position),
                normal,
                uv: uvs[index],
                color,
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: vertex_colors[index],
                smoothness: 0.6,
            });
        }
    }

    vertices
}

fn demo_cube_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity(36);
    for face_index in 0..6 {
        let base = face_index * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    indices
}
