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

use review_ui::WorkspaceMode;

use crate::{
    App, DRAG_ZOOM_SENSITIVITY, DragMode, WHEEL_LINE_ZOOM_STEP, WHEEL_PIXELS_PER_ZOOM_STEP,
    framing_safe_area, monitor_refresh_interval,
};

impl App {
    /// A window resize: re-derive the monitor frame cap (a resize may follow a move
    /// to another monitor), flag the windowed bounds for a deferred resample, update
    /// the camera aspect ratios + framing safe-area, and resize the swapchain (a
    /// failed resize is non-fatal — the old buffers stay valid for this frame).
    pub(crate) fn handle_resized(&mut self, size: PhysicalSize<u32>, window: &Window) {
        self.refresh_interval = monitor_refresh_interval(window);
        // Defer recording to `about_to_wait`: a maximize resize lands before winit's
        // maximized flag is set, so sampling here would store maximized geometry.
        self.windowed_bounds_dirty = true;

        if let Some(renderer) = self.renderer.as_mut()
            && size.height > 0
        {
            let aspect = size.width as f32 / size.height as f32;
            renderer.set_camera_aspect_ratio(aspect);
            renderer.set_uv_aspect_ratio(aspect);
            let (safe_w, safe_h) = framing_safe_area(size.height, window.scale_factor() as f32);
            renderer.set_framing_safe_area(safe_w, safe_h);
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let _ = gpu.resize(size.width, size.height);
        }

        window.request_redraw();
    }

    /// A mouse-button press/release: drag-mode selection (orbit/pan/zoom depending on
    /// workspace and modifiers), startup-help dismissal, and double-click-to-open.
    /// `egui_consumed` is whether egui claimed this event.
    pub(crate) fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
        egui_consumed: bool,
    ) {
        if state == ElementState::Released {
            self.drag_mode = None;
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
            self.redraw_requested = true;
            return;
        }

        if egui_consumed {
            return;
        }

        // The UV viewport is a 2D pan/zoom workspace: LMB pans, RMB zooms (down =
        // in). The 3D scene keeps LMB orbit / RMB pan-or-zoom.
        let uv_mode = self.ui.mode == WorkspaceMode::Uv;
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
                // out), plain RMB pans.
                self.drag_mode = Some(if uv_mode || self.modifiers.alt_key() {
                    DragMode::Zoom
                } else {
                    DragMode::Pan
                });
            }
            MouseButton::Middle => {
                self.drag_mode = Some(DragMode::Pan);
            }
            _ => {}
        }
    }

    /// Pointer motion: drive the active drag (orbit/pan/zoom, 2D in UV mode) and
    /// record the new pointer position for the next delta.
    pub(crate) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>, window: &Window) {
        let current = Vec2::new(position.x as f32, position.y as f32);

        if let (Some(renderer), Some(last), Some(mode)) = (
            self.renderer.as_mut(),
            self.last_pointer_position,
            self.drag_mode,
        ) {
            let delta = current - last;
            let uv_mode = self.ui.mode == WorkspaceMode::Uv;
            let size = window.inner_size();
            let viewport = Vec2::new(size.width as f32, size.height as f32);
            match mode {
                DragMode::Orbit => renderer.orbit_camera(delta),
                // Pan drives the 2D UV camera in UV mode, the 3D camera otherwise.
                DragMode::Pan => {
                    if uv_mode {
                        renderer.pan_uv_camera(delta, viewport);
                    } else {
                        renderer.pan_camera(delta, viewport);
                    }
                }
                // Pointer down (positive screen delta) zooms in, up zooms out —
                // matching the wheel's positive-is-in sign.
                DragMode::Zoom => {
                    if uv_mode {
                        renderer.zoom_uv_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    } else {
                        renderer.zoom_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    }
                }
            }
            self.redraw_requested = true;
        }

        self.last_pointer_position = Some(current);
    }

    /// A scroll-wheel event: zoom the active (2D UV or 3D) camera unless egui claimed
    /// it. Line deltas (mice) and pixel deltas (trackpads) are normalized to one
    /// zoom step.
    pub(crate) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta, egui_consumed: bool) {
        if egui_consumed {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINE_ZOOM_STEP,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / WHEEL_PIXELS_PER_ZOOM_STEP,
        };
        if self.ui.mode == WorkspaceMode::Uv {
            renderer.zoom_uv_camera(amount);
        } else {
            renderer.zoom_camera(amount);
        }
        self.redraw_requested = true;
    }
}
