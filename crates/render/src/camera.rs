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

/// The 3D cameras' vertical field of view until the user picks another, in
/// degrees. Public because the chrome's Camera options panel resets to it, and a
/// second copy there would drift from the one the cameras start with.
pub const DEFAULT_FOV_Y_DEGREES: f32 = 50.0;

const CAMERA_TRANSITION_SECONDS: f32 = 0.3;
/// Uniform breathing room left around a framed fit (4%), inside the visible
/// area, so content never sits hard against the chrome's edges.
const FRAME_MARGIN: f32 = 1.04;
/// The narrowest a framing area may get, as a half-span in normalised device
/// coordinates: a window squeezed down to almost nothing between its panels
/// still frames the model at 5% of its width rather than at infinity.
const MIN_FRAMING_HALF_SPAN: f32 = 0.05;
/// Halvings the centring pan is solved to. The bracket is the pan range the fit
/// allows, so this resolves it to far below a pixel at any scale.
const CENTRING_STEPS: usize = 48;
/// Floor under the share of an orthographic framing's distance that is left to
/// clear the model once the pivot's own offset is paid for. An area centred so
/// far off the view axis that the pivot runs away from the model as fast as the
/// eye backs off has no distance that clears it; this keeps that one finite.
const MIN_CLEARANCE_SHARE: f32 = 0.25;

/// Largest far/near ratio we use when fitting the near plane. Perspective uses
/// infinite Reversed-Z, so the far value no longer clips geometry there; the
/// ratio still keeps the near plane from collapsing when framing tiny content and
/// provides the finite far range used by orthographic projection.
const MAX_DEPTH_RATIO: f32 = 5_000.0;
/// Fraction of the orbit distance the near plane may sit at once a zoom brings
/// the pivot in closer than the depth-ratio fit above would clear. Below 1.0 so
/// the pivot — and a margin in front of it — always stays inside the frustum.
/// Without this the ratio fit is what ends up stopping a zoom-in, and it stops
/// it *further out* the larger the scene is: framing a 500 m level puts the near
/// plane at 10 cm, so the surface being inspected clips away while the camera
/// still has distance left to give.
const NEAR_PIVOT_FRACTION: f32 = 0.25;
/// Absolute floor for the near plane so it never collapses to zero.
const MIN_Z_NEAR: f32 = 1.0e-5;
/// Floor under the orbit distance, so the eye can never reach or pass its own
/// pivot. Deliberately far below any scale a model is authored at (0.1 mm, world
/// units being meters) — inspecting a rivet or a texel of trim is exactly what
/// this viewer is for, so nothing but the user should stop a zoom-in. Shared
/// with the framing fit, where it guards degenerate (zero-size) bounds.
const MIN_ORBIT_DISTANCE: f32 = 1.0e-4;
/// Guards a degenerate zero-span orthographic projection. Sized under what
/// [`MIN_ORBIT_DISTANCE`] yields, so it never cuts an ortho zoom short of what
/// the perspective view allows.
const MIN_ORTHO_HALF_HEIGHT: f32 = 1.0e-6;

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

    /// This camera framed on the UV box `min..max` with a `margin` (a fraction of
    /// the box) around it, at its own aspect ratio.
    pub fn fitted_to(self, min: Vec2, max: Vec2, margin: f32) -> Self {
        let size = (max - min).max(Vec2::splat(1.0e-4)) * (1.0 + margin * 2.0);
        let aspect = self.aspect_ratio.max(0.1);
        let half_height = (size.y * 0.5).max(size.x * 0.5 / aspect);
        Self {
            center: (min + max) * 0.5,
            half_height: half_height.clamp(UV_MIN_HALF_HEIGHT, UV_MAX_HALF_HEIGHT),
            aspect_ratio: self.aspect_ratio,
        }
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

/// Where a framed model has to land: the shape of the image a 3D camera draws,
/// and the part of that image the user can actually see.
///
/// The two differ because the chrome is painted *over* the scene. A single view
/// spans the whole window, and the toolbar, status bar, Outliner and Inspector
/// each cover one edge of it, so a fit to the window puts the model's sides under
/// the side panels. The Opt and Aud splits draw each view into its own half of
/// the visible area instead: nothing covers that, but it is half as wide, so a
/// fit made at the window's shape overflows both sides of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramingArea {
    /// Width over height of the image the camera draws.
    pub aspect_ratio: f32,
    /// The visible part of that image as fractions of its size, top-left
    /// (`0,0`) to bottom-right (`1,1`): y down, as the chrome measures it.
    pub visible_min: Vec2,
    pub visible_max: Vec2,
}

impl FramingArea {
    /// All of an image `aspect_ratio` wide per unit of height.
    pub fn whole(aspect_ratio: f32) -> Self {
        Self {
            aspect_ratio,
            visible_min: Vec2::ZERO,
            visible_max: Vec2::ONE,
        }
    }

    /// The same area shrunk about its own centre to `fraction` of its size.
    pub fn shrunk(self, fraction: f32) -> Self {
        let center = (self.visible_min + self.visible_max) * 0.5;
        let half = (self.visible_max - self.visible_min) * 0.5 * fraction;
        Self {
            visible_min: center - half,
            visible_max: center + half,
            ..self
        }
    }

    /// The visible area, less the framing margin, as the tangents of the view
    /// angles to its edges: `[left, right]` and `[bottom, top]`. A point
    /// `(x, y)` across from the view axis at depth `z` is in it when `x / z`
    /// and `y / z` fall inside those spans, which is the form the fit is solved
    /// in.
    fn tangent_spans(self, fov_y_radians: f32) -> ([f32; 2], [f32; 2]) {
        let tan_up = (fov_y_radians * 0.5).clamp(0.01, 1.5).tan();
        let tan_right = tan_up * self.aspect_ratio.max(0.1);
        let min = self.visible_min.clamp(Vec2::ZERO, Vec2::ONE);
        let max = self.visible_max.clamp(Vec2::ZERO, Vec2::ONE);
        // Fractions (y down) to normalised device coordinates (y up).
        let [left, right] = margined_span(min.x * 2.0 - 1.0, max.x * 2.0 - 1.0);
        let [bottom, top] = margined_span(1.0 - max.y * 2.0, 1.0 - min.y * 2.0);
        (
            [left * tan_right, right * tan_right],
            [bottom * tan_up, top * tan_up],
        )
    }
}

/// The span `lo..hi` pulled in about its centre by [`FRAME_MARGIN`], and held
/// open to [`MIN_FRAMING_HALF_SPAN`] so a degenerate area still frames.
fn margined_span(lo: f32, hi: f32) -> [f32; 2] {
    let center = (lo + hi) * 0.5;
    let half = ((hi - lo) * 0.5 / FRAME_MARGIN).max(MIN_FRAMING_HALF_SPAN);
    [center - half, center + half]
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
            fov_y_radians: DEFAULT_FOV_Y_DEGREES.to_radians(),
            scene_radius: 1.0,
        };
        base.framed_to_bounds(
            GRID_BOUNDS,
            FramingArea::whole(base.aspect_ratio),
            CameraProjection::Perspective,
        )
    }
}

impl OrbitCamera {
    /// Back to the default view, keeping the live aspect ratio and the user's
    /// field of view.
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        let fov_y_radians = self.fov_y_radians;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
        self.set_fov_keeping_extent(fov_y_radians);
    }

    /// Change the vertical field of view, dollying along the view axis so the
    /// plane through the orbit pivot keeps its size on screen: only how strongly
    /// the perspective converges changes, not what is framed.
    ///
    /// Holding `distance * tan(fov / 2)` fixed is what that means, and it is the
    /// product [`Self::view_extent`] and the orthographic half-height are both
    /// built from — so a FOV change moves neither. The orthographic view, the
    /// ambient-occlusion radius derived from the extent, and the size a model
    /// keeps across a projection toggle are all left exactly as they were, which
    /// is what makes the field of view a perspective-only setting rather than a
    /// second zoom.
    ///
    /// Ignores an angle outside `(0, PI)`, where the tangent has no meaning.
    pub fn set_fov_keeping_extent(&mut self, fov_y_radians: f32) {
        if !(fov_y_radians > 0.0 && fov_y_radians < std::f32::consts::PI) {
            return;
        }
        let old_half = (self.fov_y_radians * 0.5).tan();
        let new_half = (fov_y_radians * 0.5).tan();
        self.distance = (self.distance * old_half / new_half).max(MIN_ORBIT_DISTANCE);
        self.fov_y_radians = fov_y_radians;
    }

    /// Frame `bounds` across this camera's whole image, in perspective.
    pub fn frame_bounds(&mut self, bounds: Bounds) {
        *self = self.framed_to_bounds(
            bounds,
            FramingArea::whole(self.aspect_ratio),
            CameraProjection::Perspective,
        );
    }

    /// Frame the camera so `bounds` fills the visible part of `area`, as tight
    /// as it can be and centred in it, seen through `projection`.
    ///
    /// Only the target and distance move. The fit is made at `area`'s aspect
    /// ratio rather than the camera's own: the Opt split draws this camera into a
    /// half-width view and re-aspects it there, and `app` keeps the field at the
    /// window's shape.
    ///
    /// The projection matters because the two see a box differently: perspective
    /// binds on the corners nearest the eye, which look biggest, and orthographic
    /// sees every corner at the size it has at the pivot. A perspective fit seen
    /// orthographically can overflow the area by a third of its size (measured
    /// over the cases the tests below frame), which is what `F` used to do
    /// there. Each is fit exactly for itself, so a projection toggle, which keeps
    /// the distance, can leave the model loose or over an edge until the next `F`.
    /// Holding both fits at once was tried and costs too much: up to a quarter of
    /// the fill, and orthographic off-centre by a fifth.
    ///
    /// An orthographic view's size is tied to the distance (its half-height is
    /// `distance · tan(fov/2)`), but its eye is still a point on the axis with a
    /// near plane in front of it, so the distance is also held where orbiting
    /// cannot slice the model ([`orthographic_clearance`]). Looking down a tall
    /// box, that, not the area, is what binds — a cost of the coupling, which
    /// the perspective fit this replaced paid too.
    pub fn framed_to_bounds(
        mut self,
        bounds: Bounds,
        area: FramingArea,
        projection: CameraProjection,
    ) -> Self {
        let center = bounds.center();
        let half_size = bounds.size() * 0.5;
        let rotation = self.rotation();
        // Camera basis. `forward` points from the eye toward (and past) the
        // target, so a corner's depth from the eye is `distance + forward·c`.
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let forward = self.forward_dir();
        let (span_x, span_y) = area.tangent_spans(self.fov_y_radians);

        // Each corner's offset from the box centre along the camera basis, as
        // `(across, depth)` per screen axis: across is `right·c` or `up·c`, and
        // depth is `forward·c` as perspective sees it, or 0 as orthographic does.
        // The box contains the model, so a fit that holds its corners holds
        // every vertex.
        let seen_depth = |depth: f32| match projection {
            CameraProjection::Perspective => depth,
            CameraProjection::Orthographic => 0.0,
        };
        let mut x = [(0.0_f32, 0.0_f32); 8];
        let mut y = x;
        let mut i = 0;
        for cx in [-1.0_f32, 1.0] {
            for cy in [-1.0_f32, 1.0] {
                for cz in [-1.0_f32, 1.0] {
                    let c = Vec3::new(cx * half_size.x, cy * half_size.y, cz * half_size.z);
                    let depth = seen_depth(forward.dot(c));
                    x[i] = (right.dot(c), depth);
                    y[i] = (up.dot(c), depth);
                    i += 1;
                }
            }
        }
        // Perspective's fit already keeps every corner in front of the eye: a
        // corner at or behind it cannot be inside the frustum.
        let clearance = match projection {
            CameraProjection::Perspective => 0.0,
            CameraProjection::Orthographic => {
                orthographic_clearance(half_size.length(), span_x, span_y)
            }
        };

        let distance = fit_distance(&x, span_x)
            .max(fit_distance(&y, span_y))
            .max(clearance)
            .max(MIN_ORBIT_DISTANCE);
        // The axis that set the distance is already pinned between its two
        // edges; the other has room, and both are centred in what they have.
        let pan_x = centring_pan(&x, span_x, distance);
        let pan_y = centring_pan(&y, span_y, distance);

        // Pan the target laterally by the solved offset (the eye follows, so
        // depth along forward is unchanged).
        self.target = center - right * pan_x - up * pan_y;
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
        self.distance = (self.distance * scale).max(MIN_ORBIT_DISTANCE);
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
    ///
    /// That ratio fit alone, though, is a zoom limit in disguise: it is scaled to
    /// the *scene*, not to how close the camera has come, so on a large model it
    /// parks the near plane metres out and clips away the very surface the user
    /// zoomed in to read. So the near plane also tracks the pivot
    /// (`NEAR_PIVOT_FRACTION`), taking whichever of the two is nearer — the
    /// ratio governs the ordinary case, the pivot term takes over on a close
    /// approach and keeps what's being inspected inside the frustum.
    pub fn near_far(self) -> (f32, f32) {
        // Far must clear the grid even when the model is tiny.
        let content_radius = self.scene_radius.max(GRID_FAR_RADIUS);
        let z_far = (self.distance + content_radius).max(MIN_Z_NEAR * 2.0);
        let z_near = (z_far / MAX_DEPTH_RATIO)
            .min(self.distance * NEAR_PIVOT_FRACTION)
            .max(MIN_Z_NEAR);
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
        (self.distance * (self.fov_y_radians * 0.5).tan()).max(MIN_ORTHO_HALF_HEIGHT)
    }

    /// The world ray under a point of the viewport, as `(origin, direction)`
    /// with a unit direction — so a hit parameter `t` along it reads as a world
    /// distance, directly comparable with [`Self::near_far`].
    ///
    /// `ndc` is normalised device coordinates: `-1..1` across the viewport with
    /// **y up**, which is what [`Self::project`] hands back and the inverse of
    /// what a pointer position in pixels gives.
    ///
    /// Built from the camera basis rather than by inverting the view-projection
    /// matrix, because both projections here are unusual enough that the inverse
    /// is the harder thing to get right: perspective is reversed-Z with an
    /// *infinite* far plane, and orthographic passes its near and far swapped.
    /// The basis says the same thing without a depth convention to trip over.
    pub fn screen_ray(self, ndc: Vec2, projection_mode: CameraProjection) -> (Vec3, Vec3) {
        let rotation = self.rotation();
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let forward = self.forward_dir();
        let aspect = self.aspect_ratio.max(0.1);
        match projection_mode {
            CameraProjection::Perspective => {
                // Every ray leaves the eye; the pixel picks the direction.
                let tan_up = (self.fov_y_radians * 0.5).tan();
                let tan_right = tan_up * aspect;
                let direction =
                    (forward + right * (ndc.x * tan_right) + up * (ndc.y * tan_up)).normalize();
                (self.eye_position(), direction)
            }
            CameraProjection::Orthographic => {
                // Every ray runs along the view axis; the pixel picks the origin.
                let half_height = self.orthographic_half_height();
                let half_width = half_height * aspect;
                let origin =
                    self.eye_position() + right * (ndc.x * half_width) + up * (ndc.y * half_height);
                (origin, forward)
            }
        }
    }

    /// Where `world` lands in the viewport, in pixels from its top-left corner
    /// (y **down**, as a pointer position is), or `None` when it is behind the
    /// camera.
    ///
    /// The inverse of [`Self::screen_ray`], and the mapping the bounding-box
    /// dimension labels already place themselves with — shared here so the bone
    /// pick, which needs a screen-space distance to a drawn edge, measures
    /// against exactly the pixels the edge was drawn at.
    pub fn project(
        self,
        world: Vec3,
        projection_mode: CameraProjection,
        viewport_size: Vec2,
    ) -> Option<Vec2> {
        let clip = self.view_projection(projection_mode) * world.extend(1.0);
        // Reversed-Z leaves `w` as the view depth in perspective and 1 in
        // orthographic, so this is the "behind the camera" test in both.
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * viewport_size.x,
            (0.5 - ndc.y * 0.5) * viewport_size.y,
        ))
    }

    /// The `ndc` for a pointer position in pixels from the viewport's top-left
    /// corner — [`Self::project`]'s mapping run backwards, and what feeds
    /// [`Self::screen_ray`].
    pub fn ndc_from_viewport(position: Vec2, viewport_size: Vec2) -> Vec2 {
        let size = viewport_size.max(Vec2::splat(1.0));
        Vec2::new(
            (position.x / size.x) * 2.0 - 1.0,
            1.0 - (position.y / size.y) * 2.0,
        )
    }

    /// The world-space height of the viewport at the orbit target's depth — how
    /// much of the world the user is actually looking at, in scene units.
    ///
    /// This is the scale-free anchor for anything that has to look the same on a
    /// 10 cm prop and a 10 km landscape. A model's *bounding sphere* cannot play
    /// that role: it says how big the thing is, not how closely it is being
    /// examined, so a screen-space effect keyed to it changes meaning entirely
    /// between a prop and an interior.
    ///
    /// One expression covers both projections, because
    /// `Self::orthographic_half_height` is defined as the same
    /// `distance * tan(fov/2)`: a perspective and an orthographic camera framing
    /// the same content report the same extent, so nothing downstream needs a
    /// projection branch.
    pub fn view_extent(self) -> f32 {
        (2.0 * self.distance * (self.fov_y_radians * 0.5).tan()).max(1e-6)
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

/// The smallest distance at which every point fits `span` for *some* pan.
///
/// A point `(across, depth)` is in view when `lo·(d + depth) <= across + pan <=
/// hi·(d + depth)`. Rearranged, `pan - hi·d <= hi·depth - across` and `lo·d -
/// pan <= across - lo·depth` must hold for every point; adding the tightest of
/// each eliminates the pan and leaves `d >= -(a + b) / (hi - lo)`. At that
/// distance the silhouette touches both edges, so the fit is exact in one step.
/// (It used to be iterated — fit a distance, re-centre, repeat six times — and
/// the last re-centre moved the pan after the last fit, so the pair it returned
/// had never been checked against each other.)
fn fit_distance(points: &[(f32, f32)], [lo, hi]: [f32; 2]) -> f32 {
    let a = points
        .iter()
        .map(|&(across, depth)| hi * depth - across)
        .fold(f32::INFINITY, f32::min);
    let b = points
        .iter()
        .map(|&(across, depth)| across - lo * depth)
        .fold(f32::INFINITY, f32::min);
    -(a + b) / (hi - lo)
}

/// The least distance an orthographic framing may sit at so that orbiting
/// from it never puts the model through the near plane.
///
/// An orbit swings the eye round the pivot at `distance`, so a point `r` from
/// the pivot comes as close as `distance - r` to it. The pivot is not the box's
/// centre: centring the box in an off-centre area pans it there by the area's
/// centre times the distance (`offset`, in the spans' tangent units), so every
/// corner is within `radius + offset · distance` of it. The near plane is never
/// further out than `far / MAX_DEPTH_RATIO`, with `far = distance + max(radius,
/// GRID_FAR_RADIUS)` (`near_far`), and solving `distance - radius - offset ·
/// distance >= far / ratio` for the distance gives this. Clearing only the
/// corners in front of the eye framed tighter and sliced the model on the first
/// orbit; clearing the box's centre rather than the pivot did the same in an
/// off-centre area.
fn orthographic_clearance(radius: f32, [left, right]: [f32; 2], [bottom, top]: [f32; 2]) -> f32 {
    let offset = Vec2::new(left + right, bottom + top).length() * 0.5;
    let ratio = 1.0 / MAX_DEPTH_RATIO;
    let reach = radius.max(GRID_FAR_RADIUS);
    (radius + reach * ratio) / (1.0 - offset - ratio).max(MIN_CLEARANCE_SHARE)
}

/// The pan that centres the silhouette of `points` in `span` at `distance`:
/// the one leaving its two outermost corners equally far in from the edges.
///
/// It lies in the range of pans that keep every point in view, which
/// `distance` guarantees is not empty. Every corner's screen position grows
/// with the pan, so the silhouette's centre does too, and a bisection over that
/// range finds it. Putting the *box centre* on the area's centre instead would
/// leave perspective's near corners, which look bigger, against one edge and a
/// gap at the other.
fn centring_pan(points: &[(f32, f32)], [lo, hi]: [f32; 2], distance: f32) -> f32 {
    let mut low = f32::NEG_INFINITY;
    let mut high = f32::INFINITY;
    for &(across, depth) in points {
        low = low.max(lo * (distance + depth) - across);
        high = high.min(hi * (distance + depth) - across);
    }
    if low >= high {
        // Pinned at both edges (the binding axis) or crossed by rounding.
        return 0.5 * (low + high);
    }
    // How far the silhouette's centre sits right of the span's, doubled. At
    // `low` a corner touches the near edge and at `high` the far one, so this
    // is negative at one end of the range and positive at the other.
    let off_centre = |pan: f32| {
        let (mut first, mut last) = (f32::INFINITY, f32::NEG_INFINITY);
        for &(across, depth) in points {
            let screen = (across + pan) / (distance + depth).max(f32::EPSILON);
            first = first.min(screen);
            last = last.max(screen);
        }
        first + last - (lo + hi)
    };
    for _ in 0..CENTRING_STEPS {
        let mid = 0.5 * (low + high);
        if off_centre(mid) < 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
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

    /// One mouse-wheel notch's worth of zoom, matching `app`'s
    /// `WHEEL_LINE_ZOOM_STEP`, so the zoom-range tests exercise the real step.
    const WHEEL_NOTCH: f32 = 0.5;

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

    /// A FOV change is a dolly, not a zoom: the pivot plane keeps its size, so
    /// the orthographic view built from the same product does not move at all.
    #[test]
    fn a_fov_change_keeps_the_pivot_plane_and_the_orthographic_view() {
        let before = OrbitCamera {
            distance: 4.0,
            ..OrbitCamera::default()
        };
        for degrees in [10.0_f32, 35.0, 90.0, 120.0] {
            let mut after = before;
            after.set_fov_keeping_extent(degrees.to_radians());

            assert_eq!(after.fov_y_radians, degrees.to_radians());
            assert_eq!(after.target, before.target);
            let extent = before.view_extent();
            assert!(
                (after.view_extent() - extent).abs() < 1e-5 * extent,
                "{degrees} deg: extent {} became {}",
                extent,
                after.view_extent(),
            );
            // The orthographic x/y scale is the whole of what it shows; its z
            // terms follow the near/far fit, which moves with the eye.
            let ortho_before = before.projection_matrix(CameraProjection::Orthographic);
            let ortho_after = after.projection_matrix(CameraProjection::Orthographic);
            assert!((ortho_after.x_axis.x - ortho_before.x_axis.x).abs() < 1e-5);
            assert!((ortho_after.y_axis.y - ortho_before.y_axis.y).abs() < 1e-5);
        }

        // A wider lens has to come closer to frame the same plane.
        let mut wide = before;
        wide.set_fov_keeping_extent(90.0_f32.to_radians());
        assert!(wide.distance < before.distance);
    }

    #[test]
    fn a_degenerate_fov_is_ignored() {
        let before = OrbitCamera::default();
        for bad in [0.0, -1.0, std::f32::consts::PI, f32::NAN] {
            let mut after = before;
            after.set_fov_keeping_extent(bad);
            assert_eq!(after.fov_y_radians, before.fov_y_radians);
            assert_eq!(after.distance, before.distance);
        }
    }

    #[test]
    fn zoom_in_is_not_capped_short_of_close_inspection() {
        let mut camera = OrbitCamera::default();
        for _ in 0..400 {
            camera.zoom(WHEEL_NOTCH);
        }

        // A tenth of a millimetre from the pivot: whatever the user is reading,
        // the wheel got them to it.
        assert!(camera.distance <= 2.0e-4, "distance {}", camera.distance);
        assert!(camera.distance > 0.0);
    }

    #[test]
    fn near_plane_follows_the_pivot_on_a_close_approach() {
        // A large scene: the depth-ratio fit alone would park the near plane
        // 10 cm out and clip away everything the zoom was aimed at.
        let mut camera = OrbitCamera::default().framed_to_bounds(
            Bounds {
                min: Vec3::splat(-500.0),
                max: Vec3::splat(500.0),
            },
            FramingArea::whole(16.0 / 9.0),
            CameraProjection::Perspective,
        );
        while camera.distance > 0.01 {
            camera.zoom(WHEEL_NOTCH);
        }

        let (near, _) = camera.near_far();
        assert!(
            near < camera.distance,
            "near {near} pivot {}",
            camera.distance
        );
    }

    /// The areas the viewer asks framing to fill, on a 1600x900 window: all of
    /// it, the band between the toolbar and status bar, that band less both side
    /// panels or the Outliner alone, and one half of the Opt split.
    fn framing_areas() -> Vec<(&'static str, FramingArea)> {
        let (width, height) = (1600.0, 900.0);
        let band = |left: f32, right: f32| FramingArea {
            aspect_ratio: width / height,
            visible_min: Vec2::new(left / width, 48.0 / height),
            visible_max: Vec2::new(1.0 - right / width, 1.0 - 42.0 / height),
        };
        vec![
            ("whole", FramingArea::whole(width / height)),
            ("bars", band(0.0, 0.0)),
            ("panels", band(300.0, 300.0)),
            ("outliner", band(300.0, 0.0)),
            ("split half", FramingArea::whole(500.0 / 810.0)),
        ]
    }

    /// The shapes bounds come in: a wide arch, a tall column, a flat panel and a
    /// cube, away from the origin so the target has somewhere to go.
    fn framing_boxes() -> Vec<Bounds> {
        let center = Vec3::new(2.0, 1.5, -3.0);
        [
            Vec3::new(4.0, 3.0, 1.0),
            Vec3::new(0.6, 5.0, 0.6),
            Vec3::new(3.0, 0.05, 2.0),
            Vec3::splat(1.0),
        ]
        .into_iter()
        .map(|size| Bounds {
            min: center - size * 0.5,
            max: center + size * 0.5,
        })
        .collect()
    }

    /// Every framing the tests below check, labelled for their messages: each
    /// area and box from a spread of directions and lenses, fit for
    /// `projection`.
    fn framings(projection: CameraProjection) -> Vec<(String, FramingArea, Bounds, OrbitCamera)> {
        let mut cases = Vec::new();
        for (name, area) in framing_areas() {
            for (index, bounds) in framing_boxes().into_iter().enumerate() {
                for yaw in (0..360).step_by(30) {
                    for pitch in [-80.0_f32, -35.0, 0.0, 30.0, 80.0] {
                        for fov in [30.0_f32, 50.0, 90.0] {
                            // The camera keeps the window's shape, as `app`'s does;
                            // the fit must go by the area's.
                            let start = OrbitCamera {
                                yaw: (yaw as f32).to_radians(),
                                pitch: pitch.to_radians(),
                                fov_y_radians: fov.to_radians(),
                                aspect_ratio: 16.0 / 9.0,
                                ..OrbitCamera::default()
                            };
                            cases.push((
                                format!("{name}, box {index}, yaw {yaw} pitch {pitch} fov {fov}"),
                                area,
                                bounds,
                                start.framed_to_bounds(bounds, area, projection),
                            ));
                        }
                    }
                }
            }
        }
        cases
    }

    /// Where each corner of `bounds` lands in an image of `area`'s shape, as
    /// fractions of it (y down), through `camera` in `projection`. Every corner
    /// must also be past the near plane, in either projection, or it is not
    /// drawn wherever it lands.
    fn landed(
        camera: OrbitCamera,
        area: FramingArea,
        bounds: Bounds,
        projection: CameraProjection,
    ) -> Vec<Vec2> {
        let camera = OrbitCamera {
            aspect_ratio: area.aspect_ratio,
            ..camera
        };
        let (near, _) = camera.near_far();
        let mut points = Vec::new();
        for x in [bounds.min.x, bounds.max.x] {
            for y in [bounds.min.y, bounds.max.y] {
                for z in [bounds.min.z, bounds.max.z] {
                    let corner = Vec3::new(x, y, z);
                    let depth = camera.forward_dir().dot(corner - camera.eye_position());
                    assert!(
                        depth > near,
                        "a corner is {depth} from the eye, near {near}"
                    );
                    let point = camera.project(corner, projection, Vec2::ONE);
                    points.push(point.expect("a framed corner is in front of the eye"));
                }
            }
        }
        points
    }

    /// How much of `area`'s visible part, less the margin, `points` span along
    /// each axis, and how far their centre sits off its centre - both as
    /// fractions of that span.
    fn fit_of(points: &[Vec2], area: FramingArea) -> (Vec2, Vec2) {
        let first = points.iter().copied().reduce(Vec2::min).unwrap();
        let last = points.iter().copied().reduce(Vec2::max).unwrap();
        let center = (area.visible_min + area.visible_max) * 0.5;
        let span = (area.visible_max - area.visible_min) / FRAME_MARGIN;
        (
            (last - first) / span,
            ((first + last) * 0.5 - center) / span,
        )
    }

    /// What `F` promises: every corner of the box lands inside the part of the
    /// image no chrome covers. Fitting to the whole window while the side panels
    /// covered its edges, or at the window's shape while the split drew a
    /// half-width view, put an arch's sides out of sight.
    #[test]
    fn a_framed_box_lands_inside_the_visible_area() {
        for projection in [
            CameraProjection::Perspective,
            CameraProjection::Orthographic,
        ] {
            for (case, area, bounds, camera) in framings(projection) {
                for point in landed(camera, area, bounds, projection) {
                    let inside = point.cmpge(area.visible_min - 1e-4).all()
                        && point.cmple(area.visible_max + 1e-4).all();
                    assert!(
                        inside,
                        "{case}, {projection:?}: a corner lands at {point:?}, outside \
                         {:?}..{:?}",
                        area.visible_min, area.visible_max
                    );
                }
            }
        }
    }

    /// And it is a *fit*, not merely inside: the silhouette spans the visible
    /// area, less the margin, along one axis and is centred in it along both.
    /// The one exception is an orthographic view held back to keep its eye out of
    /// the box (see `framed_to_bounds`), which still centres what it shows.
    #[test]
    fn a_framed_box_fills_and_centres_in_the_visible_area() {
        for projection in [
            CameraProjection::Perspective,
            CameraProjection::Orthographic,
        ] {
            for (case, area, bounds, camera) in framings(projection) {
                let points = landed(camera, area, bounds, projection);
                let (fill, off_centre) = fit_of(&points, area);
                let held_back =
                    projection == CameraProjection::Orthographic && held_back(camera, area, bounds);
                assert!(
                    fill.max_element() > 0.999 || held_back,
                    "{case}, {projection:?}: the silhouette spans only {fill:?} of the area"
                );
                assert!(
                    off_centre.abs().max_element() < 1e-3,
                    "{case}, {projection:?}: the silhouette sits {off_centre:?} off the \
                     area's centre"
                );
            }
        }
    }

    /// Whether an orthographic `camera` sits where its fit had to hold it back
    /// to clear the model for orbiting, rather than where `area` would allow.
    fn held_back(camera: OrbitCamera, area: FramingArea, bounds: Bounds) -> bool {
        let (span_x, span_y) = area.tangent_spans(camera.fov_y_radians);
        let clearance = orthographic_clearance(bounds.size().length() * 0.5, span_x, span_y);
        camera.distance <= clearance * 1.0001
    }

    /// An orthographic framing is a place to orbit from, not only a picture: its
    /// eye sits close in (the view's size is tied to the distance), so a fit that
    /// cleared only the corners in front of it put the far ones through the near
    /// plane as soon as the camera turned.
    #[test]
    fn an_orthographic_framing_survives_an_orbit() {
        for (_, area, bounds, framed) in framings(CameraProjection::Orthographic) {
            for yaw in (0..360).step_by(45) {
                for pitch in [-1.5_f32, -0.6, 0.0, 0.6, 1.5] {
                    let camera = OrbitCamera {
                        yaw: (yaw as f32).to_radians(),
                        pitch,
                        ..framed
                    };
                    // `landed` asserts every corner is past the near plane.
                    landed(camera, area, bounds, CameraProjection::Orthographic);
                }
            }
        }
    }

    #[test]
    fn near_plane_still_uses_the_depth_ratio_at_normal_range() {
        let camera = OrbitCamera::default();
        let (near, far) = camera.near_far();

        assert!((near - far / MAX_DEPTH_RATIO).abs() < 1e-6, "near {near}");
    }

    #[test]
    fn orthographic_projection_uses_reversed_z() {
        let camera = OrbitCamera::default();
        let (near, far) = camera.near_far();
        let projection = camera.projection_matrix(CameraProjection::Orthographic);

        assert!((ndc_z(projection, -near) - 1.0).abs() < 1e-5);
        assert!(ndc_z(projection, -far).abs() < 1e-5);
    }

    /// A camera at each of a spread of orientations and framings, so a test
    /// failure names the case rather than one lucky angle passing for all.
    fn sample_cameras() -> Vec<OrbitCamera> {
        let mut cameras = Vec::new();
        for (yaw, pitch) in [(0.0, 0.0), (0.7, -0.4), (-2.1, 0.9), (3.0, 0.2)] {
            for (distance, aspect) in [(3.0, 16.0 / 9.0), (40.0, 1.0), (0.8, 0.5)] {
                cameras.push(OrbitCamera {
                    target: Vec3::new(0.3, 1.2, -0.7),
                    yaw,
                    pitch,
                    distance,
                    aspect_ratio: aspect,
                    scene_radius: distance,
                    ..OrbitCamera::default()
                });
            }
        }
        cameras
    }

    /// The pick ray under a pixel must pass through every world point that
    /// projects to that pixel — the property the whole viewport pick rests on.
    #[test]
    fn screen_ray_round_trips_project() {
        let viewport = Vec2::new(1280.0, 720.0);
        for camera in sample_cameras() {
            for projection in [
                CameraProjection::Perspective,
                CameraProjection::Orthographic,
            ] {
                // Points spread through the volume the camera is looking at.
                for ix in -2..=2 {
                    for iy in -2..=2 {
                        for iz in -1..=1 {
                            let spread = camera.view_extent() * 0.3;
                            let world = camera.target
                                + camera.rotation().transform_vector3(Vec3::new(
                                    ix as f32 * spread,
                                    iy as f32 * spread,
                                    iz as f32 * spread,
                                ));
                            let Some(pixel) = camera.project(world, projection, viewport) else {
                                continue;
                            };
                            let ndc = OrbitCamera::ndc_from_viewport(pixel, viewport);
                            let (origin, dir) = camera.screen_ray(ndc, projection);
                            let offset = world - origin;
                            // The point lies on the ray: its offset from the
                            // origin is parallel to the direction.
                            let perpendicular = offset.cross(dir).length();
                            assert!(
                                perpendicular < 1e-3 * offset.length().max(1.0),
                                "{projection:?} yaw {} pitch {} distance {}: {world:?} is \
                                 {perpendicular} off the ray under its own pixel {pixel:?}",
                                camera.yaw,
                                camera.pitch,
                                camera.distance,
                            );
                            // And in front of it, which is what makes `t` a
                            // usable distance.
                            assert!(
                                offset.dot(dir) > 0.0,
                                "{projection:?}: point behind the ray"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A unit direction is what lets a hit parameter read as a world distance
    /// and be compared against the near/far planes.
    #[test]
    fn screen_rays_have_unit_directions() {
        for camera in sample_cameras() {
            for projection in [
                CameraProjection::Perspective,
                CameraProjection::Orthographic,
            ] {
                for ndc in [
                    Vec2::ZERO,
                    Vec2::new(-1.0, -1.0),
                    Vec2::new(1.0, 1.0),
                    Vec2::new(0.37, -0.82),
                ] {
                    let (_, dir) = camera.screen_ray(ndc, projection);
                    assert!((dir.length() - 1.0).abs() < 1e-5, "{projection:?} {ndc:?}");
                }
            }
        }
    }

    /// Orthographic rays are parallel and perspective ones are not — the whole
    /// reason the two projections need separate arms.
    #[test]
    fn ortho_rays_are_parallel_and_perspective_rays_share_an_origin() {
        let camera = OrbitCamera {
            distance: 5.0,
            aspect_ratio: 1.5,
            ..OrbitCamera::default()
        };
        let corners = [Vec2::new(-1.0, -1.0), Vec2::new(1.0, 1.0)];

        let ortho: Vec<_> = corners
            .iter()
            .map(|&ndc| camera.screen_ray(ndc, CameraProjection::Orthographic))
            .collect();
        assert!(
            ortho[0].1.abs_diff_eq(ortho[1].1, 1e-6),
            "directions differ"
        );
        assert!(
            ortho[0].0.distance(ortho[1].0) > 1.0,
            "origins should span the viewport"
        );

        let perspective: Vec<_> = corners
            .iter()
            .map(|&ndc| camera.screen_ray(ndc, CameraProjection::Perspective))
            .collect();
        assert!(perspective[0].0.abs_diff_eq(perspective[1].0, 1e-6));
        assert!(!perspective[0].1.abs_diff_eq(perspective[1].1, 1e-3));
    }

    /// The centre pixel looks straight down the view axis in both projections.
    #[test]
    fn the_centre_pixel_looks_along_the_view_axis() {
        for camera in sample_cameras() {
            for projection in [
                CameraProjection::Perspective,
                CameraProjection::Orthographic,
            ] {
                let (origin, dir) = camera.screen_ray(Vec2::ZERO, projection);
                assert!(dir.abs_diff_eq(camera.forward_dir(), 1e-5));
                assert!(origin.abs_diff_eq(camera.eye_position(), 1e-4));
            }
        }
    }

    /// The Aud split frames a UV box with a margin at the camera's own aspect:
    /// centred on it, and tall or wide enough for whichever side binds.
    #[test]
    fn a_uv_camera_fits_a_box_at_its_aspect() {
        let camera = UvCamera {
            aspect_ratio: 2.0,
            ..UvCamera::default()
        };
        // A tall box binds on height.
        let tall = camera.fitted_to(Vec2::new(0.2, 0.0), Vec2::new(0.4, 1.0), 0.1);
        assert!((tall.center - Vec2::new(0.3, 0.5)).length() < 1e-6);
        assert!((tall.half_height - 0.6).abs() < 1e-6);
        // A wide box binds on width: half of 2.4 across, over an aspect of 2.
        let wide = camera.fitted_to(Vec2::ZERO, Vec2::new(2.0, 0.1), 0.1);
        assert!((wide.half_height - 0.6).abs() < 1e-6);
        assert_eq!(wide.aspect_ratio, 2.0);
    }
}
