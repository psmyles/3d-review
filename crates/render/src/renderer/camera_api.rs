//! The camera half of the [`Renderer`] façade.
//!
//! Three cameras: the 3D orbit camera, the UV viewport's 2D camera, and the Opt
//! split's right-hand camera. `app` drives all of them from input; the math
//! itself lives in [`crate::camera`].

use glam::{Vec2, Vec3};
use review_model::Bounds;

use crate::camera::{CameraTransition, OrbitCamera};
use crate::{GRID_BOUNDS, HOME_FILL_FRACTION, Renderer};

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

    /// Set the fraction of the viewport that subsequent framing should fill, so
    /// the model lands inside the band left visible by the toolbar / status bar.
    pub fn set_framing_safe_area(&mut self, width_fraction: f32, height_fraction: f32) {
        self.framing_safe_area = Vec2::new(width_fraction, height_fraction);
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
        self.animate_camera_to(self.camera.framed_to_bounds(bounds, self.framing_safe_area));
    }

    /// Snap (no animation) to a framing of `bounds`. Used when a model is
    /// loaded into the empty viewport, where a fly-in from the home view would
    /// only delay showing the model already framed.
    pub fn snap_camera_to_bounds(&mut self, bounds: Bounds) {
        self.camera_transition = None;
        self.camera = self.camera.framed_to_bounds(bounds, self.framing_safe_area);
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
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
