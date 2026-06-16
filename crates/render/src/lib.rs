use glam::{Mat4, Vec2, Vec3};
use review_model::Bounds;

mod geometry;
mod scene;

pub use scene::{SCENE_DEPTH_FORMAT, SCENE_SAMPLE_COUNT, SceneCallback};

const CAMERA_TRANSITION_SECONDS: f32 = 0.3;

/// Largest far/near ratio we let the projection produce. The depth buffer
/// (`Depth24Plus`) only has so many distinguishable values; a huge range spends
/// almost all of them in empty space in front of the model, leaving close and
/// intersecting faces to flicker / swap draw order as you zoom. Bounding the
/// ratio keeps enough precision across the model. ~5000:1 is comfortable for a
/// 24-bit depth buffer.
const MAX_DEPTH_RATIO: f32 = 5_000.0;
/// Absolute floor for the near plane so it never collapses to zero.
const MIN_Z_NEAR: f32 = 0.01;
/// Half-extent of the static reference grid: a 2 m square floor (±1 m) ruled in
/// 10 cm cells. Shared with `geometry::scene_lines` and the home-view framing so
/// the grid's size is defined in exactly one place. World units are meters.
pub(crate) const GRID_HALF_EXTENT: f32 = 1.0;
/// Axis-aligned bounds of that flat grid, used to frame the empty "home" view so
/// the whole floor is visible on launch and on reset.
const GRID_BOUNDS: Bounds = Bounds {
    min: Vec3::new(-GRID_HALF_EXTENT, 0.0, -GRID_HALF_EXTENT),
    max: Vec3::new(GRID_HALF_EXTENT, 0.0, GRID_HALF_EXTENT),
};
/// Worst-case radius of the grid (its corner, ~1.41 m) with margin. The far
/// plane must reach it so the grid isn't clipped behind small models.
const GRID_FAR_RADIUS: f32 = GRID_HALF_EXTENT * 2.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShadingMode {
    Wireframe,
    Unlit,
    #[default]
    Shaded,
    ShadedWireframe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CameraProjection {
    #[default]
    Perspective,
    Orthographic,
}

/// Which built-in checker texture the UV-checker view samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckerTexture {
    #[default]
    Greyscale,
    Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDebugOptions {
    pub shading_mode: ShadingMode,
    pub uv_checker: bool,
    pub uv_checker_texture: CheckerTexture,
    /// Checker repeats across the 0..1 UV range; clamped to 1..=16 by the UI.
    pub uv_checker_tiling: u32,
    /// Which model UV set the checker view samples (0-based). Only meaningful
    /// when the model carries more than one UV set.
    pub uv_channel: u32,
    pub show_grid: bool,
    /// Whether the model's axis-aligned bounding box is drawn as a wireframe box.
    pub show_bounding_box: bool,
    pub face_normals: bool,
    pub vertex_normals: bool,
    pub face_normal_length: f32,
    pub vertex_normal_length: f32,
    pub face_normal_color: [f32; 4],
    pub vertex_normal_color: [f32; 4],
    /// Color of the wireframe lines (wireframe / shaded-wireframe modes), baked
    /// into the line vertex buffer and rebuilt when it changes.
    pub wireframe_color: [f32; 4],
    /// Color of the bounding-box edges, baked into its line buffer and rebuilt
    /// when it changes.
    pub bounding_box_color: [f32; 4],
}

impl Default for SceneDebugOptions {
    fn default() -> Self {
        Self {
            shading_mode: ShadingMode::Shaded,
            uv_checker: false,
            uv_checker_texture: CheckerTexture::Greyscale,
            uv_checker_tiling: 4,
            uv_channel: 0,
            show_grid: true,
            show_bounding_box: false,
            face_normals: false,
            vertex_normals: false,
            face_normal_length: 0.18,
            vertex_normal_length: 0.18,
            face_normal_color: [1.0, 0.1, 0.1, 0.95],
            vertex_normal_color: [0.14, 0.92, 0.96, 0.95],
            wireframe_color: [1.0, 1.0, 1.0, 1.0],
            bounding_box_color: [1.0, 0.803_921_6, 0.250_980_4, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RendererConfig {
    pub preferred_backends: wgpu::Backends,
    pub clear_color: wgpu::Color,
}

impl Default for RendererConfig {
    fn default() -> Self {
        // On Windows, request DX12 only: adapter/device creation is ~100 ms
        // cheaper than bringing up the Vulkan loader + ICD (measured on an RTX
        // 4080), and DX12 is guaranteed on Windows 10+. Other platforms keep
        // Vulkan/Metal. This is the "prefer DX12 on Windows" decision in
        // CLAUDE.md §4, now enforced rather than left to adapter selection.
        #[cfg(windows)]
        let preferred_backends = wgpu::Backends::DX12;
        #[cfg(not(windows))]
        let preferred_backends = wgpu::Backends::VULKAN | wgpu::Backends::METAL;

        Self {
            preferred_backends,
            clear_color: wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
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
    /// Bounding-sphere radius of the framed content around `target`. The near /
    /// far planes are fit to this each frame (see `near_far`) instead of being
    /// stored, so depth precision stays optimal as `distance` changes on zoom.
    pub scene_radius: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // A neutral "review" home view: a front-right isometric angle, framed so
        // the whole 2 m reference grid is visible when the scene is empty.
        let base = Self {
            target: Vec3::ZERO,
            // +45° yaw parks the eye in the +X/+Y/+Z octant (front-right iso): +X
            // reads lower-right, +Z lower-left, both facing the viewer. (-45° is
            // the mirror image and shows the -X side instead.)
            yaw: 45.0_f32.to_radians(),
            pitch: -35.264_39_f32.to_radians(),
            // `distance` / `scene_radius` are recomputed by `framed_to_bounds`.
            distance: 7.5,
            aspect_ratio: 16.0 / 9.0,
            fov_y_radians: 50.0_f32.to_radians(),
            scene_radius: 1.0,
        };
        base.framed_to_bounds(GRID_BOUNDS)
    }
}

impl OrbitCamera {
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    pub fn frame_bounds(&mut self, bounds: Bounds) {
        *self = self.framed_to_bounds(bounds);
    }

    pub fn framed_to_bounds(mut self, bounds: Bounds) -> Self {
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
        // Bounding-sphere radius around `target` (corner distance). Drives the
        // per-frame near/far fit in `near_far`.
        self.scene_radius = half_size.length().max(0.001);
        self
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-1.5, 1.5);
    }

    pub fn set_offset_direction(&mut self, direction: Vec3) {
        *self = self.with_offset_direction(direction);
    }

    pub fn with_offset_direction(mut self, direction: Vec3) -> Self {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON {
            return self;
        }

        self.pitch = (-direction.y).asin().clamp(-1.5, 1.5);
        self.yaw = direction.x.atan2(direction.z);
        self
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

    pub fn view_space_direction(self, direction: Vec3) -> Vec3 {
        self.rotation().inverse().transform_vector3(direction)
    }

    pub fn view_matrix(self) -> Mat4 {
        let world_from_camera = Mat4::from_translation(self.eye_position()) * self.rotation();
        world_from_camera.inverse()
    }

    /// Near / far planes fit to the current view each frame. The far plane
    /// reaches past the content (model *and* the reference grid); the near plane
    /// is pushed as far forward as a bounded far/near ratio allows so the depth
    /// buffer keeps its precision across the model regardless of zoom. This is
    /// what prevents close / intersecting faces from flickering and swapping
    /// draw order — a fixed tiny near plane with a huge far plane does not.
    pub fn near_far(self) -> (f32, f32) {
        // Far must clear the grid even when the model is tiny.
        let content_radius = self.scene_radius.max(GRID_FAR_RADIUS);
        let z_far = (self.distance + content_radius).max(MIN_Z_NEAR * 2.0);
        let z_near = (z_far / MAX_DEPTH_RATIO).max(MIN_Z_NEAR);
        (z_near, z_far)
    }

    pub fn view_projection(self, projection_mode: CameraProjection) -> Mat4 {
        let view = self.view_matrix();
        let (z_near, z_far) = self.near_far();
        let projection = match projection_mode {
            CameraProjection::Perspective => {
                Mat4::perspective_rh(self.fov_y_radians, self.aspect_ratio, z_near, z_far)
            }
            CameraProjection::Orthographic => {
                let half_height = self.orthographic_half_height();
                let half_width = half_height * self.aspect_ratio.max(0.1);
                Mat4::orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    z_near,
                    z_far,
                )
            }
        };

        projection * view
    }

    fn orthographic_half_height(self) -> f32 {
        (self.distance * (self.fov_y_radians * 0.5).tan()).max(0.001)
    }

    fn rotation(self) -> Mat4 {
        Mat4::from_rotation_y(self.yaw) * Mat4::from_rotation_x(self.pitch)
    }
}

#[derive(Debug, Clone, Copy)]
struct CameraTransition {
    start: OrbitCamera,
    end: OrbitCamera,
    elapsed_seconds: f32,
    duration_seconds: f32,
}

impl CameraTransition {
    fn new(start: OrbitCamera, end: OrbitCamera) -> Self {
        Self {
            start,
            end,
            elapsed_seconds: 0.0,
            duration_seconds: CAMERA_TRANSITION_SECONDS,
        }
    }

    fn step(&mut self, delta_seconds: f32) -> (OrbitCamera, bool) {
        self.elapsed_seconds = (self.elapsed_seconds + delta_seconds).min(self.duration_seconds);
        let t = if self.duration_seconds <= 0.0 {
            1.0
        } else {
            self.elapsed_seconds / self.duration_seconds
        };
        let eased = ease_in_out_cubic(t);
        let finished = self.elapsed_seconds >= self.duration_seconds;
        (lerp_camera(self.start, self.end, eased), finished)
    }
}

fn lerp_camera(start: OrbitCamera, end: OrbitCamera, t: f32) -> OrbitCamera {
    OrbitCamera {
        target: start.target.lerp(end.target, t),
        yaw: lerp_angle(start.yaw, end.yaw, t),
        pitch: start.pitch + (end.pitch - start.pitch) * t,
        distance: start.distance + (end.distance - start.distance) * t,
        aspect_ratio: end.aspect_ratio,
        fov_y_radians: start.fov_y_radians + (end.fov_y_radians - start.fov_y_radians) * t,
        scene_radius: start.scene_radius + (end.scene_radius - start.scene_radius) * t,
    }
}

fn lerp_angle(start: f32, end: f32, t: f32) -> f32 {
    let delta = (end - start + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    start + delta * t
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

#[derive(Debug)]
pub struct Renderer {
    pub config: RendererConfig,
    pub camera: OrbitCamera,
    camera_transition: Option<CameraTransition>,
}

impl Renderer {
    pub fn new(config: RendererConfig) -> Self {
        Self {
            config,
            camera: OrbitCamera::default(),
            camera_transition: None,
        }
    }

    pub fn orbit_camera(&mut self, delta: Vec2) {
        self.camera_transition = None;
        self.camera.orbit(delta);
    }

    pub fn set_camera_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.camera.aspect_ratio = aspect_ratio;
        if let Some(transition) = self.camera_transition.as_mut() {
            transition.start.aspect_ratio = aspect_ratio;
            transition.end.aspect_ratio = aspect_ratio;
        }
    }

    pub fn pan_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.camera_transition = None;
        self.camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_camera(&mut self, amount: f32) {
        self.camera_transition = None;
        self.camera.zoom(amount);
    }

    pub fn animate_camera_to(&mut self, end: OrbitCamera) {
        self.camera_transition = Some(CameraTransition::new(self.camera, end));
    }

    pub fn animate_camera_to_bounds(&mut self, bounds: Bounds) {
        self.animate_camera_to(self.camera.framed_to_bounds(bounds));
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
    }

    /// Animate back to the default "home" view, re-framing the grid for the live
    /// aspect ratio so the whole floor stays visible regardless of window shape.
    pub fn animate_camera_to_home(&mut self) {
        let home = OrbitCamera {
            aspect_ratio: self.camera.aspect_ratio,
            ..OrbitCamera::default()
        };
        self.animate_camera_to(home.framed_to_bounds(GRID_BOUNDS));
    }

    pub fn update_camera_animation(&mut self, delta_seconds: f32) -> bool {
        let Some(transition) = self.camera_transition.as_mut() else {
            return false;
        };

        let (camera, finished) = transition.step(delta_seconds);
        self.camera = camera;
        if finished {
            self.camera_transition = None;
        }
        true
    }

    pub fn is_camera_animating(&self) -> bool {
        self.camera_transition.is_some()
    }
}
