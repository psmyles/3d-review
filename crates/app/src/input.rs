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
    /// depending on workspace and modifiers) and double-click-to-open.
    /// `egui_consumed` is whether egui claimed this event.
    pub(crate) fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
        egui_consumed: bool,
    ) {
        if state == ElementState::Released {
            // A press that never moved is a click, and in Select mode a click
            // picks. Read before the drag state is cleared: `pending_drag` (or a
            // settled `drag_mode`) is what says egui did *not* take the press —
            // a press it grabbed leaves both `None`, and must not also select
            // something under the panel the user was actually dragging.
            let unclaimed = self.pending_drag.is_some() || self.drag_mode.is_some();
            let click = (button == MouseButton::Left)
                .then(|| self.click_press.take())
                .flatten();

            self.drag_mode = None;
            self.pending_drag = None;
            // A flight lives only as long as the drag that armed it: a movement
            // key still down when the button comes up must not arm the next one.
            self.flycam.release_all();

            if let Some(position) = click
                && unclaimed
            {
                // The primary modifier is Cmd on macOS, which is `app`'s own
                // distinction to make (`primary_held`) rather than one to read
                // back out of egui.
                let mode = review_ui::SelectMode::new(
                    crate::shortcuts::primary_held(self.modifiers),
                    self.modifiers.shift_key(),
                );
                self.pick_click(position, mode);
            }
            return;
        }

        if egui_consumed {
            return;
        }

        // The UV viewport is a 2D pan/zoom workspace: LMB pans, RMB zooms (down =
        // in). The 3D scene is LMB orbit / RMB look / MMB pan. The Tex viewport
        // handles its own pan/zoom inside egui (its canvas senses the drag), so
        // an unclaimed press there must not start a 3D-camera drag.
        // A press in the Aud split's UV half drives that half's 2D camera, with the
        // UV workspace's own bindings.
        self.drag_in_aud_uv = self
            .last_pointer_position
            .is_some_and(|position| self.in_aud_uv_view(position));
        let uv_mode = match self.ui.mode {
            WorkspaceMode::Uv => true,
            _ if self.drag_in_aud_uv => true,
            WorkspaceMode::ThreeD | WorkspaceMode::Opt | WorkspaceMode::Aud => false,
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
                    self.pending_drag = None;
                } else {
                    self.pending_drag = Some(if uv_mode {
                        DragMode::Pan
                    } else {
                        DragMode::Orbit
                    });
                    if let Some(position) = self.last_pointer_position {
                        self.last_primary_click = Some((Instant::now(), position));
                        // Where a click would land, if this press turns out not
                        // to be the start of a drag.
                        self.click_press = self.picking_enabled().then_some(position);
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
                self.pending_drag = Some(if uv_mode || self.modifiers.alt_key() {
                    DragMode::Zoom
                } else if self.modifiers.shift_key() {
                    DragMode::Pan
                } else {
                    DragMode::Look
                });
            }
            MouseButton::Middle => {
                self.pending_drag = Some(DragMode::Pan);
            }
            _ => {}
        }
    }

    /// Turn the press a frame ago into a live camera drag, unless egui took it.
    ///
    /// A press cannot be judged as it arrives. `egui_winit` answers "did egui
    /// claim this?" out of the *previous* frame's layout, and a resize handle is
    /// exactly where that answer is wrong: egui straddles a panel's or a window's
    /// grab zone across its edge, so the outer half of it sits over what the last
    /// frame still considered free viewport. The press came back unclaimed, armed
    /// an orbit, and the scene then spun under the reader for the whole of the
    /// resize - the drag itself going to egui, its motion going to the camera as
    /// well.
    ///
    /// So a press only *proposes* a drag ([`App::pending_drag`]), and this runs
    /// once the egui pass that resolves it has: if egui grabbed a widget with it,
    /// the proposal is dropped, otherwise it becomes the live drag. The cost is a
    /// frame of latency on starting an orbit, which is inside the redraw pacing
    /// interval and so not something a hand can feel.
    ///
    /// `egui_is_using_pointer` and not `egui_wants_pointer_input`: the question
    /// here is only whether egui *grabbed* something - merely hovering an area was
    /// already answered - correctly - when the press arrived.
    pub(crate) fn settle_pending_drag(&mut self, egui_ctx: &egui::Context) {
        let Some(mode) = self.pending_drag.take() else {
            return;
        };
        if !egui_ctx.egui_is_using_pointer() {
            self.drag_mode = Some(mode);
        }
    }

    /// Pointer motion: drive the active drag (orbit/look/pan/zoom, 2D in UV mode) and
    /// record the new pointer position for the next delta.
    pub(crate) fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>, window: &Window) {
        let current = Vec2::new(position.x as f32, position.y as f32);

        // Past a few pixels the press is a drag, not a click: the camera keeps
        // it and nothing will be selected when the button comes up. The
        // threshold is what lets one button do both jobs without a modifier.
        if self
            .click_press
            .is_some_and(|press| !crate::pick::is_click(press, current))
        {
            self.click_press = None;
        }
        // Hovering previews what a click would pick. Not while a drag is live —
        // the pointer is driving the camera then, and whatever it sweeps over is
        // not being pointed at.
        if self.picking_enabled() && self.drag_mode.is_none() && self.pending_drag.is_none() {
            self.hover_pending = Some(current);
        }

        // Read before the renderer borrow below.
        let synced = self.opt_cameras_synced();
        let half_view = self
            .opt_split_halves()
            .or_else(|| self.aud_split_halves())
            .map(|[(_, size), _]| size);
        if let (Some(renderer), Some(last), Some(mode)) = (
            self.renderer.as_mut(),
            self.last_pointer_position,
            self.drag_mode,
        ) {
            let delta = current - last;
            let uv_mode = self.ui.mode == WorkspaceMode::Uv;
            let aud_uv = self.drag_in_aud_uv;
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
                    if aud_uv {
                        renderer.pan_aud_uv_camera(delta, half_view.unwrap_or(viewport));
                    } else if uv_mode {
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
                    if aud_uv {
                        renderer.zoom_aud_uv_camera(delta.y * DRAG_ZOOM_SENSITIVITY);
                    } else if uv_mode {
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
        self.opt_split_halves()
            .is_some_and(|[_, (right, _)]| position.x >= right.x)
    }

    /// The Opt split's two views, left then right, each as `(top-left, size)` in
    /// the physical pixels a pointer position is measured in; `None` outside the
    /// split layout.
    ///
    /// The renderer lays the split out inside the chrome-free scene area rather
    /// than across the whole window, so the halves are `ui`'s
    /// [`review_ui::split_halves`] of *that* rect — the same call the divider and
    /// the dimension labels make. Deriving them from the window instead would put
    /// the pointer in the wrong view for every pixel between the two centres.
    pub(crate) fn opt_split_halves(&self) -> Option<[(Vec2, Vec2); 2]> {
        if self.ui.mode != WorkspaceMode::Opt || self.ui.opt.layout != OptLayout::Split {
            return None;
        }
        let rect = self.ui.scene_viewport?;
        let scale = self.window.as_ref()?.scale_factor() as f32;
        let physical = |half: egui::Rect| {
            (
                Vec2::new(half.left(), half.top()) * scale,
                Vec2::new(half.width(), half.height()) * scale,
            )
        };
        let (left, right) = review_ui::split_halves(rect);
        Some([physical(left), physical(right)])
    }

    /// The Aud split's two halves (the model, then its UV layout) in physical
    /// pixels, as [`Self::opt_split_halves`]; `None` when Aud is not split.
    pub(crate) fn aud_split_halves(&self) -> Option<[(Vec2, Vec2); 2]> {
        if self.ui.mode != WorkspaceMode::Aud || !self.ui.aud.split() {
            return None;
        }
        let rect = self.ui.scene_viewport?;
        let scale = self.window.as_ref()?.scale_factor() as f32;
        let physical = |half: egui::Rect| {
            (
                Vec2::new(half.left(), half.top()) * scale,
                Vec2::new(half.width(), half.height()) * scale,
            )
        };
        let (left, right) = review_ui::split_halves(rect);
        Some([physical(left), physical(right)])
    }

    /// Whether a pointer position falls in the Aud split's UV half.
    pub(crate) fn in_aud_uv_view(&self, position: Vec2) -> bool {
        self.aud_split_halves()
            .is_some_and(|[_, (right, _)]| position.x >= right.x)
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
    /// (`docs/ARCHITECTURE.md`, Platform decisions D16). macOS only in practice —
    /// winit reports `PinchGesture` nowhere else — but routed unconditionally,
    /// because which gestures a platform sends is winit's business and not
    /// something to `cfg` on here.
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
        let aud_uv = self
            .last_pointer_position
            .is_some_and(|position| self.in_aud_uv_view(position));
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if aud_uv {
            renderer.zoom_aud_uv_camera(amount);
            self.redraw.requested = true;
            return;
        }
        match self.ui.mode {
            WorkspaceMode::Uv => renderer.zoom_uv_camera(amount),
            WorkspaceMode::ThreeD | WorkspaceMode::Opt | WorkspaceMode::Aud => {
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

/// How far the pointer may move between press and release and still count as a
/// click rather than a drag. Generous enough to absorb the shake of clicking a
/// mouse, tight enough that a deliberate orbit never selects anything.
pub(crate) const CLICK_MAX_DISTANCE_PX: f32 = 4.0;

/// Camera zoom per pixel of a right-button zoom-drag (pointer-down zooms in).
pub(crate) const DRAG_ZOOM_SENSITIVITY: f32 = 0.01;

/// Camera zoom per wheel notch for line-based scroll deltas (mice).
pub(crate) const WHEEL_LINE_ZOOM_STEP: f32 = 0.5;

/// Pixel-precise scroll (trackpads) divided by this to match one wheel notch.
pub(crate) const WHEEL_PIXELS_PER_ZOOM_STEP: f32 = 120.0;

/// Camera zoom per unit of trackpad pinch scale (`docs/ARCHITECTURE.md`, Platform
/// decisions D16). A pinch delta is a scale *fraction* — a comfortable two-finger
/// spread accumulates to roughly 1.0 over its length — so this is the zoom that
/// whole gesture is worth, not a per-notch step like the wheel's.
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
/// and the bands' height from [`review_ui::theme::chrome_height`], which is the
/// sum of the two band tokens as drawn. `scale_factor` therefore enters only
/// through the height conversion — the chrome is a fixed number of points, so it
/// takes a *larger* share of a HiDPI window, which has fewer points for the same
/// pixels. That is the whole point: the bands stay one apparent size on screen.
pub(crate) fn framing_safe_area(height_px: u32, scale_factor: f32) -> (f32, f32) {
    // A degenerate scale (a window that reports 0, or worse) means "unknown", not
    // "infinitely dense": fall back to 1:1 rather than dividing the height up.
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let logical_height = height_px as f32 / scale;
    let chrome = review_ui::theme::chrome_height();
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

    /// The chrome is a fixed number of *points*, so it reserves a bigger fraction
    /// of a HiDPI window: the same 1000 physical pixels are 1000 points at 100%
    /// but only 500 at 200%, and a band that keeps one apparent size on screen has
    /// to eat twice the share of them.
    ///
    /// Derived from [`review_ui::theme::chrome_height`] rather than written out,
    /// so retuning a band's token can't silently invalidate the expectation — an
    /// earlier revision of this test hardcoded the figures and went stale the first
    /// time the toolbar was resized.
    #[test]
    fn the_framing_safe_area_reserves_the_chrome_in_points() {
        let chrome = review_ui::theme::chrome_height();
        let (width, unscaled) = framing_safe_area(1000, 1.0);
        let (_, scaled) = framing_safe_area(1000, 2.0);
        assert_eq!(width, 1.0);
        assert!(
            (unscaled - (1000.0 - chrome) / 1000.0).abs() < 1e-6,
            "{unscaled}"
        );
        assert!((scaled - (500.0 - chrome) / 500.0).abs() < 1e-6, "{scaled}");
        assert!(
            scaled < unscaled,
            "{scaled} should reserve more than {unscaled}"
        );
    }

    /// A window shorter than its own chrome has no band left to frame into, so it
    /// frames against the whole window rather than a zero (or negative) fraction.
    /// A degenerate scale factor falls back to 1:1 rather than inflating the height.
    #[test]
    fn a_window_shorter_than_the_chrome_frames_whole() {
        let short = review_ui::theme::chrome_height() as u32 - 1;
        assert_eq!(framing_safe_area(short, 1.0), (1.0, 1.0));
        assert_eq!(framing_safe_area(short, 0.0), (1.0, 1.0));
    }
}
