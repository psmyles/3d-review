//! The right-button flycam: the WASD/QE movement Unity and Unreal bind, and the
//! state that makes a *held* key into continuous motion.
//!
//! Split out beside `input.rs` (the pointer gestures) and `shortcuts.rs` (the
//! key dispatch) because it is neither: the keys only set direction bits, and the
//! camera actually moves once per frame from [`App::step_flycam`], scaled by that
//! frame's elapsed time. Data still flows one way — this reads `App`'s drag / key
//! state and drives the renderer's public camera API (invariant 2).
//!
//! Movement is gated on the right button being held, exactly as in both editors:
//! the same drag that looks around is what arms the movement keys, so bare WASD
//! stays free and a stray key press can never send the camera drifting.

use std::time::Instant;

use glam::Vec3;

use crate::App;
use crate::input::DragMode;

/// Largest slice of time one flight step advances by. The viewer redraws on
/// demand, so the first frame after an idle gap carries the whole gap as its
/// delta — integrating that would teleport the camera the moment a key goes
/// down. One ~30 Hz frame is plenty to keep motion smooth (the same reasoning,
/// and the same cap, as the camera transitions in `update_camera_animation`).
const MAX_FLY_STEP: f32 = 1.0 / 30.0;

/// Multiplier applied to the fly speed per wheel notch, and the range it may
/// reach. One notch is ~15%, so a couple of flicks make a noticeable difference
/// and a full spin crosses the range — the feel both editors have.
const FLY_SPEED_PER_NOTCH: f32 = 1.15;
const MIN_FLY_SPEED_SCALE: f32 = 0.05;
const MAX_FLY_SPEED_SCALE: f32 = 20.0;

/// The flycam's state: which movement keys are down, the user's speed setting,
/// and when the last step was integrated.
pub(crate) struct FlyCam {
    keys: FlyKeys,
    /// Wheel-adjusted multiplier over the framing-scaled base speed
    /// (`OrbitCamera::fly_speed`). Persists across flights — it is a preference,
    /// not drag state — but is deliberately *not* undoable: like the camera
    /// itself it is view state.
    speed_scale: f32,
    /// When [`App::step_flycam`] last moved the camera, or `None` when no flight
    /// is in progress, so the first step of a new one advances by nothing rather
    /// than by however long the viewer sat idle.
    last_step: Option<Instant>,
}

impl Default for FlyCam {
    fn default() -> Self {
        Self {
            keys: FlyKeys::default(),
            speed_scale: 1.0,
            last_step: None,
        }
    }
}

impl FlyCam {
    /// Drop every held direction. Called wherever a drag ends, so a key still
    /// down when the button comes up (or the pointer leaves) can't quietly arm
    /// the next flight.
    pub(crate) fn release_all(&mut self) {
        self.keys = FlyKeys::default();
    }
}

/// The movement keys currently held, as one flag per direction. A set rather
/// than a single direction because the diagonals matter: W+D flies forward-right
/// in one motion, as it does in both editors.
#[derive(Default, Clone, Copy)]
struct FlyKeys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl FlyKeys {
    /// Whether anything is held at all — the cheap test the redraw pacing asks.
    fn any(self) -> bool {
        self.forward || self.back || self.left || self.right || self.up || self.down
    }

    /// The held keys as a unit vector in camera axes (x right, y world-up, z
    /// forward), or `None` when nothing is held or the held keys cancel out
    /// (W+S). Normalized so a diagonal isn't faster than a straight run.
    fn direction(self) -> Option<Vec3> {
        let axis = |positive: bool, negative: bool| f32::from(positive) - f32::from(negative);
        let direction = Vec3::new(
            axis(self.right, self.left),
            axis(self.up, self.down),
            axis(self.forward, self.back),
        );
        (direction.length_squared() > 0.0).then(|| direction.normalize())
    }
}

/// Whether a movement-key event belongs to the flycam, given whether it is a
/// press and whether the look drag that arms flight is live.
///
/// Pure so the four cases can be stated outright: `Q` in particular has to reach
/// the flycam while flying and the Select tool the rest of the time, and getting
/// that backwards silently costs one of the two.
fn fly_key_applies(pressed: bool, look_armed: bool) -> bool {
    !pressed || look_armed
}

/// One of the six flycam directions, as named by the key that drives it.
#[derive(Clone, Copy)]
pub(crate) enum FlyDirection {
    Forward,
    Back,
    Left,
    Right,
    Up,
    Down,
}

impl FlyDirection {
    /// The direction a bare character key flies, or `None` for a key that isn't
    /// one of the six. `key` is already lowercased by the caller.
    ///
    /// W/A/S/D and Q/E are the Unity + Unreal set: WASD moves along the view,
    /// Q/E drop and lift along *world* up.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "w" => Some(Self::Forward),
            "s" => Some(Self::Back),
            "a" => Some(Self::Left),
            "d" => Some(Self::Right),
            "e" => Some(Self::Up),
            "q" => Some(Self::Down),
            _ => None,
        }
    }
}

impl App {
    /// Whether a movement key press belongs to the flycam right now.
    ///
    /// Only while the right-button look drag is live — which is the gesture that
    /// arms flight in the first place, so an unarmed press was inert anyway.
    /// Saying so *before* the key is swallowed is what lets `Q` carry a second
    /// meaning (the Select tool) without taking anything away from flying: with
    /// the right button down it flies, and the rest of the time it toggles.
    ///
    /// A **release** always belongs to the flycam, whatever the buttons are
    /// doing by then: a key still down when the drag ends would otherwise leave
    /// its direction stuck on with nothing left to clear it.
    pub(crate) fn fly_key_applies(&self, pressed: bool) -> bool {
        fly_key_applies(pressed, self.flycam_look_armed())
    }

    /// Whether the right-button look drag that arms flight is live. The proposal
    /// counts as well as the settled drag: a press is only promoted a frame
    /// later (`settle_pending_drag`), and a key pressed inside that frame is
    /// still part of the same gesture.
    fn flycam_look_armed(&self) -> bool {
        matches!(self.drag_mode, Some(DragMode::Look))
            || matches!(self.pending_drag, Some(DragMode::Look))
    }

    /// Set or clear one direction's held flag (`shortcuts.rs` calls this on key
    /// down / up).
    pub(crate) fn set_fly_key(&mut self, direction: FlyDirection, held: bool) {
        let keys = &mut self.flycam.keys;
        let flag = match direction {
            FlyDirection::Forward => &mut keys.forward,
            FlyDirection::Back => &mut keys.back,
            FlyDirection::Left => &mut keys.left,
            FlyDirection::Right => &mut keys.right,
            FlyDirection::Up => &mut keys.up,
            FlyDirection::Down => &mut keys.down,
        };
        *flag = held;
    }

    /// Whether a flight is under way: the right button is down looking around
    /// *and* a movement key is held. The redraw pacing in `frame.rs` keeps
    /// scheduling frames while this holds, which is what turns a held key into
    /// continuous motion (invariant 6: a live interaction, not UI state).
    pub(crate) fn flycam_active(&self) -> bool {
        matches!(self.drag_mode, Some(DragMode::Look)) && self.flycam.keys.any()
    }

    /// Adjust the fly speed by `notches` of wheel, and report whether the wheel
    /// was claimed. Only while the look drag is up — that is where both editors
    /// put it, and it is what leaves the wheel zooming the rest of the time.
    ///
    /// Deliberately not gated on a key being held: setting the speed *before*
    /// moving off is how the gesture is normally used.
    pub(crate) fn adjust_fly_speed(&mut self, notches: f32) -> bool {
        if !matches!(self.drag_mode, Some(DragMode::Look)) {
            return false;
        }
        self.flycam.speed_scale = (self.flycam.speed_scale * FLY_SPEED_PER_NOTCH.powf(notches))
            .clamp(MIN_FLY_SPEED_SCALE, MAX_FLY_SPEED_SCALE);
        true
    }

    /// Move the camera by this frame's share of the held direction. Called once
    /// per frame from `render`, next to the camera-transition step it mirrors.
    pub(crate) fn step_flycam(&mut self) {
        let Some(direction) = self
            .flycam
            .keys
            .direction()
            .filter(|_| self.flycam_active())
        else {
            // Nothing in flight: forget the timestamp so the next flight starts
            // from a zero delta rather than from however long this pause lasted.
            self.flycam.last_step = None;
            return;
        };

        let now = Instant::now();
        let seconds = self
            .flycam
            .last_step
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32())
            .min(MAX_FLY_STEP);
        self.flycam.last_step = Some(now);
        if seconds <= 0.0 {
            return;
        }

        let speed_scale = self.flycam.speed_scale;
        // The flight belongs to whichever Opt split view the look drag started
        // in — the same anchor the drag itself uses, so a flight and the look
        // steering it can never drive different cameras.
        let opt_right = self.drag_in_opt_right_view;
        let synced = self.opt_cameras_synced();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if opt_right {
            renderer.fly_opt_camera(direction, seconds, speed_scale);
        } else {
            renderer.fly_camera(direction, seconds, speed_scale);
        }
        if synced {
            renderer.sync_opt_camera();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Q` means two things, and which one it means is decided here.
    #[test]
    fn a_movement_key_reaches_the_flycam_only_while_the_look_drag_is_live() {
        // Pressed with the right button down: flying.
        assert!(fly_key_applies(true, true));
        // Pressed with no drag: free for whatever else the key is bound to,
        // which for `Q` is the Select tool.
        assert!(!fly_key_applies(true, false));
        // A release always reaches the flycam, whatever the buttons are doing by
        // then — otherwise a key still down when the drag ends leaves its
        // direction stuck on with nothing left to clear it.
        assert!(fly_key_applies(false, true));
        assert!(fly_key_applies(false, false));
    }

    #[test]
    fn opposed_keys_cancel_and_diagonals_stay_unit_length() {
        let mut keys = FlyKeys {
            forward: true,
            back: true,
            ..FlyKeys::default()
        };
        assert!(keys.any());
        assert!(keys.direction().is_none());

        keys = FlyKeys {
            forward: true,
            right: true,
            ..FlyKeys::default()
        };
        let direction = keys.direction().expect("a diagonal is a direction");
        assert!((direction.length() - 1.0).abs() < 1e-6);
        assert_eq!(direction.y, 0.0);
    }
}
