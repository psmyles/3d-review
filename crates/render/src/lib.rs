use glam::{Mat4, Vec2, Vec3};
use review_model::Bounds;

mod scene;

pub use scene::{SceneCallback, SCENE_DEPTH_FORMAT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView {
    Shaded,
    Wireframe,
    FaceNormals,
    VertexNormals,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneDebugOptions {
    pub wireframe: bool,
    pub face_normals: bool,
    pub vertex_normals: bool,
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
            // A neutral "review" home view: centered on the demo cube and close to
            // an isometric angle so proportions read correctly on reset.
            target: Vec3::new(0.0, 0.53, 0.0),
            yaw: -45.0_f32.to_radians(),
            pitch: -35.264_39_f32.to_radians(),
            distance: 7.5,
            aspect_ratio: 16.0 / 9.0,
            fov_y_radians: 50.0_f32.to_radians(),
            z_near: 0.02,
            z_far: 10_000.0,
        }
    }
}

impl OrbitCamera {
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    pub fn frame_bounds(&mut self, bounds: Bounds) {
        let center = bounds.center();
        let half_size = bounds.size() * 0.5;
        let rotation = self.rotation();
        let right = rotation.transform_vector3(Vec3::X).abs();
        let up = rotation.transform_vector3(Vec3::Y).abs();
        let forward = self.forward_dir().abs();
        let half_width = right.dot(half_size).max(0.25);
        let half_height = up.dot(half_size).max(0.25);
        let half_depth = forward.dot(half_size).max(0.25);
        let half_vertical_fov = (self.fov_y_radians * 0.5).clamp(0.01, 1.5);
        let half_horizontal_fov = (half_vertical_fov.tan() * self.aspect_ratio.max(0.1)).atan();
        let distance_for_height = half_height / half_vertical_fov.tan();
        let distance_for_width = half_width / half_horizontal_fov.tan();

        self.target = center;
        self.distance = (distance_for_width.max(distance_for_height) + half_depth) * 1.1;
        self.z_far = (self.distance + half_depth * 4.0).max(100.0);
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-1.5, 1.5);
    }

    pub fn zoom(&mut self, amount: f32) {
        let scale = (1.0 - amount * 0.1).clamp(0.2, 5.0);
        self.distance = (self.distance * scale).max(0.05);
    }

    pub fn pan_screen_delta(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 {
            return;
        }

        let rotation = self.rotation();
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let view_height = 2.0 * self.distance * (self.fov_y_radians * 0.5).tan();
        let view_width = view_height * self.aspect_ratio;
        let delta_x = delta_pixels.x / viewport_size.x * view_width;
        let delta_y = delta_pixels.y / viewport_size.y * view_height;

        self.target += (-right * delta_x) + (up * delta_y);
    }

    pub fn eye_position(self) -> Vec3 {
        self.target - self.forward_dir() * self.distance
    }

    pub fn forward_dir(self) -> Vec3 {
        self.rotation().transform_vector3(Vec3::NEG_Z)
    }

    pub fn view_matrix(self) -> Mat4 {
        let world_from_camera = Mat4::from_translation(self.eye_position()) * self.rotation();
        world_from_camera.inverse()
    }

    pub fn view_projection(self) -> Mat4 {
        let view = self.view_matrix();
        let projection = Mat4::perspective_rh(
            self.fov_y_radians,
            self.aspect_ratio,
            self.z_near,
            self.z_far,
        );

        projection * view
    }

    fn rotation(self) -> Mat4 {
        Mat4::from_rotation_y(self.yaw) * Mat4::from_rotation_x(self.pitch)
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
