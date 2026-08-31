//! Keyboard-shortcut dispatch: the viewport key bindings `window_event` routes to.
//!
//! Split out of `main.rs` beside `input.rs` (the pointer gestures), with the same
//! shape — one `impl App` block of shallow, named handlers — so the
//! `ApplicationHandler::window_event` match stays a thin dispatcher. Data still
//! flows one way: these mutate `App`'s UI / camera state and request redraws, never
//! the renderer's internals beyond its public camera API (invariant 2).

use review_render::{ShadingMode, selection_bounds};
use review_ui::{Selection, TexViewRequest, WorkspaceMode};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use crate::App;

impl App {
    /// Dispatch a viewport keyboard shortcut on key-down. Ctrl-modified keys are
    /// file commands; the bare keys are view / camera shortcuts and only fire
    /// when no modifier is held (so Shift/Alt/Ctrl combinations stay free).
    /// Keyboard events egui has already consumed are filtered out by the caller.
    ///
    /// Adding/changing a binding here? Update the startup help card's tables in
    /// `crates/ui/src/help.rs` (`LEFT_SHORTCUTS` / `RIGHT_SHORTCUTS` /
    /// `CHORD_SHORTCUTS`) in the same change — they are the user-facing mirror
    /// of this dispatch.
    pub(crate) fn handle_keyboard_shortcut(&mut self, event: &KeyEvent) {
        // Escape clears any Outliner selection (mesh part or material). It's a Named
        // key, so handle it before the Character extraction below.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::Escape))
        {
            if self.ui.selection.is_active() {
                self.ui.selection = Selection::None;
                self.redraw.requested = true;
            }
            return;
        }

        let Key::Character(character) = &event.logical_key else {
            return;
        };

        if self.modifiers.control_key() {
            // File commands fire on key-down only.
            if event.state != ElementState::Pressed {
                return;
            }
            if character.eq_ignore_ascii_case("n") {
                self.reset_to_start_state();
            } else if character.eq_ignore_ascii_case("o") {
                self.open_model_from_dialog();
            } else if character.eq_ignore_ascii_case("z") {
                // Ctrl+Z undoes; Ctrl+Shift+Z redoes (the common alt-redo chord).
                if self.modifiers.shift_key() {
                    self.redo();
                } else {
                    self.undo();
                }
            } else if character.eq_ignore_ascii_case("y") {
                self.redo();
            }
            return;
        }

        if !self.modifiers.is_empty() {
            return;
        }

        // Matched case-insensitively, exactly like the Ctrl chords above: winit's
        // `ModifiersState` carries no Caps Lock bit, so the guard above can't see
        // it — with Caps Lock on the key arrives as "G" and an exact match would
        // leave every one of these shortcuts dead.
        let key = character.to_ascii_lowercase();

        // In the UV workspace the 3D camera shortcuts (WASD orbit / shading /
        // grid) don't apply; only F / R, which reframe the 2D UV view.
        if self.ui.mode == WorkspaceMode::Uv {
            if event.state == ElementState::Pressed && matches!(key.as_str(), "f" | "r") {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.reset_uv_camera();
                }
                self.redraw.requested = true;
            }
            return;
        }

        // The Tex workspace is a pure-egui 2D viewer; only F / R apply, refitting
        // the image. The request makes the texture view ease to the fitted view on
        // its next paint (the animated counterpart of the zoom-readout toggle).
        if self.ui.mode == WorkspaceMode::Texture {
            if event.state == ElementState::Pressed && matches!(key.as_str(), "f" | "r") {
                self.ui.texture_view.request = Some(TexViewRequest::Fit);
                self.redraw.requested = true;
            }
            return;
        }

        // Each 45° orbit step (radians). Sign maps the requested side to the
        // yaw/pitch convention in `OrbitCamera` (negative pitch lifts the eye up).
        const ORBIT_STEP: f32 = std::f32::consts::FRAC_PI_4;

        // WASD orbits fire on key *release*: holding a key emits a burst of
        // repeat key-down events (which would snap the camera with no animation),
        // but exactly one release — so a brief hold animates a single clean step.
        if event.state == ElementState::Released {
            match key.as_str() {
                "a" => self.orbit_camera_step(ORBIT_STEP, 0.0),
                "d" => self.orbit_camera_step(-ORBIT_STEP, 0.0),
                "w" => self.orbit_camera_step(0.0, -ORBIT_STEP),
                "s" => self.orbit_camera_step(0.0, ORBIT_STEP),
                _ => return,
            }
            self.redraw.requested = true;
            return;
        }

        // Remaining shortcuts are instant toggles/commands on key-down.
        match key.as_str() {
            "`" => self.ui.debug.wireframe_overlay = !self.ui.debug.wireframe_overlay,
            "1" => self.ui.shading_mode = ShadingMode::Wireframe,
            "2" => self.ui.shading_mode = ShadingMode::Unlit,
            "3" => self.ui.shading_mode = ShadingMode::Shaded,
            "i" => self.ui.show_stats = !self.ui.show_stats,
            "g" => self.ui.show_grid = !self.ui.show_grid,
            // Opt's A/B swap. Flipping which mesh is solid in place is the most
            // reliable way to spot where a simplification moved the silhouette.
            // Only the overlay has a solid mesh to flip: the split draws both,
            // and its stats cards name which side is which, so swapping the
            // halves there would leave them describing the wrong view.
            "x" => {
                if self.ui.mode != WorkspaceMode::Opt
                    || !self.ui.opt.has_result()
                    || self.ui.opt.layout != review_ui::OptLayout::Overlay
                {
                    return;
                }
                self.ui.opt.side = self.ui.opt.side.swapped();
            }
            "f" => self.frame_camera_on_key(),
            "r" => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.animate_camera_to_home();
                }
            }
            _ => return,
        }

        self.redraw.requested = true;
    }

    /// Animate a relative 45° camera orbit (radians) for the WASD shortcuts.
    fn orbit_camera_step(&mut self, yaw_delta: f32, pitch_delta: f32) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_orbit_by(yaw_delta, pitch_delta);
        }
    }

    /// Frame the camera on `F`. With a mesh part (a node) selected, alternate
    /// between framing just that part and the whole model on successive presses;
    /// otherwise (nothing, or a material, selected) always frame the whole model.
    fn frame_camera_on_key(&mut self) {
        // Resolve the selected part's bounds first, before the renderer is borrowed
        // mutably (both borrow `self`). Only a node counts as a "mesh part" here.
        let part_bounds = match self.ui.selection {
            Selection::Node(_) => selection_bounds(&self.scene_model, self.ui.selection),
            _ => None,
        };
        let target = match part_bounds {
            Some(part) => {
                self.frame_showing_selection = !self.frame_showing_selection;
                if self.frame_showing_selection {
                    Some(part)
                } else {
                    self.scene_model.bounds.or(Some(part))
                }
            }
            None => self.scene_model.bounds,
        };
        if let (Some(renderer), Some(bounds)) = (self.renderer.as_mut(), target) {
            renderer.animate_camera_to_bounds(bounds);
        }
    }
}
