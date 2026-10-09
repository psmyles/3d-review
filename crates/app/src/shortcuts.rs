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
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::App;
use crate::flycam::FlyDirection;

/// Whether the **primary** modifier — the one file and edit commands chord with —
/// is held: `Ctrl` on Windows and Linux, `Cmd` on macOS (`docs/ARCHITECTURE.md`,
/// Platform decisions D10).
///
/// Ctrl is not an acceptable file-command modifier on a Mac, where `Ctrl`+click is
/// a *secondary* click and `Cmd`+O is what every application binds. The bare-key
/// camera shortcuts below are unaffected — they fire only with no modifier at all,
/// on either OS — and the `Alt`+RMB zoom drag in [`crate::input`] stays `Alt`
/// (`Option` on a Mac), which is the DCC convention rather than an OS one.
///
/// The user-facing name for this key in the chrome is `review_ui`'s
/// `primary_key!`.
pub(crate) fn primary_held(modifiers: ModifiersState) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    }
}

impl App {
    /// Dispatch a viewport keyboard shortcut. Keys modified by the primary
    /// modifier ([`primary_held`]) are file commands; the bare keys are view /
    /// camera shortcuts and only fire when no modifier is held (so every
    /// combination stays free). Everything here acts on key-down except the six
    /// flycam movement keys, which are *held* rather than pressed.
    /// Keyboard events egui has already consumed are filtered out by the caller.
    pub(crate) fn handle_keyboard_shortcut(&mut self, event: &KeyEvent) {
        // Escape clears the whole selection — the primary, both multi-selection
        // sets and the range anchor — whether it was made in the Outliner or by
        // clicking the viewport. It's a Named key, so handle it before the
        // Character extraction below.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::Escape))
        {
            if self.ui.has_selection() {
                self.ui.clear_selection();
                self.redraw.requested = true;
            }
            return;
        }

        // Space plays / pauses the selected clip. A Named key like Escape, so it
        // is handled before the Character extraction; bare (no modifiers), 3D
        // workspace only, and only while a clip is selected — otherwise it stays
        // unbound.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::Space))
        {
            if self.modifiers.is_empty()
                && self.ui.mode == WorkspaceMode::ThreeD
                && self.ui.animation.selected_clip.is_some()
            {
                self.toggle_playback();
                self.redraw.requested = true;
            }
            return;
        }

        // F1 opens the manual, at the page for whatever the pointer is over: an
        // open options panel's own page, or the current workspace's. Named like
        // Escape and Space, so it comes before the Character extraction.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::F1))
        {
            self.open_context_help();
            return;
        }

        let Key::Character(character) = &event.logical_key else {
            return;
        };

        if primary_held(self.modifiers) {
            // File commands fire on key-down only.
            if event.state != ElementState::Pressed {
                return;
            }
            if character.eq_ignore_ascii_case("n") {
                self.reset_to_start_state();
            } else if character.eq_ignore_ascii_case("o") {
                self.open_model_from_dialog();
            } else if character.eq_ignore_ascii_case("s") {
                // Primary+S saves the comments back into the file; with Shift,
                // into a new one.
                if self.ui.comments.writable() {
                    if self.modifiers.shift_key() {
                        self.save_comments_as();
                    } else {
                        self.save_comments(crate::comments_save::AfterSave::Nothing);
                    }
                }
            } else if character.eq_ignore_ascii_case("z") {
                // Primary+Z undoes; Primary+Shift+Z redoes (the common alt-redo
                // chord, and the only redo chord on a Mac).
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

        // Matched case-insensitively, exactly like the chords above: winit's
        // `ModifiersState` carries no Caps Lock bit, so the guard above can't see
        // it — with Caps Lock on the key arrives as "G" and an exact match would
        // leave every one of these shortcuts dead.
        let key = character.to_ascii_lowercase();

        // The flycam's six movement keys (`flycam.rs`) are *held*, not pressed:
        // they only set a direction bit here, and `step_flycam` moves the camera
        // once per frame while the right button is down. None of them is bound to
        // anything else, so an unarmed press is simply inert.
        //
        // A release is honoured whatever modifiers are down and whatever
        // workspace is up: pressing a modifier (or switching view) mid-flight
        // must not leave a direction stuck on with no key left to clear it.
        if let Some(direction) = FlyDirection::from_key(&key)
            && self.fly_key_applies(event.state == ElementState::Pressed)
        {
            let pressed = event.state == ElementState::Pressed;
            if pressed && (!self.modifiers.is_empty() || !self.ui.mode.is_scene()) {
                return;
            }
            self.set_fly_key(direction, pressed);
            self.redraw.requested = true;
            return;
        }

        if !self.modifiers.is_empty() {
            return;
        }

        // In the UV workspace the 3D display shortcuts (shading / grid) don't
        // apply; only F / R, which reframe the 2D UV view.
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

        // Every remaining shortcut is an instant toggle / command on key-down.
        if event.state != ElementState::Pressed {
            return;
        }
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
            // Switches the left button between turning the camera and picking.
            // `Q` is also the flycam's "down", but only while the right button
            // is held, which is what `fly_key_applies` above separates.
            "q" => {
                if !self.ui.mode.is_scene() {
                    return;
                }
                self.ui.tool = self.ui.tool.toggled();
                // Nothing is under the pointer as far as View mode is
                // concerned, and a stale highlight would outlive the tool.
                if self.ui.tool == review_ui::ViewportTool::View {
                    self.set_hover(None);
                }
            }
            // Switches the left button between turning the camera and leaving
            // review comments, in the 3D workspace.
            "c" => {
                if self.ui.mode != WorkspaceMode::ThreeD {
                    return;
                }
                self.ui.tool = self.ui.tool.toggled_comment();
                if self.ui.tool != review_ui::ViewportTool::Select {
                    self.set_hover(None);
                }
            }
            "f" => self.frame_camera_on_key(),
            // Single-frame stepping through the selected clip (pauses playback).
            "," | "." => {
                if self.ui.mode != WorkspaceMode::ThreeD
                    || self.ui.animation.selected_clip.is_none()
                {
                    return;
                }
                self.step_frame(if key == "," { -1 } else { 1 });
            }
            "r" => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.animate_camera_to_home();
                }
            }
            _ => return,
        }

        self.redraw.requested = true;
    }

    /// Frame the camera on `F`. With a mesh part (a node) selected, alternate
    /// between framing just that part and the whole model on successive presses;
    /// otherwise (nothing, or a material, selected) always frame the whole model.
    fn frame_camera_on_key(&mut self) {
        // Resolve the selected part's bounds first, before the renderer is borrowed
        // mutably (both borrow `self`). Only a node counts as a "mesh part" here.
        let part_bounds = match self.ui.selection {
            // The whole selected set, so framing wraps everything highlighted
            // rather than only the last part clicked.
            Selection::Node(_) => selection_bounds(
                &self.scene_model,
                self.ui.selection,
                &self.ui.selected_node_set(),
            ),
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
            // `ui.bounds` is the selected clip's motion envelope while a clip is
            // selected, else the model's own bounds — so F frames the whole
            // motion rather than a single pose.
            None => self.ui.bounds.or(self.scene_model.bounds),
        };
        if let (Some(renderer), Some(bounds)) = (self.renderer.as_mut(), target) {
            renderer.animate_camera_to_bounds(bounds);
        }
    }

    /// Open the manual at whatever the pointer is over.
    ///
    /// An open options panel wins: the pointer resting on one is a specific
    /// question, and `OptionPanel::window_id` gives every panel window a stable
    /// egui layer id to match against. Otherwise the workspace's own page, which
    /// is the general question F1 is usually asking.
    fn open_context_help(&mut self) {
        let panel = self.egui_ctx.as_ref().and_then(|ctx| {
            let pointer = ctx.input(|input| input.pointer.hover_pos())?;
            let layer = ctx.layer_id_at(pointer)?;
            review_ui::option_panel_at(layer)
        });
        let page = panel.unwrap_or_else(|| review_ui::workspace_help_page(self.ui.mode));
        self.ui.help.open_page(page);
        self.redraw.requested = true;
    }
}
