use glam::{Vec2, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: Vec4,
    pub tangent: Vec4,
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            color: Vec4::ONE,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelStats {
    pub polygon_count: usize,
    pub triangle_count: usize,
    pub vertex_count: usize,
    pub uv_set_count: usize,
    pub material_count: usize,
    pub draw_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelData {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub faces: Vec<TopologyFace>,
    pub tri_to_face: Vec<u32>,
    pub bounds: Option<Bounds>,
    pub stats: ModelStats,
    pub materials: Vec<MaterialInfo>,
    pub warnings: Vec<ModelWarning>,
}

impl ModelData {
    pub fn recompute_bounds(&mut self) {
        let mut bounds = Bounds::EMPTY;

        for vertex in &self.vertices {
            bounds.include_point(vertex.position);
        }

        self.bounds = (!bounds.is_empty()).then_some(bounds);
    }
}
