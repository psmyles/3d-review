use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Mat4, Vec2, Vec3};
use review_model::{Bounds, MaterialImportDefaults};

mod bloom;
mod config;
mod geometry;
mod ibl;
mod material;
mod mipmap;
mod post;
mod scene;
mod selection;
mod ssao;
mod targets;
mod tex;
mod texture;

pub use config::*;
pub use ibl::ibl_supported;
pub use material::{
    AlphaMode, MaterialChange, MaterialEdit, MaterialSnapshot, MaterialState, RoughnessWorkflow,
    TextureBinding,
};
pub use scene::{EGUI_DEPTH_FORMAT, EGUI_MSAA_SAMPLE_COUNT, SCENE_DEPTH_FORMAT, SceneCallback};
pub use selection::{Selection, SelectionView};
pub use ssao::ssao_supported;
pub use tex::TexCallback;
pub use texture::{
    ChannelSelect, DecodedImage, TEXTURE_SLOT_COUNT, TextureSlot, decode_image, suggested_channel,
};

const CAMERA_TRANSITION_SECONDS: f32 = 0.3;
/// Shorter transition used for the WASD 45° orbit steps, which fire repeatedly
/// and want a snappier response than the default framing/snap animation.
const ORBIT_TRANSITION_SECONDS: f32 = 0.1;
/// Uniform breathing room left around a framed fit (4%), on top of any
/// safe-area inset, so content never sits hard against the viewport edges.
const FRAME_MARGIN: f32 = 1.04;
/// Fraction of the safe area the empty "home" grid view fills. Below 1.0 so the
/// reference grid sits comfortably back in the viewport with margin around it,
/// rather than filling the window edge-to-edge. Only affects the home/reset
/// view — loaded models still frame tight to the safe area.
const HOME_FILL_FRACTION: f32 = 0.68;

/// Largest far/near ratio we use when fitting the near plane. Perspective uses
/// infinite Reversed-Z, so the far value no longer clips geometry there; the
/// ratio still keeps the near plane from collapsing when framing tiny content and
/// provides the finite far range used by orthographic projection.
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

/// Default half-height (in UV units) of the UV viewport, so the unit square is
/// shown with comfortable margin around it. The visible vertical span is twice
/// this; `> 0.5` leaves the 0..1 square framed back from the edges.
const UV_DEFAULT_HALF_HEIGHT: f32 = 0.72;
/// Clamp range for the UV camera's half-height so zoom can't invert or run away.
const UV_MIN_HALF_HEIGHT: f32 = 0.02;
const UV_MAX_HALF_HEIGHT: f32 = 50.0;

/// A 2D pan/zoom camera for the UV viewport. Maps UV space (the 0..1 unit square
/// the model's UVs live in) to the screen with an aspect-corrected orthographic
/// projection, so the unit square always stays square regardless of window
/// shape. `center` is the UV point shown at the viewport center; `half_height`
/// is half the visible vertical span in UV units (smaller = zoomed in).
#[derive(Debug, Clone, Copy)]
pub struct UvCamera {
    pub center: Vec2,
    pub half_height: f32,
    pub aspect_ratio: f32,
}

impl Default for UvCamera {
    fn default() -> Self {
        Self {
            // Center on the middle of the 0..1 unit square.
            center: Vec2::new(0.5, 0.5),
            half_height: UV_DEFAULT_HALF_HEIGHT,
            aspect_ratio: 16.0 / 9.0,
        }
    }
}

impl UvCamera {
    /// Reset to the default framing, preserving the live aspect ratio.
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    fn half_width(self) -> f32 {
        self.half_height * self.aspect_ratio.max(0.1)
    }

    /// Pan the view by a pointer drag (pixels), keeping the grabbed UV point
    /// under the cursor: the content follows the drag direction.
    pub fn pan_screen_delta(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 {
            return;
        }
        let du = delta_pixels.x / viewport_size.x * (2.0 * self.half_width());
        let dv = delta_pixels.y / viewport_size.y * (2.0 * self.half_height);
        // Drag right (+x) shows lower-u content at center; drag down (+y, with v
        // up) shows higher-v content at center.
        self.center.x -= du;
        self.center.y += dv;
    }

    /// Zoom about the view center. Positive `amount` zooms in (matches the orbit
    /// camera's wheel/zoom-drag sign), shrinking the visible span.
    pub fn zoom(&mut self, amount: f32) {
        let scale = (1.0 - amount * 0.1).clamp(0.2, 5.0);
        self.half_height = (self.half_height * scale).clamp(UV_MIN_HALF_HEIGHT, UV_MAX_HALF_HEIGHT);
    }

    /// Aspect-corrected orthographic view-projection mapping UV-plane points
    /// `(u, v, 0)` to clip space, with v pointing up like a UV editor.
    pub fn view_projection(self) -> Mat4 {
        let half_w = self.half_width();
        Mat4::orthographic_rh(
            self.center.x - half_w,
            self.center.x + half_w,
            self.center.y - self.half_height,
            self.center.y + self.half_height,
            -1.0,
            1.0,
        )
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
        base.framed_to_bounds(GRID_BOUNDS, Vec2::ONE)
    }
}

impl OrbitCamera {
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    pub fn frame_bounds(&mut self, bounds: Bounds) {
        *self = self.framed_to_bounds(bounds, Vec2::ONE);
    }

    /// Frame the camera so `bounds` fills the viewport, tight and centred.
    ///
    /// `safe_area` is the fraction of the viewport (x = width, y = height) that
    /// framing should aim to fill — `Vec2::ONE` is the whole window. The 3D
    /// scene is painted full-window with the toolbar / status-bar chrome drawn
    /// *over* its top and bottom, so passing the visible fraction there keeps
    /// the model out from under the chrome.
    pub fn framed_to_bounds(mut self, bounds: Bounds, safe_area: Vec2) -> Self {
        let center = bounds.center();
        let half_size = bounds.size() * 0.5;
        let rotation = self.rotation();
        // Camera basis. `forward` points from the eye toward (and past) the
        // target, so a corner's depth from the eye is `distance + forward·c`.
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let forward = self.forward_dir();
        // Loosen the usable FOV by the safe-area fractions and the uniform
        // margin, so the silhouette is fit *inside* the visible band rather than
        // the full window.
        let safe_h = (safe_area.x / FRAME_MARGIN).clamp(0.05, 1.0);
        let safe_v = (safe_area.y / FRAME_MARGIN).clamp(0.05, 1.0);
        let half_fov = (self.fov_y_radians * 0.5).clamp(0.01, 1.5).tan();
        let tan_v = half_fov * safe_v;
        let tan_h = half_fov * self.aspect_ratio.max(0.1) * safe_h;

        // Per-corner offsets from the box centre projected onto the camera
        // basis: `(u, v, w)` = (right·c, up·c, forward·c). `w` (depth along
        // forward) is unaffected by a lateral re-centring pan, so it's computed
        // once; `u`/`v` shift uniformly as the target pans.
        let mut corners = [(0.0_f32, 0.0_f32, 0.0_f32); 8];
        let mut i = 0;
        for cx in [-1.0_f32, 1.0] {
            for cy in [-1.0_f32, 1.0] {
                for cz in [-1.0_f32, 1.0] {
                    let c = Vec3::new(cx * half_size.x, cy * half_size.y, cz * half_size.z);
                    corners[i] = (right.dot(c), up.dot(c), forward.dot(c));
                    i += 1;
                }
            }
        }

        // Solve the fit distance *and* a lateral re-centring pan together. The
        // per-corner screen-fill constraint is
        //   |u| <= tan_h * (distance + w)   and   |v| <= tan_v * (distance + w)
        // so distance >= |u|/tan_h - w (and likewise for v). A pure box-centre
        // fit (pan = 0) makes the *nearest* extreme corner touch the edge while
        // the far corner leaves a gap — perspective magnifies near geometry, so
        // the silhouette is not actually centred. Each pass fits the smallest
        // distance for the current pan, then shifts the pan so the projected
        // silhouette straddles the centre. The depth term makes the re-centre
        // non-linear, so iterate to a fixed point (cheap: 8 corners, 6 passes).
        let mut pan_u = 0.0_f32;
        let mut pan_v = 0.0_f32;
        let mut distance = 0.05_f32;
        for _ in 0..6 {
            distance = 0.0;
            for &(u, v, w) in &corners {
                distance = distance
                    .max((u + pan_u).abs() / tan_h - w)
                    .max((v + pan_v).abs() / tan_v - w);
            }
            distance = distance.max(0.05);
            pan_u +=
                silhouette_recenter(corners.iter().map(|&(u, _, w)| (u + pan_u, distance + w)));
            pan_v +=
                silhouette_recenter(corners.iter().map(|&(_, v, w)| (v + pan_v, distance + w)));
        }

        // Pan the target laterally by the solved offset (eye follows, so depth
        // along forward is unchanged) to centre the projected silhouette.
        self.target = center - right * pan_u - up * pan_v;
        self.distance = distance;
        // Bounding-sphere radius around the box centre (corner distance). Drives
        // the per-frame near/far fit in `near_far`.
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
        self.projection_matrix(projection_mode) * self.view_matrix()
    }

    /// The projection matrix alone (view → clip), fit to the current near/far.
    /// Split out from [`view_projection`] so passes that work in view space (SSAO
    /// reconstructs view-space position from this and projects sample points back
    /// through it) can get the projection without the view baked in.
    ///
    /// [`view_projection`]: OrbitCamera::view_projection
    pub fn projection_matrix(self, projection_mode: CameraProjection) -> Mat4 {
        let (z_near, z_far) = self.near_far();
        match projection_mode {
            CameraProjection::Perspective => {
                Mat4::perspective_infinite_reverse_rh(self.fov_y_radians, self.aspect_ratio, z_near)
            }
            CameraProjection::Orthographic => {
                let half_height = self.orthographic_half_height();
                let half_width = half_height * self.aspect_ratio.max(0.1);
                Mat4::orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    z_far,
                    z_near,
                )
            }
        }
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
        Self::with_duration(start, end, CAMERA_TRANSITION_SECONDS)
    }

    fn with_duration(start: OrbitCamera, end: OrbitCamera, duration_seconds: f32) -> Self {
        Self {
            start,
            end,
            elapsed_seconds: 0.0,
            duration_seconds,
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

/// Additional lateral pan that makes the two screen-space extreme corners on one
/// axis straddle the viewport centre symmetrically. Each item is `(lateral,
/// depth)` for a corner — `lateral` already includes the running pan, `depth` is
/// `distance + forward·offset`. Returns the extra pan to apply on that axis.
fn silhouette_recenter(corners: impl Iterator<Item = (f32, f32)>) -> f32 {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let mut lo_corner = (0.0_f32, 1.0_f32);
    let mut hi_corner = (0.0_f32, 1.0_f32);
    for (lateral, depth) in corners {
        let depth = depth.max(1e-3);
        let screen = lateral / depth;
        if screen < lo {
            lo = screen;
            lo_corner = (lateral, depth);
        }
        if screen > hi {
            hi = screen;
            hi_corner = (lateral, depth);
        }
    }
    let (lat_lo, depth_lo) = lo_corner;
    let (lat_hi, depth_hi) = hi_corner;
    // Solve Δ so (lat_lo + Δ)/depth_lo = -(lat_hi + Δ)/depth_hi, i.e. the two
    // extreme corners project to equal-and-opposite screen offsets.
    -(lat_lo * depth_hi + lat_hi * depth_lo) / (depth_lo + depth_hi)
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ndc_z(projection: Mat4, view_z: f32) -> f32 {
        let clip = projection * Vec3::new(0.0, 0.0, view_z).extend(1.0);
        clip.z / clip.w
    }

    #[test]
    fn perspective_projection_uses_reversed_z() {
        let camera = OrbitCamera::default();
        let (near, _) = camera.near_far();
        let projection = camera.projection_matrix(CameraProjection::Perspective);

        assert!((ndc_z(projection, -near) - 1.0).abs() < 1e-5);
        let distant = ndc_z(projection, -near * 1_000.0);
        assert!(distant > 0.0 && distant < 0.01);
    }

    #[test]
    fn orthographic_projection_uses_reversed_z() {
        let camera = OrbitCamera::default();
        let (near, far) = camera.near_far();
        let projection = camera.projection_matrix(CameraProjection::Orthographic);

        assert!((ndc_z(projection, -near) - 1.0).abs() < 1e-5);
        assert!(ndc_z(projection, -far).abs() < 1e-5);
    }
}

#[derive(Debug)]
pub struct Renderer {
    pub config: RendererConfig,
    pub camera: OrbitCamera,
    /// The 2D camera for the UV viewport, independent of the 3D orbit camera.
    pub uv_camera: UvCamera,
    camera_transition: Option<CameraTransition>,
    /// Fraction of the viewport (x = width, y = height) framing should fill,
    /// leaving room for the chrome that overlays the full-window 3D scene. Set
    /// by `app` from the live window + chrome sizes; `Vec2::ONE` = whole window.
    framing_safe_area: Vec2,
    /// Editable per-material PBR parameters, seeded from the loaded model's import
    /// defaults and edited live via [`MaterialEdit`] intents (invariant 2). Carried
    /// into the scene callback each frame; the renderer-side table re-uploads them
    /// when `material_revision` changes.
    material_states: Vec<MaterialState>,
    /// Display names paired with `material_states`, for the app→UI snapshot.
    material_names: Vec<String>,
    /// Bumped on every material edit (and on model load) so the GPU table is
    /// re-uploaded without a full mesh rebuild.
    material_revision: u64,
}

impl Renderer {
    pub fn new(config: RendererConfig) -> Self {
        Self {
            config,
            camera: OrbitCamera::default(),
            uv_camera: UvCamera::default(),
            camera_transition: None,
            framing_safe_area: Vec2::ONE,
            material_states: Vec::new(),
            material_names: Vec::new(),
            material_revision: 0,
        }
    }

    /// Seed the editable material table from a freshly loaded model's import
    /// defaults (or clear it for an empty model). Bumps the material revision so
    /// the GPU table is rebuilt/re-uploaded on the next frame.
    pub fn set_model_materials(&mut self, materials: &[MaterialImportDefaults]) {
        self.material_states = materials
            .iter()
            .map(|material| MaterialState {
                base_color: material.base_color,
                metallic: material.metallic,
                // Roughness is the complement of the imported glossiness.
                roughness: (1.0 - material.smoothness).clamp(0.0, 1.0),
                emissive: material.emissive,
                ..MaterialState::default()
            })
            .collect();
        self.material_names = materials
            .iter()
            .map(|material| material.name.clone())
            .collect();
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Apply one UI material-edit intent to the editable table, bumping the
    /// revision so the renderer re-uploads. Out-of-range indices are ignored.
    pub fn set_material_param(&mut self, edit: MaterialEdit) {
        let Some(state) = self.material_states.get_mut(edit.index) else {
            return;
        };
        match edit.change {
            MaterialChange::BaseColor(rgb) => state.base_color = Vec3::from_array(rgb),
            MaterialChange::Metallic(value) => state.metallic = value.clamp(0.0, 1.0),
            MaterialChange::Roughness(value) => state.roughness = value.clamp(0.0, 1.0),
            MaterialChange::Emissive(rgb) => state.emissive = Vec3::from_array(rgb),
            MaterialChange::Channel(slot, channel) => {
                // Re-route an already-assigned slot; ignored if the slot is empty
                // or out of range.
                if let Some(Some(binding)) = state.textures.get_mut(slot) {
                    binding.channel = channel;
                }
            }
            MaterialChange::AlphaMode(mode) => state.alpha_mode = mode,
            MaterialChange::AlphaCutoff(value) => state.alpha_cutoff = value.clamp(0.0, 1.0),
            MaterialChange::Workflow(workflow) => state.workflow = workflow,
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// The editable material parameters, carried into the scene callback each frame.
    pub fn material_states(&self) -> &[MaterialState] {
        &self.material_states
    }

    /// The current material revision (bumped on edit / load).
    pub fn material_revision(&self) -> u64 {
        self.material_revision
    }

    /// A name+value snapshot of the editable materials for the UI (invariant 2:
    /// the UI reads this plain value, never renderer-owned state).
    pub fn material_snapshot(&self) -> Vec<MaterialSnapshot> {
        self.material_names
            .iter()
            .cloned()
            .zip(self.material_states.iter().cloned())
            .map(|(name, state)| MaterialSnapshot { name, state })
            .collect()
    }

    /// Assign (or replace) a decoded image to one of a material's seven texture
    /// slots, with the chosen channel routing. The image is shared by `Arc` (the
    /// app decodes once and may reuse it across slots / materials). Bumps the
    /// revision so the GPU table uploads + rebinds on the next frame. Out-of-range
    /// material indices are ignored.
    pub fn set_texture_slot(
        &mut self,
        material: usize,
        slot: TextureSlot,
        path: PathBuf,
        image: Arc<DecodedImage>,
        channel: ChannelSelect,
    ) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = Some(TextureBinding {
            path,
            image,
            channel,
        });
        // Assigning an opacity map switches the material to alpha-blend so the
        // translucency shows; the Inspector can switch it to Clip.
        if slot == TextureSlot::Opacity && state.alpha_mode == AlphaMode::Opaque {
            state.alpha_mode = AlphaMode::Blend;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Clear a material's texture slot back to the shader's neutral fallback.
    pub fn clear_texture_slot(&mut self, material: usize, slot: TextureSlot) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = None;
        // Clearing the opacity map restores opaque compositing.
        if slot == TextureSlot::Opacity {
            state.alpha_mode = AlphaMode::Opaque;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Replace the decoded image of every texture binding that references `path`
    /// (across all materials / slots) with `image` — the disk-auto-reload path.
    /// Keeps each binding's channel routing. Returns `true` (and bumps the
    /// revision) when at least one binding matched.
    pub fn reload_texture(&mut self, path: &Path, image: Arc<DecodedImage>) -> bool {
        let mut changed = false;
        for state in &mut self.material_states {
            for binding in state.textures.iter_mut().flatten() {
                if binding.path == path {
                    binding.image = Arc::clone(&image);
                    changed = true;
                }
            }
        }
        if changed {
            self.material_revision = self.material_revision.wrapping_add(1);
        }
        changed
    }

    /// Every distinct source path currently bound to a material slot (for the disk
    /// watcher to register / reconcile).
    pub fn texture_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        for state in &self.material_states {
            for binding in state.textures.iter().flatten() {
                if !paths.contains(&binding.path) {
                    paths.push(binding.path.clone());
                }
            }
        }
        paths
    }

    pub fn set_uv_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.uv_camera.aspect_ratio = aspect_ratio;
    }

    pub fn pan_uv_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.uv_camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_uv_camera(&mut self, amount: f32) {
        self.uv_camera.zoom(amount);
    }

    pub fn reset_uv_camera(&mut self) {
        self.uv_camera.reset();
    }

    /// Set the fraction of the viewport that subsequent framing should fill, so
    /// the model lands inside the band left visible by the toolbar / status bar.
    pub fn set_framing_safe_area(&mut self, width_fraction: f32, height_fraction: f32) {
        self.framing_safe_area = Vec2::new(width_fraction, height_fraction);
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
        self.animate_camera_to(self.camera.framed_to_bounds(bounds, self.framing_safe_area));
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
    }

    /// Animate a relative orbit by the given yaw / pitch deltas (radians). Based
    /// off any in-flight transition's target (not the mid-flight camera) so
    /// repeated key presses chain into successive 45° steps. Pitch is clamped to
    /// match interactive [`OrbitCamera::orbit`]. Uses the shorter
    /// [`ORBIT_TRANSITION_SECONDS`] so each step feels snappy.
    pub fn animate_orbit_by(&mut self, yaw_delta: f32, pitch_delta: f32) {
        let mut end = self.camera_transition.map_or(self.camera, |t| t.end);
        end.yaw += yaw_delta;
        end.pitch = (end.pitch + pitch_delta).clamp(-1.5, 1.5);
        self.camera_transition = Some(CameraTransition::with_duration(
            self.camera,
            end,
            ORBIT_TRANSITION_SECONDS,
        ));
    }

    /// The default "home" view, re-framed for the live aspect ratio and the
    /// chrome-aware safe area so the whole grid stays visible regardless of
    /// window shape. Shared by the animated reset and the instant startup frame
    /// so both land on exactly the same view.
    fn home_camera(&self) -> OrbitCamera {
        let home = OrbitCamera {
            aspect_ratio: self.camera.aspect_ratio,
            ..OrbitCamera::default()
        };
        // Fill only a fraction of the safe area so the grid sits back from the
        // edges (see HOME_FILL_FRACTION) instead of filling the window.
        home.framed_to_bounds(GRID_BOUNDS, self.framing_safe_area * HOME_FILL_FRACTION)
    }

    /// Animate back to the home view.
    pub fn animate_camera_to_home(&mut self) {
        self.animate_camera_to(self.home_camera());
    }

    /// Snap (no animation) to the home view. Used at startup once the real
    /// window size / safe area are known, so the initial frame matches the
    /// reset view rather than the full-window `OrbitCamera::default` framing.
    pub fn reset_camera_to_home(&mut self) {
        self.camera_transition = None;
        self.camera = self.home_camera();
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
