use glam::{Mat4, Vec2, Vec3};
use review_model::Bounds;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView {
    Shaded,
    Wireframe,
    FaceNormals,
    VertexNormals,
}

#[derive(Debug, Clone, Copy)]
pub struct RendererConfig {
    pub preferred_backends: wgpu::Backends,
    pub clear_color: wgpu::Color,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            preferred_backends: wgpu::Backends::DX12
                | wgpu::Backends::VULKAN
                | wgpu::Backends::METAL,
            clear_color: wgpu::Color {
                r: 0.035,
                g: 0.037,
                b: 0.043,
                a: 1.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub aspect_ratio: f32,
    pub fov_y_radians: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: -0.65,
            pitch: -0.45,
            distance: 6.0,
            aspect_ratio: 16.0 / 9.0,
            fov_y_radians: 45.0_f32.to_radians(),
            z_near: 0.02,
            z_far: 10_000.0,
        }
    }
}

impl OrbitCamera {
    pub fn frame_bounds(&mut self, bounds: Bounds) {
        let radius = bounds.radius().max(0.5);
        self.target = bounds.center();
        self.distance = radius * 2.6;
        self.z_far = (radius * 12.0).max(100.0);
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-1.5, 1.5);
    }

    pub fn zoom(&mut self, amount: f32) {
        let scale = (1.0 - amount * 0.1).clamp(0.2, 5.0);
        self.distance = (self.distance * scale).max(0.05);
    }

    pub fn view_projection(self) -> Mat4 {
        let rotation = Mat4::from_rotation_y(self.yaw) * Mat4::from_rotation_x(self.pitch);
        let forward = rotation.transform_vector3(Vec3::NEG_Z);
        let eye = self.target - forward * self.distance;
        let view = Mat4::look_at_rh(eye, self.target, Vec3::Y);
        let projection = Mat4::perspective_rh(
            self.fov_y_radians,
            self.aspect_ratio,
            self.z_near,
            self.z_far,
        );

        projection * view
    }
}

#[derive(Debug)]
pub struct Renderer {
    pub config: RendererConfig,
    pub camera: OrbitCamera,
    pub debug_view: DebugView,
}

impl Renderer {
    pub fn new(config: RendererConfig) -> Self {
        Self {
            config,
            camera: OrbitCamera::default(),
            debug_view: DebugView::Shaded,
        }
    }
}
