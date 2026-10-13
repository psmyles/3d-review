//! The camera half of the [`Renderer`] façade.
//!
//! Three cameras: the 3D orbit camera, the UV viewport's 2D camera, and the Opt
//! split's right-hand camera. `app` drives all of them from input; the math
//! itself lives in [`crate::camera`].

use glam::{Vec2, Vec3};
use review_model::Bounds;

use crate::camera::{CameraTransition, FramingArea, OrbitCamera};
use crate::{CameraProjection, GRID_BOUNDS, HOME_FILL_FRACTION, Renderer};

/// Which of the two orbit cameras a gesture drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraTarget {
    /// The 3D viewport's camera.
    Scene,
    /// The Opt split view's right-hand camera.
    Opt,
}

impl Renderer {
    /// Which of the two orbit cameras a gesture drives.
    ///
    /// The 3D camera cancels any running transition when the user grabs it; the
    /// Opt split's right-hand camera has no transition of its own, because every
    /// animated move (framing, home, the gizmo) drives the main camera and the
    /// second view follows it. That difference is the *only* one between the two
    /// families of gesture method, which is why they share one selector.
    fn camera_mut(&mut self, target: CameraTarget) -> &mut OrbitCamera {
        match target {
            CameraTarget::Scene => {
                self.camera_transition = None;
                &mut self.camera
            }
            CameraTarget::Opt => &mut self.opt_camera,
        }
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

    /// Set where subsequent framing puts the model: the shape of the image the
    /// main camera is drawn into, and the part of it the chrome leaves visible,
    /// seen through `projection`. `app` calls this every frame, after laying the
    /// chrome out, because a side panel can be resized or closed at any time.
    pub fn set_framing_area(&mut self, area: FramingArea, projection: CameraProjection) {
        self.framing_area = area;
        self.framing_projection = projection;
    }

    /// `camera` framed on `bounds` into the live framing area.
    fn framed(&self, camera: OrbitCamera, bounds: Bounds) -> OrbitCamera {
        camera.framed_to_bounds(bounds, self.framing_area, self.framing_projection)
    }

    pub fn orbit_camera(&mut self, delta: Vec2) {
        self.camera_mut(CameraTarget::Scene).orbit(delta);
    }

    /// Turn the 3D camera in place (the right-button look drag), as against
    /// [`orbit_camera`], which swings it around the pivot.
    ///
    /// [`orbit_camera`]: Renderer::orbit_camera
    pub fn look_camera(&mut self, delta: Vec2) {
        self.camera_mut(CameraTarget::Scene).look(delta);
    }

    /// Fly the 3D camera for `seconds` along `direction` — a unit vector in camera
    /// axes (x right, y world-up, z forward) — at the framing-scaled base speed
    /// times the caller's `speed_scale`. The WASD/QE half of the flycam; `app`
    /// integrates it per frame while the right button is held.
    pub fn fly_camera(&mut self, direction: Vec3, seconds: f32, speed_scale: f32) {
        let camera = self.camera_mut(CameraTarget::Scene);
        let distance = camera.fly_speed() * speed_scale * seconds;
        camera.fly(direction * distance);
    }

    pub fn set_camera_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.camera.aspect_ratio = aspect_ratio;
        self.opt_camera.aspect_ratio = aspect_ratio;
        if let Some(transition) = self.camera_transition.as_mut() {
            transition.start.aspect_ratio = aspect_ratio;
            transition.end.aspect_ratio = aspect_ratio;
        }
    }

    /// Set the vertical field of view of every 3D camera: the main one, the Opt
    /// split's right-hand one, and both ends of a transition in flight, so the
    /// glide cannot carry the old angle back in. Each one dollies to keep its
    /// framing ([`OrbitCamera::set_fov_keeping_extent`]).
    ///
    /// Setting the angle a camera already has leaves it bit for bit as it was
    /// (the dolly ratio is exactly 1), so `app` can hand the chrome's value over
    /// every frame without disturbing anything keyed on the camera.
    pub fn set_camera_fov(&mut self, fov_y_radians: f32) {
        self.camera.set_fov_keeping_extent(fov_y_radians);
        self.opt_camera.set_fov_keeping_extent(fov_y_radians);
        if let Some(transition) = self.camera_transition.as_mut() {
            transition.start.set_fov_keeping_extent(fov_y_radians);
            transition.end.set_fov_keeping_extent(fov_y_radians);
        }
    }

    /// Orbit / pan / zoom the Opt split view's right-hand camera. Used only while
    /// camera sync is off; with it on, `app` drives the main camera and both views
    /// follow it.
    pub fn orbit_opt_camera(&mut self, delta: Vec2) {
        self.camera_mut(CameraTarget::Opt).orbit(delta);
    }

    pub fn look_opt_camera(&mut self, delta: Vec2) {
        self.camera_mut(CameraTarget::Opt).look(delta);
    }

    pub fn fly_opt_camera(&mut self, direction: Vec3, seconds: f32, speed_scale: f32) {
        let camera = self.camera_mut(CameraTarget::Opt);
        let distance = camera.fly_speed() * speed_scale * seconds;
        camera.fly(direction * distance);
    }

    pub fn pan_opt_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.camera_mut(CameraTarget::Opt)
            .pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_opt_camera(&mut self, amount: f32) {
        self.camera_mut(CameraTarget::Opt).zoom(amount);
    }

    /// Point the Opt split view's right-hand camera wherever the main one is
    /// looking — what "sync views" does, and what re-enabling it snaps back to.
    pub fn sync_opt_camera(&mut self) {
        self.opt_camera = self.camera;
    }

    pub fn pan_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.camera_mut(CameraTarget::Scene)
            .pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_camera(&mut self, amount: f32) {
        self.camera_mut(CameraTarget::Scene).zoom(amount);
    }

    pub fn animate_camera_to(&mut self, end: OrbitCamera) {
        self.camera_transition = Some(CameraTransition::new(self.camera, end));
    }

    pub fn animate_camera_to_bounds(&mut self, bounds: Bounds) {
        self.animate_camera_to(self.framed(self.camera, bounds));
    }

    /// Snap (no animation) to a framing of `bounds`. Used when a model is
    /// loaded into the empty viewport, where a fly-in from the home view would
    /// only delay showing the model already framed.
    pub fn snap_camera_to_bounds(&mut self, bounds: Bounds) {
        self.camera_transition = None;
        self.camera = self.framed(self.camera, bounds);
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
    }

    /// The default "home" view, re-framed into the live framing area so the
    /// whole grid stays visible whatever the window's shape and whichever panels
    /// are open. Shared by the animated reset and the instant startup frame so
    /// both land on exactly the same view. Framed through the user's field of
    /// view, not the default one, so a wide lens still fits the grid.
    fn home_camera(&self) -> OrbitCamera {
        let home = OrbitCamera {
            aspect_ratio: self.camera.aspect_ratio,
            fov_y_radians: self.camera.fov_y_radians,
            ..OrbitCamera::default()
        };
        // Fill only a fraction of the area so the grid sits back from the
        // edges (see HOME_FILL_FRACTION) instead of filling it.
        home.framed_to_bounds(
            GRID_BOUNDS,
            self.framing_area.shrunk(HOME_FILL_FRACTION),
            self.framing_projection,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RendererConfig;

    /// One field of view for every 3D view: both cameras, a glide in flight, and
    /// the home view a reset lands on — which is framed through it, not through
    /// the default.
    #[test]
    fn the_field_of_view_reaches_every_camera_and_the_home_view() {
        let fov = 80.0_f32.to_radians();
        let mut renderer = Renderer::new(RendererConfig::default());
        renderer.animate_camera_to_bounds(GRID_BOUNDS);
        renderer.set_camera_fov(fov);

        assert_eq!(renderer.camera.fov_y_radians, fov);
        assert_eq!(renderer.opt_camera.fov_y_radians, fov);
        let transition = renderer.camera_transition.expect("a glide is in flight");
        assert_eq!(transition.start.fov_y_radians, fov);
        assert_eq!(transition.end.fov_y_radians, fov);

        renderer.reset_camera_to_home();
        assert_eq!(renderer.camera.fov_y_radians, fov);
    }

    /// Handing over the angle the cameras already have must change nothing, or
    /// the per-frame call would restart everything keyed on the camera.
    #[test]
    fn setting_the_same_field_of_view_is_a_no_op() {
        let mut renderer = Renderer::new(RendererConfig::default());
        renderer.set_camera_fov(65.0_f32.to_radians());
        let before = (renderer.camera.distance, renderer.opt_camera.distance);
        renderer.set_camera_fov(65.0_f32.to_radians());
        assert_eq!(
            (renderer.camera.distance, renderer.opt_camera.distance),
            before
        );
    }
}
