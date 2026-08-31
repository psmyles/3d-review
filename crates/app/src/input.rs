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

use crate::window_state::monitor_refresh_interval;
use crate::{
    App, DRAG_ZOOM_SENSITIVITY, DragMode, WHEEL_LINE_ZOOM_STEP, WHEEL_PIXELS_PER_ZOOM_STEP,
    framing_safe_area,
};

impl App {
    /// A window resize: re-derive the monitor frame cap (a resize may follow a move
    /// to another monitor), flag the windowed bounds for a deferred resample, update
    /// the camera aspect ratios + framing safe-area, and resize the swapchain (a
    /// failed resize is non-fatal — the old buffers stay valid for this frame).
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

        // Non-fatal on its own (the old buffers stay valid, and the next successful
        // resize recovers), but a resize that keeps failing means a wedged device —
        // so it goes through the same once-per-session fault report as the frame
        // loop's, which is also what keeps a dragged window edge from stacking a
        // toast per event.
        let resize_error = self
            .gpu
            .as_mut()
            .and_then(|gpu| gpu.resize(size.width, size.height).err());
        if let Some(error) = resize_error {
            self.report_gpu_fault("Swapchain resize failed", error);
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
            self.redraw.requested = true;
            return;
        }

        if egui_consumed {
            return;
        }

        // The UV viewport is a 2D pan/zoom workspace: LMB pans, RMB zooms (down =
        // in). The 3D scene keeps LMB orbit / RMB pan-or-zoom. The Tex viewport
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
