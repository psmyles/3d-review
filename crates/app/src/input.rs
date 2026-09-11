//! Window-input routing: the per-event handlers `window_event` dispatches to.
//!
//! Split out of `main.rs` so the `ApplicationHandler::window_event` match stays a
//! thin dispatcher and each gesture's logic is a shallow, named method (rather than
//! one deeply nested match). Data still flows one way: these mutate `App`'s camera /
//! drag state and request redraws, never the renderer's internals beyond its public
//! camera API (invariant 2).

use std::time::Instant;

use glam::Vec2;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::window::Window;

use review_ui::{OptLayout, WorkspaceMode};

use crate::App;
use crate::window_state::monitor_refresh_interval;

impl App {
    /// A window resize: re-derive the monitor frame cap (a resize may follow a move
    /// to another monitor), flag the windowed bounds for a deferred resample, update
    /// the camera aspect ratios + framing safe-area, and resize the swapchain.
    pub(crate) fn handle_resized(&mut self, size: PhysicalSize<u32>, window: &Window) {
        self.redraw.refresh_interval = monitor_refresh_interval(window);
        // Defer recording to `about_to_wait`: a maximize resize lands before winit's
        // maximized flag is set, so sampling here would store maximized geometry.
        self.placement.bounds_dirty = true;

        if let Some(renderer) = self.renderer.as_mut()
            && size.height > 0
        {
            let aspect = size.width as f32 / size.height as f32;
            renderer.set_camera_aspect_ratio(aspect);
            renderer.set_uv_aspect_ratio(aspect);
            let (safe_w, safe_h) = framing_safe_area(size.height, window.scale_factor() as f32);
            renderer.set_framing_safe_area(safe_w, safe_h);
        }

        // Infallible: a zero size (a minimized window) is remembered and the frame
        // skipped, and a backend resize that fails leaves the old buffers for the next
        // frame to draw into. Neither is anything this handler could act on, which is
        // why the swapchain resize stopped returning a `Result` in the sokol port.
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size.width, size.height);
        }

        window.request_redraw();
    }

    /// A mouse-button press/release: drag-mode selection (orbit/look/pan/zoom
    /// depending on workspace and modifiers), startup-help dismissal, and
    /// double-click-to-open.
    /// `egui_consumed` is whether egui claimed this event.
    pub(crate) fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
        egui_consumed: bool,
    ) {
        if state == ElementState::Released {
            self.drag_mode = None;
            // A flight lives only as long as the drag that armed it: a movement
            // key still down when the button comes up must not arm the next one.
            self.flycam.release_all();
            return;
        }

        // The startup help overlay is up: it swallows the click in egui (so the
        // chrome beneath stays inert), but we still drive dismissal here. A plain
        // click hides it; a double-click also opens the file picker — the same
        // gesture as on the empty viewport, so it reuses the same double-click
        // detection. The first click seeds `last_primary_click`; the second arrives
        // after the overlay is gone and opens the dialog via the branch below.
        if button == MouseButton::Left && self.ui.show_help_overlay {
            if self.should_open_on_double_click() {
                self.open_model_from_dialog();
            } else if let Some(position) = self.last_pointer_position {
                self.last_primary_click = Some((Instant::now(), position));
            }
            self.ui.show_help_overlay = false;
            self.redraw.requested = true;
            return;
        }

        if egui_consumed {
            return;
        }

        // The UV viewport is a 2D pan/zoom workspace: LMB pans, RMB zooms (down =
        // in). The 3D scene is LMB orbit / RMB look / MMB pan. The Tex viewport
        // handles its own pan/zoom inside egui (its canvas senses the drag), so
        // an unclaimed press there must not start a 3D-camera drag.
        let uv_mode = match self.ui.mode {
            WorkspaceMode::Uv => true,
            WorkspaceMode::ThreeD | WorkspaceMode::Opt => false,
            WorkspaceMode::Texture => return,
        };
        // Which Opt split view this drag belongs to, fixed at press time so the
        // drag doesn't switch cameras if the pointer crosses the divider.
        self.drag_in_opt_right_view = self
            .last_pointer_position
            .is_some_and(|position| self.in_opt_right_view(position));
        match button {
            MouseButton::Left => {
                if self.should_open_on_double_click() {
                    self.open_model_from_dialog();
                    self.drag_mode = None;
                } else {
                    self.drag_mode = Some(if uv_mode {
                        DragMode::Pan
                    } else {
                        DragMode::Orbit
                    });
                    if let Some(position) = self.last_pointer_position {
                        self.last_primary_click = Some((Instant::now(), position));
                    }
                }
            }
            MouseButton::Right => {
                // UV mode: RMB zoom-drags. 3D: Alt+RMB zoom-drags (down = in, up =
                // out), Shift+RMB pans, plain RMB looks around — and, while it is
                // held, arms the WASD/QE flycam (`flycam.rs`), which is the gesture
                // Unity and Unreal both bind.
                //
                // Shift+RMB is there because pan lost its plain-RMB binding to the
                // look drag and the middle button it moved to is not a button every
                // pointing device has (a Mac trackpad has none).
                self.drag_mode = Some(if uv_mode || self.modifiers.alt_key() {
                    DragMode::Zoom
                } else if self.modifiers.shift_key() {
                    DragMode::Pan
                } else {
                    DragMode::Look
                });
            }
            MouseButton::Middle => {
                self.drag_mode = Some(DragMode::Pan);
            }
            _ => {}
        }
    }

    /// Pointer motion: drive the active drag (orbit/look/pan/zoom, 2D in UV mode) and
    /// record the new pointer position for the next delta.
    pub(crate) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>, window: &Window) {
        let current = Vec2::new(position.x as f32, position.y as f32);

        // Read before the renderer borrow below.
        let synced = self.opt_cameras_synced();
        let half_view = self.opt_split_metrics().map(|(_, size)| size);
        if let (Some(renderer), Some(last), Some(mode)) = (
            self.renderer.as_mut(),
            self.last_pointer_position,
            self.drag_mode,
        ) {
            let delta = current - last;
            let uv_mode = self.ui.mode == WorkspaceMode::Uv;
            let size = window.inner_size();
            let viewport = Vec2::new(size.width as f32, size.height as f32);
            // In the Opt split view with sync off, the drag belongs to whichever
            // half it started in — decided at press time, so a drag that wanders
            // across the divider keeps moving the camera it began with.
            let opt_right = self.drag_in_opt_right_view;
            match mode {
                DragMode::Orbit => {
                    if opt_right {
                        renderer.orbit_opt_camera(delta);
                    } else {
                        renderer.orbit_camera(delta);
                    }
                }
                // Look turns the camera in place instead of around the pivot. 3D
                // only — the UV viewport binds RMB to its zoom drag, so it never
                // starts one.
                DragMode::Look => {
                    if opt_right {
                        renderer.look_opt_camera(delta);
                    } else {
                        renderer.look_camera(delta);
                    }
                }
                // Pan drives the 2D UV camera in UV mode, the 3D camera otherwise.
                DragMode::Pan => {
                    if uv_mode {
                        renderer.pan_uv_camera(delta, viewport);
                    } else if opt_right {
                        renderer.pan_opt_camera(delta, half_view.unwrap_or(viewport));
                    } else {
                        // A pan is scaled by the view it happens in, so in the
                        // split it covers the same world distance as one across a
                        // half-width window rather than a full one.
                        renderer.pan_camera(delta, half_view.unwrap_or(viewport));
                    }
                }
                // Pointer down (positive screen delta) zooms in, up zooms out —
                // matching the wheel's positive-is-in sign.
                DragMode::Zoom => {
                    if uv_mode {
                        renderer.zoom_uv_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    } else if opt_right {
                        renderer.zoom_opt_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    } else {
                        renderer.zoom_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    }
                }
            }
            // With the views synced, the right camera simply follows the left, so
            // dragging either moves both.
            if synced {
                renderer.sync_opt_camera();
            }
            self.redraw.requested = true;
        }

        self.last_pointer_position = Some(current);
    }

    /// Whether the Opt workspace is currently showing two views that share a
    /// camera. False in every other workspace, and in the single-view overlay —
    /// which has only one camera to begin with.
    pub(crate) fn opt_cameras_synced(&self) -> bool {
        self.ui.mode == WorkspaceMode::Opt
            && (self.ui.opt.layout == OptLayout::Overlay || self.ui.opt.camera_sync)
    }

    /// Whether a pointer position falls in the *right* half of an unsynced Opt
    /// split view — the half driven by the second camera.
    fn in_opt_right_view(&self, position: Vec2) -> bool {
        if self.ui.opt.camera_sync {
            return false;
        }
        self.opt_split_metrics()
            .is_some_and(|(divider, _)| position.x >= divider)
    }

    /// Where the Opt split divides, and how big each of its two views is — both
    /// in physical pixels, `None` outside the split layout.
    ///
    /// The renderer lays the split out inside the chrome-free scene area rather
    /// than across the whole window, so the divider is the centre of *that* rect;
    /// deriving it from the window instead would put the pointer in the wrong
    /// view for every pixel between the two centres.
    fn opt_split_metrics(&self) -> Option<(f32, Vec2)> {
        if self.ui.mode != WorkspaceMode::Opt || self.ui.opt.layout != OptLayout::Split {
            return None;
        }
        let rect = self.ui.scene_viewport?;
        let scale = self.window.as_ref()?.scale_factor() as f32;
        Some((
            rect.center().x * scale,
            Vec2::new(rect.width() * 0.5 * scale, rect.height() * scale),
        ))
    }

    /// A scroll-wheel event: zoom the active (2D UV or 3D) camera unless egui claimed
    /// it. Line deltas (mice) and pixel deltas (trackpads) are normalized to one
    /// zoom step.
    pub(crate) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta, egui_consumed: bool) {
        if egui_consumed {
            return;
        }
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINE_ZOOM_STEP,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / WHEEL_PIXELS_PER_ZOOM_STEP,
        };
        // While the look drag is up the wheel sets how fast the flycam moves
        // rather than dollying the camera — where both Unity and Unreal put it.
        if self.adjust_fly_speed(amount) {
            return;
        }
        self.zoom_active_camera(amount);
    }

    /// A trackpad pinch: zoom the active camera, exactly as the wheel does
    /// (`mac-port-plan.md` D16). macOS only in practice — winit reports
    /// `PinchGesture` nowhere else — but routed unconditionally, because which
    /// gestures a platform sends is winit's business and not something to `cfg` on
    /// here.
    ///
    /// `delta` is a *scale fraction* per event (roughly ±0.01–0.05 as the fingers
    /// move), not a pixel count, so it gets its own sensitivity rather than the
    /// wheel's. It is filtered for finiteness before it reaches the camera: a NaN
    /// would propagate straight into the orbit distance and leave the model
    /// permanently gone with nothing to show why.
    pub(crate) fn handle_pinch_gesture(&mut self, delta: f64, egui_consumed: bool) {
        if egui_consumed || !delta.is_finite() {
            return;
        }
        self.zoom_active_camera(delta as f32 * PINCH_ZOOM_STEP);
    }

    /// Apply `amount` zoom steps to whichever camera the current workspace (and, in
    /// the Opt split, the pointer) says is active. Shared by the wheel and the pinch
    /// so the two cannot drift apart on which view they act on.
    fn zoom_active_camera(&mut self, amount: f32) {
        // The wheel acts on whichever Opt split view the pointer is over — unlike
        // a drag there is no press to anchor it to, and hovering is the natural
        // way to say which view you mean.
        let opt_right = self
            .last_pointer_position
            .is_some_and(|position| self.in_opt_right_view(position));
        let synced = self.opt_cameras_synced();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match self.ui.mode {
            WorkspaceMode::Uv => renderer.zoom_uv_camera(amount),
            WorkspaceMode::ThreeD | WorkspaceMode::Opt => {
                if opt_right {
                    renderer.zoom_opt_camera(amount);
                } else {
                    renderer.zoom_camera(amount);
                }
            }
            // The Tex viewport zooms inside egui (its canvas claims the wheel);
            // an unclaimed wheel there must not zoom the hidden 3D camera.
            WorkspaceMode::Texture => return,
        }
        if synced {
            renderer.sync_opt_camera();
        }
        self.redraw.requested = true;
    }
}

/// Camera zoom per pixel of a right-button zoom-drag (pointer-down zooms in).
pub(crate) const DRAG_ZOOM_SENSITIVITY: f32 = 0.01;

/// Camera zoom per wheel notch for line-based scroll deltas (mice).
pub(crate) const WHEEL_LINE_ZOOM_STEP: f32 = 0.5;

/// Pixel-precise scroll (trackpads) divided by this to match one wheel notch.
pub(crate) const WHEEL_PIXELS_PER_ZOOM_STEP: f32 = 120.0;

/// Camera zoom per unit of trackpad pinch scale (`mac-port-plan.md` D16). A pinch
/// delta is a scale *fraction* — a comfortable two-finger spread accumulates to
/// roughly 1.0 over its length — so this is the zoom that whole gesture is worth,
/// not a per-notch step like the wheel's.
pub(crate) const PINCH_ZOOM_STEP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    Orbit,
    /// Turn the camera in place (the right-button drag Unity and Unreal bind).
    /// Also what arms the WASD/QE flycam — see `flycam.rs`.
    Look,
    Pan,
    Zoom,
}

/// Fraction of the window framing should fill, leaving room for the chrome that
/// overlays the full-window 3D scene (toolbar on top, status bar on the bottom)
/// so a framed model doesn't hide under it. Width is left unconstrained — the
/// option panel floats and is transient.
///
/// Both terms are egui points: the window height converted from physical pixels,
/// and the bands' height from [`review_ui::theme::chrome_height`], which scales
/// the design-pixel tokens exactly as the bands themselves are scaled when drawn.
/// Reading those tokens raw against a height in points over-reserved the chrome
/// on every scaled display — half again at 150%, twice over at 200%.
pub(crate) fn framing_safe_area(height_px: u32, scale_factor: f32) -> (f32, f32) {
    let logical_height = height_px as f32 / scale_factor.max(0.1);
    let chrome = review_ui::theme::chrome_height(scale_factor);
    let height_fraction = if logical_height > chrome {
        (logical_height - chrome) / logical_height
    } else {
        1.0
    };
    (1.0, height_fraction.clamp(0.4, 1.0))
}

#[cfg(test)]
mod tests {
    use super::framing_safe_area;

    /// The chrome tokens are design pixels, scaled to points exactly as the bands
    /// that draw them are — so the band reserved for the chrome is the same slice
    /// of the window at every display scale. Reading them raw against a height
    /// already in points instead over-reserved 46 points at 150% and 69 at 200%,
    /// framing every loaded model visibly small on a HiDPI display.
    #[test]
    fn the_framing_safe_area_holds_across_display_scales() {
        let (width, unscaled) = framing_safe_area(1000, 1.0);
        let (_, scaled) = framing_safe_area(1000, 2.0);
        assert_eq!(width, 1.0);
        assert!((unscaled - scaled).abs() < 1e-6, "{unscaled} vs {scaled}");
        // 1000 physical pixels of window, less the 73 + 64 design pixels of bands.
        assert!((scaled - 0.863).abs() < 1e-4, "{scaled}");
    }

    /// A window shorter than its own chrome has no band left to frame into, so it
    /// frames against the whole window rather than a zero (or negative) fraction.
    #[test]
    fn a_window_shorter_than_the_chrome_frames_whole() {
        assert_eq!(framing_safe_area(100, 1.0), (1.0, 1.0));
        assert_eq!(framing_safe_area(100, 0.0), (1.0, 1.0));
    }
}
