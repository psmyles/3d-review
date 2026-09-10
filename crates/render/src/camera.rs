//! The viewer's camera math: [`OrbitCamera`] for the 3D viewport, [`UvCamera`] for
//! the 2D UV viewport, and the [`CameraTransition`] that eases between two 3D
//! framings.
//!
//! Pure `glam` math with no Direct3D types — the host-agnostic half of the crate
//! (invariant 9: no `unsafe` and no GPU plumbing here). [`Renderer`] owns the live
//! instances and drives them from `app`'s input intents.
//!
//! [`Renderer`]: crate::Renderer

use glam::{Mat4, Vec2, Vec3};
use review_model::Bounds;

use crate::{CameraProjection, GRID_BOUNDS, GRID_FAR_RADIUS};

const CAMERA_TRANSITION_SECONDS: f32 = 0.3;
/// Uniform breathing room left around a framed fit (4%), on top of any
/// safe-area inset, so content never sits hard against the viewport edges.
const FRAME_MARGIN: f32 = 1.04;

/// Largest far/near ratio we use when fitting the near plane. Perspective uses
/// infinite Reversed-Z, so the far value no longer clips geometry there; the
/// ratio still keeps the near plane from collapsing when framing tiny content and
/// provides the finite far range used by orthographic projection.
const MAX_DEPTH_RATIO: f32 = 5_000.0;
/// Absolute floor for the near plane so it never collapses to zero.
const MIN_Z_NEAR: f32 = 0.01;

/// Flycam speed (world units per second) per unit of framed scene radius, so a
/// 2 cm prop and a 200 m level move at the same *apparent* pace: at this rate a
/// straight run crosses the framed content's diameter in a little over a second.
const FLY_SPEED_PER_RADIUS: f32 = 1.5;
/// Floor under the scaled flycam speed, so framing something microscopic doesn't
/// leave the movement keys apparently dead. Sized to the near-plane floor.
const MIN_FLY_SPEED: f32 = 0.01;

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

    /// Rotate about the camera's **own eye position** — the first-person "look
    /// around" Unity and Unreal bind to a right-button drag, as against
    /// [`orbit`], which swings the eye around a stationary `target`.
    ///
    /// The eye is pinned and the pivot re-projected `distance` ahead along the new
    /// view direction, so a left-button orbit afterwards turns around whatever the
    /// user has just looked at rather than around where they came from.
    ///
    /// [`orbit`]: OrbitCamera::orbit
    pub fn look(&mut self, delta: Vec2) {
        let eye = self.eye_position();
        self.orbit(delta);
        self.target = eye + self.forward_dir() * self.distance;
    }

    /// Move the camera bodily through the scene, keeping its orientation — the
    /// WASD/QE flycam. `delta` is a world-unit displacement in camera axes: x
    /// right and z forward (both tilted with the view), and y **world** up, which
    /// is what makes Q/E rise and fall vertically however the camera is pitched.
    ///
    /// Only `target` moves; the eye is derived from it, so `distance` — and with
    /// it the near/far fit and the orbit pivot — travels along with the camera.
    pub fn fly(&mut self, delta: Vec3) {
        let right = self.rotation().transform_vector3(Vec3::X);
        self.target += right * delta.x + Vec3::Y * delta.y + self.forward_dir() * delta.z;
    }

    /// Base flycam speed in world units per second, scaled to the framed content
    /// so movement feels the same whatever real-world size the model is authored
    /// at. `app` multiplies this by the user's wheel-adjusted speed scale.
    pub fn fly_speed(self) -> f32 {
        (self.scene_radius * FLY_SPEED_PER_RADIUS).max(MIN_FLY_SPEED)
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
    /// Split out from [`view_projection`] so passes that work in view space (GTAO
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
pub(crate) struct CameraTransition {
    pub(crate) start: OrbitCamera,
    pub(crate) end: OrbitCamera,
    elapsed_seconds: f32,
    duration_seconds: f32,
}

impl CameraTransition {
    pub(crate) fn new(start: OrbitCamera, end: OrbitCamera) -> Self {
        Self {
            start,
            end,
            elapsed_seconds: 0.0,
            duration_seconds: CAMERA_TRANSITION_SECONDS,
        }
    }

    pub(crate) fn step(&mut self, delta_seconds: f32) -> (OrbitCamera, bool) {
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

/// The shared ease-in-out cubic curve: slow at both ends, fastest in the middle.
///
/// The camera transitions run on it, and it is re-exported from the crate root so
/// the chrome's own animations (the Tex viewport's fit) ease identically rather
/// than each keeping a private copy that can drift.
pub fn ease_in_out_cubic(t: f32) -> f32 {
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
    fn look_pivots_about_the_eye() {
        let mut camera = OrbitCamera::default();
        let eye = camera.eye_position();
        camera.look(Vec2::new(40.0, 25.0));

        // The eye is pinned and only the aim changes; the pivot rides `distance`
        // ahead of it along the new forward.
        assert!((camera.eye_position() - eye).length() < 1e-4);
        let expected_target = eye + camera.forward_dir() * camera.distance;
        assert!((camera.target - expected_target).length() < 1e-4);
    }

    #[test]
    fn fly_translates_eye_and_pivot_together() {
        let mut camera = OrbitCamera::default();
        let eye = camera.eye_position();
        let target = camera.target;
        let distance = camera.distance;
        camera.fly(Vec3::new(0.0, 0.5, 2.0));

        // A flight is a rigid translation: the eye/pivot separation (and so the
        // near/far fit) is untouched, and both moved by the same vector.
        assert!((camera.distance - distance).abs() < 1e-5);
        let moved = camera.target - target;
        assert!((camera.eye_position() - eye - moved).length() < 1e-4);
        // Forward is world-tilted by the default pitch, the vertical lane is not.
        assert!((moved - (camera.forward_dir() * 2.0 + Vec3::Y * 0.5)).length() < 1e-4);
    }

    #[test]
    fn fly_speed_scales_with_the_framed_content() {
        let mut small = OrbitCamera::default();
        small.frame_bounds(Bounds {
            min: Vec3::splat(-0.01),
            max: Vec3::splat(0.01),
        });
        let mut large = OrbitCamera::default();
        large.frame_bounds(Bounds {
            min: Vec3::splat(-100.0),
            max: Vec3::splat(100.0),
        });

        assert!(large.fly_speed() > small.fly_speed() * 100.0);
        assert!(small.fly_speed() >= MIN_FLY_SPEED);
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
