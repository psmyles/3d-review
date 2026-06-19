// Suppress the console window in release builds — a shipped GUI viewer should
// open as a window, not alongside a terminal. Debug builds keep the console so
// `tracing` output stays visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod startup_paint;
mod window_state;

use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use glam::Vec2;
use review_import::{LoadOptions, load_model};
use review_model::{Bvh, ModelData};
use review_render::{
    EGUI_DEPTH_FORMAT, EGUI_MSAA_SAMPLE_COUNT, Renderer, RendererConfig, ShadingMode,
    ibl_supported, ssao_supported, supported_msaa_levels,
};
use review_ui::{
    AxisGizmoAction, UiOutput, UiState, WorkspaceMode, draw_overlay, draw_startup_fade,
    draw_viewport_scene, install_fonts,
};
use tracing::{info, warn};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState},
    window::{Window, WindowAttributes, WindowId},
};

use window_state::WindowPlacement;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let event_loop = EventLoop::new().context("failed to create winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);

    // A file path passed on the command line (e.g. when Windows launches the exe
    // for a double-clicked `.fbx` via the registered file association) is loaded
    // once the window is up. See `resumed`.
    let initial_model = std::env::args_os().nth(1).map(PathBuf::from);

    let mut app = App {
        initial_model,
        ..App::default()
    };
    event_loop
        .run_app(&mut app)
        .context("application event loop failed")
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: Option<egui::Context>,
    egui_state: Option<egui_winit::State>,
    egui_painter: Option<egui_wgpu::winit::Painter>,
    drag_mode: Option<DragMode>,
    last_pointer_position: Option<Vec2>,
    last_primary_click: Option<(Instant, Vec2)>,
    /// Latest keyboard modifier state, tracked from `ModifiersChanged` so
    /// per-key events (which don't carry modifiers in winit) can test for
    /// chords like Ctrl+N.
    modifiers: ModifiersState,
    last_render_instant: Option<Instant>,
    /// When egui has asked to be repainted at a future time (e.g. a UI fade
    /// animation). Drives `ControlFlow::WaitUntil` so the loop sleeps until then
    /// instead of spinning. `None` = wait for the next input/redraw event.
    repaint_at: Option<Instant>,
    /// An interactive event (drag, hover, wheel, key) has requested a redraw.
    /// Folded into the paced `repaint_at` schedule in `about_to_wait` rather than
    /// triggering an immediate `request_redraw`, so a high-polling-rate mouse or
    /// key auto-repeat can't drive rendering faster than the monitor refresh.
    redraw_requested: bool,
    /// Minimum spacing between continuously-rendered frames, derived from the
    /// active monitor's refresh rate. Caps redraw to the display so animation
    /// doesn't render faster than it can be shown (the swapchain doesn't pace us
    /// on the Vulkan path). Defaults to 60 Hz until a monitor is known.
    refresh_interval: Duration,
    scene_model: Arc<ModelData>,
    scene_revision: u64,
    /// Triangle BVH over [`Self::scene_model`], used to occlude the bounding-box
    /// dimension labels against the mesh. Built lazily the first frame the labels
    /// need it (the bounding-box view is on) and reused across frames; a heavy
    /// per-model structure we don't pay for unless the feature is used. `None`
    /// until built; [`Self::occlusion_bvh_revision`] tracks which model it covers
    /// so it rebuilds when a new model loads.
    occlusion_bvh: Option<Bvh>,
    occlusion_bvh_revision: u64,
    ui: UiState,
    /// Model to load once the window/renderer exist, taken from the command line
    /// (file association / `3d-review.exe <path>`). Consumed in `resumed`.
    initial_model: Option<PathBuf>,
    /// The process was launched with a request to start maximized (e.g. a
    /// shortcut set to **Run: Maximized**). Set in `resumed` and applied at
    /// window creation, since winit doesn't honor the OS hint on its own.
    start_maximized: bool,
    /// The most recent *non-maximized* window placement (outer position + inner
    /// size), tracked from `Moved`/`Resized` events so it's available to persist
    /// on exit. Recorded only while the window isn't maximized, so un-maximizing
    /// a restored session returns to a real window rather than a fullscreen rect.
    last_windowed_bounds: Option<((i32, i32), (u32, u32))>,
    /// Launch fade-in progress: while set, each frame paints a full-screen cover
    /// that dissolves from the startup black to the live viewer (see
    /// `startup_paint`). Seeded just before the first frame in `resumed` and
    /// cleared once the fade completes, after which the steady state draws no
    /// cover. Advanced by a *capped per-frame delta* (not absolute wall-clock
    /// time) so a slow first frame — pipeline/shader warm-up, surface acquire,
    /// the OS window-open animation — can't fast-forward the whole fade in one
    /// stall and make the viewer snap in. Same guard `update_camera_animation`
    /// uses for camera transitions.
    fade: Option<FadeProgress>,
}

/// State of the launch fade-in. `elapsed` is animation time accumulated across
/// rendered frames (capped per frame), `last_tick` is when it last advanced.
struct FadeProgress {
    elapsed: Duration,
    last_tick: Instant,
}

/// Duration of the launch fade-in (startup black → viewer). Short on purpose:
/// long enough to read as a dissolve, brief enough not to feel like a wait.
const STARTUP_FADE: Duration = Duration::from_millis(200);

/// Cap on how much the fade advances in a single frame. A startup stall yields
/// one frame's worth of progress instead of the whole gap, so the dissolve
/// always plays across real presented frames rather than being skipped.
const MAX_FADE_STEP: Duration = Duration::from_millis(33);

impl Default for App {
    fn default() -> Self {
        let scene_model = Arc::new(ModelData::default());
        let ui = UiState {
            stats: scene_model.stats,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            ..UiState::default()
        };

        Self {
            window: None,
            renderer: None,
            egui_ctx: None,
            egui_state: None,
            egui_painter: None,
            drag_mode: None,
            last_pointer_position: None,
            last_primary_click: None,
            modifiers: ModifiersState::empty(),
            last_render_instant: None,
            repaint_at: None,
            redraw_requested: false,
            refresh_interval: Duration::from_secs_f64(1.0 / 60.0),
            scene_model,
            scene_revision: 0,
            occlusion_bvh: None,
            // A sentinel distinct from the initial `scene_revision` (0) so the BVH
            // is treated as stale until first built.
            occlusion_bvh_revision: u64::MAX,
            ui,
            initial_model: None,
            start_maximized: false,
            last_windowed_bounds: None,
            fade: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Orbit,
    Pan,
    Zoom,
}

/// Decode the embedded application logo into a winit window icon — used for the
/// title bar, Alt+Tab, and the taskbar button while the app is running. A decode
/// failure is non-fatal: the window simply falls back to the system default.
/// (The *exe* icon for Explorer / pinned shortcuts is embedded separately in
/// `build.rs`.)
fn load_window_icon() -> Option<winit::window::Icon> {
    const PNG: &[u8] = include_bytes!("../../../assets/icons/application-logo.png");
    let image = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    winit::window::Icon::from_rgba(image.into_raw(), width, height).ok()
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        };

        // Honor the OS launch hint (e.g. a shortcut set to "Run: Maximized").
        // winit never consults `STARTUPINFO.wShowWindow`, so we query it and set
        // the initial state ourselves.
        self.start_maximized = review_import::startup_show_maximized();

        // Restore the window to where it was last closed. A placement on a
        // monitor that's no longer connected is dropped so the window can't open
        // off-screen. The OS launch hint still forces maximized regardless.
        let saved =
            window_state::load().filter(|placement| placement_is_visible(event_loop, placement));
        let maximized = self.start_maximized || saved.is_some_and(|placement| placement.maximized);

        let mut attributes = WindowAttributes::default()
            .with_title("3D Review")
            .with_window_icon(load_window_icon())
            .with_min_inner_size(winit::dpi::LogicalSize::new(960.0, 640.0))
            .with_maximized(maximized);
        if let Some(placement) = saved {
            // Position/size are the restored (non-maximized) bounds; setting them
            // even when maximized gives un-maximize a sensible target.
            attributes = attributes
                .with_position(winit::dpi::PhysicalPosition::new(placement.x, placement.y))
                .with_inner_size(winit::dpi::PhysicalSize::new(
                    placement.width,
                    placement.height,
                ));
            self.last_windowed_bounds = Some((
                (placement.x, placement.y),
                (placement.width, placement.height),
            ));
        }

        let window = event_loop
            .create_window(attributes)
            .expect("failed to create application window");
        let window = Arc::new(window);

        // Paint the client area black immediately, before the (non-instant) wgpu
        // surface setup below, so the window never flashes its default white while
        // the renderer comes up. See `startup_paint` (the one sanctioned exception
        // to invariant 9).
        startup_paint::paint_window_black(&window);

        let renderer_config = RendererConfig::default();
        let mut renderer = Renderer::new(renderer_config);
        let size = window.inner_size();
        if size.height > 0 {
            renderer.set_camera_aspect_ratio(size.width as f32 / size.height as f32);
            renderer.set_uv_aspect_ratio(size.width as f32 / size.height as f32);
            let (safe_w, safe_h) = framing_safe_area(size.height, window.scale_factor() as f32);
            renderer.set_framing_safe_area(safe_w, safe_h);
            // Re-frame the home view for the real window size / safe area so the
            // startup view matches what reset (`animate_camera_to_home`)
            // produces, instead of the full-window `OrbitCamera::default`.
            renderer.reset_camera_to_home();
        }
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());
        install_fonts(&egui_ctx);

        let mut egui_painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            wgpu_configuration(renderer_config),
            EGUI_MSAA_SAMPLE_COUNT,
            Some(EGUI_DEPTH_FORMAT),
            false,
            true,
        ));
        pollster::block_on(egui_painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .expect("failed to initialize wgpu surface");

        // Report which backend/adapter wgpu actually selected (see
        // RendererConfig::preferred_backends — DX12 on Windows). Logged via
        // `tracing`; set RUST_LOG=info to see it.
        if let Some(render_state) = egui_painter.render_state() {
            let adapter_info = render_state.adapter.get_info();
            info!(
                backend = ?adapter_info.backend,
                adapter = %adapter_info.name,
                device_type = ?adapter_info.device_type,
                "selected wgpu adapter"
            );
            // Surface the chosen backend in the startup help overlay.
            self.ui.gpu_backend = friendly_backend_name(adapter_info.backend);
            // Gate the Anti Aliasing menu to the MSAA levels this adapter can
            // actually render the scene at (invariant 4).
            self.ui.supported_msaa = supported_msaa_levels(&render_state.adapter);
            // Gate the Environment IBL toggle on the adapter being able to build
            // the HDR maps (invariant 4).
            self.ui.ibl_supported = ibl_supported(&render_state.adapter);
            // Gate the SSAO toggle on the adapter being able to run it (invariant 4).
            self.ui.ssao_supported = ssao_supported(&render_state.adapter);
        }

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            egui_painter.max_texture_side(),
        );

        self.renderer = Some(renderer);
        self.egui_ctx = Some(egui_ctx);
        self.egui_state = Some(egui_state);
        self.egui_painter = Some(egui_painter);
        self.ui.stats = self.scene_model.stats;
        self.ui.bounds = self.scene_model.bounds;
        self.ui.uv_sets = self.scene_model.uv_set_labels();
        self.refresh_interval = monitor_refresh_interval(&window);
        self.window = Some(window.clone());
        info!("application shell started");

        // Load a file passed on the command line (file association / CLI arg)
        // now that the renderer exists. Reuses the same path as drag-drop, so
        // framing/stats/redraw behave identically.
        if let Some(path) = self.initial_model.take() {
            self.open_model_from_path(&path);
        }

        // Begin the launch fade-in from here: the renderer/chrome are ready, so
        // the first frame starts fully covered by the startup black and dissolves
        // to the live viewer over `STARTUP_FADE`. Starting it now (rather than at
        // window creation) ties the fade to the moment there's actually something
        // to reveal, so a slow surface setup doesn't eat into it.
        self.fade = Some(FadeProgress {
            elapsed: Duration::ZERO,
            last_tick: Instant::now(),
        });

        // Paint the first frame directly rather than waiting on the first
        // `RedrawRequested`, so the window shows the rendered scene as soon as
        // it appears instead of an unpainted surface.
        self.render();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Clone the `Arc` so `self` stays free to mutate inside the match arms
        // (e.g. recording window bounds), rather than holding an immutable borrow
        // of `self.window` for the whole handler.
        let Some(window) = self.window.clone() else {
            return;
        };

        if window.id() != window_id {
            return;
        }

        let egui_response = self
            .egui_state
            .as_mut()
            .map(|egui_state| egui_state.on_window_event(&window, &event));

        // egui reports `repaint` for `RedrawRequested` itself; honoring that here
        // would make every frame schedule the next one, spinning the loop
        // uncapped. Only let *other* events (input, resize, …) request a repaint,
        // and route it through the paced flag so mouse-over/hover repaints are
        // capped to the monitor refresh rather than redrawn immediately.
        if egui_response
            .as_ref()
            .is_some_and(|response| response.repaint)
            && !matches!(event, WindowEvent::RedrawRequested)
        {
            self.redraw_requested = true;
        }

        // egui claims keyboard events while a widget has focus (e.g. typing in a
        // panel's numeric field); don't let those double as viewer shortcuts.
        let egui_consumed = egui_response.is_some_and(|response| response.consumed);

        match event {
            WindowEvent::CloseRequested => {
                self.save_window_placement();
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                self.render();
            }
            WindowEvent::Moved(_) => {
                self.record_windowed_bounds();
            }
            WindowEvent::Resized(size) => {
                // A resize often accompanies a move to another monitor, which may
                // have a different refresh rate; re-derive the frame cap.
                self.refresh_interval = monitor_refresh_interval(&window);
                self.record_windowed_bounds();

                if let Some(renderer) = self.renderer.as_mut() {
                    if size.height > 0 {
                        let aspect = size.width as f32 / size.height as f32;
                        renderer.set_camera_aspect_ratio(aspect);
                        renderer.set_uv_aspect_ratio(aspect);
                        let (safe_w, safe_h) =
                            framing_safe_area(size.height, window.scale_factor() as f32);
                        renderer.set_framing_safe_area(safe_w, safe_h);
                    }
                }

                if let (Some(width), Some(height), Some(painter)) = (
                    NonZeroU32::new(size.width),
                    NonZeroU32::new(size.height),
                    self.egui_painter.as_mut(),
                ) {
                    painter.on_window_resized(egui::ViewportId::ROOT, width, height);
                }

                window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released {
                    self.drag_mode = None;
                } else if button == MouseButton::Left && self.ui.show_help_overlay {
                    // The startup help overlay is up: it swallows the click in
                    // egui (so the chrome beneath stays inert), but we still drive
                    // dismissal here. A plain click hides it; a double-click also
                    // opens the file picker — the same gesture as on the empty
                    // viewport, so it reuses the same double-click detection. The
                    // first click seeds `last_primary_click`; the second arrives
                    // after the overlay is gone and opens the dialog via the
                    // branch below.
                    if self.should_open_on_double_click() {
                        self.open_model_from_dialog();
                    } else if let Some(position) = self.last_pointer_position {
                        self.last_primary_click = Some((Instant::now(), position));
                    }
                    self.ui.show_help_overlay = false;
                    self.redraw_requested = true;
                } else if !egui_response.is_some_and(|response| response.consumed) {
                    // The UV viewport is a 2D pan/zoom workspace: LMB pans, RMB
                    // zooms (down = in). The 3D scene keeps LMB orbit / RMB
                    // pan-or-zoom.
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
                            // UV mode: RMB zoom-drags. 3D: Alt+RMB zoom-drags
                            // (down = in, up = out), plain RMB pans.
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
            }
            WindowEvent::CursorMoved { position, .. } => {
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
                        // Pan drives the 2D UV camera in UV mode, the 3D camera
                        // otherwise.
                        DragMode::Pan => {
                            if uv_mode {
                                renderer.pan_uv_camera(delta, viewport);
                            } else {
                                renderer.pan_camera(delta, viewport);
                            }
                        }
                        // Pointer down (positive screen delta) zooms in, up
                        // zooms out — matching the wheel's positive-is-in sign.
                        DragMode::Zoom => {
                            if uv_mode {
                                renderer.zoom_uv_camera(delta.y * 0.01);
                            } else {
                                renderer.zoom_camera(delta.y * 0.01);
                            }
                        }
                    }
                    self.redraw_requested = true;
                }

                self.last_pointer_position = Some(current);
            }
            WindowEvent::CursorLeft { .. } => {
                self.drag_mode = None;
                self.last_pointer_position = None;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !egui_response.is_some_and(|response| response.consumed) {
                    if let Some(renderer) = self.renderer.as_mut() {
                        let amount = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y * 0.5,
                            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 120.0,
                        };
                        if self.ui.mode == WorkspaceMode::Uv {
                            renderer.zoom_uv_camera(amount);
                        } else {
                            renderer.zoom_camera(amount);
                        }
                        self.redraw_requested = true;
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } if !egui_consumed => {
                // Any key press dismisses the startup help overlay (and still
                // performs its shortcut). Request a redraw so it clears even for
                // keys that aren't bound to a shortcut.
                if event.state == ElementState::Pressed && self.ui.show_help_overlay {
                    self.ui.show_help_overlay = false;
                    self.redraw_requested = true;
                }
                self.handle_keyboard_shortcut(&event);
            }
            WindowEvent::DroppedFile(path) => {
                self.open_model_from_path(&path);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // Fold a pending interactive redraw (drag, hover, wheel, key) into the
        // paced schedule. The earliest we'll draw is one refresh interval after
        // the last frame, so a burst of high-frequency input events coalesces
        // into a single redraw capped at the monitor refresh rate.
        if self.redraw_requested {
            self.redraw_requested = false;
            let earliest = self
                .last_render_instant
                .map_or(now, |last| last + self.refresh_interval);
            self.repaint_at = Some(self.repaint_at.map_or(earliest, |at| at.min(earliest)));
        }

        // Sleep until the next scheduled repaint (if any), otherwise block until
        // the next input event. When the scheduled time arrives, fire one redraw
        // and fall back to waiting.
        match self.repaint_at {
            Some(wake) if now >= wake => {
                self.repaint_at = None;
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(wake) => event_loop.set_control_flow(ControlFlow::WaitUntil(wake)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl App {
    fn render(&mut self) {
        let Some(window) = self.window.as_ref().cloned() else {
            return;
        };
        let Some(egui_ctx) = self.egui_ctx.as_ref().cloned() else {
            return;
        };

        window.set_title("3D Review");
        self.update_camera_animation();

        // Opacity of the launch fade cover this frame (1 = startup black, 0 =
        // fully revealed). Advancing here clears `fade_start` once it reaches 0,
        // so the steady state draws no cover; while it's > 0 the pacing logic
        // below keeps requesting frames so the dissolve animates.
        let cover_opacity = self.advance_startup_fade();

        let output_format = {
            let Some(egui_painter) = self.egui_painter.as_ref() else {
                return;
            };

            let Some(output_format) = egui_painter
                .render_state()
                .map(|render_state| render_state.target_format)
            else {
                return;
            };

            output_format
        };

        // The bounding-box dimension labels occlude against the mesh through a
        // triangle BVH. Build it lazily the first frame the labels are shown for a
        // given model (and rebuild after a new model loads); reused across frames,
        // so orbiting pays no per-frame triangle cost.
        if self.ui.debug.show_bounding_box && self.occlusion_bvh_revision != self.scene_revision {
            self.occlusion_bvh = Some(Bvh::build(&self.scene_model));
            self.occlusion_bvh_revision = self.scene_revision;
        }

        let (full_output, clear, ui_output) = {
            let Some(egui_state) = self.egui_state.as_mut() else {
                return;
            };
            let Some(renderer) = self.renderer.as_ref() else {
                return;
            };

            let raw_input = egui_state.take_egui_input(&window);
            let camera = renderer.camera;
            let uv_camera = renderer.uv_camera;
            let clear = renderer.config.clear_color;
            let scene_model = self.scene_model.clone();
            let scene_revision = self.scene_revision;
            let occlusion_bvh = self.occlusion_bvh.as_ref();
            let mut ui_output = UiOutput::default();
            let full_output = egui_ctx.run(raw_input, |ctx| {
                draw_viewport_scene(
                    ctx,
                    &self.ui,
                    camera,
                    uv_camera,
                    scene_model.clone(),
                    scene_revision,
                    output_format,
                );
                ui_output = draw_overlay(ctx, &mut self.ui, camera, &scene_model, occlusion_bvh);
                // Above all chrome: the launch fade cover (no-op once revealed).
                draw_startup_fade(ctx, cover_opacity);
            });

            egui_state.handle_platform_output(&window, full_output.platform_output.clone());
            (full_output, clear, ui_output)
        };

        self.apply_ui_output(ui_output);

        // Decide when the next frame should be drawn. Continuous motion — a live
        // camera transition, or egui asking to "repaint immediately" (zero delay)
        // — is paced to the monitor's refresh interval so the viewer never renders
        // faster than the display can show it. (We can't rely on the swapchain to
        // pace us: on the Vulkan path `present` does not block on vblank.) A finite
        // egui delay (e.g. a tooltip timer) schedules a single future wake-up, and
        // an infinite delay means everything is idle, so we wait for the next event.
        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |output| output.repaint_delay);
        let camera_animating = self
            .renderer
            .as_ref()
            .is_some_and(Renderer::is_camera_animating);
        // The launch fade is continuous motion too: keep pacing frames while the
        // cover is still dissolving so it doesn't stall on a static partial fade.
        let fade_active = cover_opacity > 0.0;
        self.repaint_at = if repaint_delay.is_zero() || camera_animating || fade_active {
            let frame_start = self.last_render_instant.unwrap_or_else(Instant::now);
            Some(frame_start + self.refresh_interval)
        } else if repaint_delay == Duration::MAX {
            None
        } else {
            Instant::now().checked_add(repaint_delay)
        };

        let pixels_per_point = full_output.pixels_per_point;
        let clipped_primitives = egui_ctx.tessellate(full_output.shapes, pixels_per_point);

        let Some(egui_painter) = self.egui_painter.as_mut() else {
            return;
        };

        egui_painter.paint_and_update_textures(
            egui::ViewportId::ROOT,
            pixels_per_point,
            [
                clear.r as f32,
                clear.g as f32,
                clear.b as f32,
                clear.a as f32,
            ],
            &clipped_primitives,
            &full_output.textures_delta,
            Vec::new(),
        );
    }

    fn open_model_from_dialog(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("FBX", &["fbx"])
            .set_title("Open Model")
            .pick_file();

        let Some(path) = file else {
            return;
        };

        self.open_model_from_path(&path);
    }

    fn open_model_from_path(&mut self, path: &Path) {
        // Loading a model (drag-drop, file dialog, or CLI arg) dismisses the
        // startup help overlay if it's still up.
        self.ui.show_help_overlay = false;

        match load_model(path, LoadOptions { triangulate: true }) {
            Ok(model) => {
                let model = Arc::new(model);

                if let Some(renderer) = self.renderer.as_mut() {
                    frame_camera_to_model(renderer, &model);
                    renderer.reset_uv_camera();
                }

                self.ui.stats = model.stats;
                self.ui.bounds = model.bounds;
                // A new model invalidates the previously selected UV channel;
                // reset to channel 0 so the picker never points past the new
                // model's UV-set count.
                self.ui.uv_checker.uv_channel = 0;
                // Refresh the UV-view dropdown labels and reset its independent
                // channel for the new model.
                self.ui.uv_sets = model.uv_set_labels();
                self.ui.uv_view_channel = 0;
                self.scene_model = model;
                self.scene_revision = self.scene_revision.saturating_add(1);
                info!(path = %path.display(), "model loaded");
            }
            Err(error) => {
                warn!(path = %path.display(), error = %error, "model load failed");
            }
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Dispatch a viewport keyboard shortcut on key-down. Ctrl-modified keys are
    /// file commands; the bare keys are view / camera shortcuts and only fire
    /// when no modifier is held (so Shift/Alt/Ctrl combinations stay free).
    /// Keyboard events egui has already consumed are filtered out by the caller.
    fn handle_keyboard_shortcut(&mut self, event: &KeyEvent) {
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
            }
            return;
        }

        if !self.modifiers.is_empty() {
            return;
        }

        // In the UV workspace the 3D camera shortcuts (WASD orbit / shading /
        // grid) don't apply; only F / R, which reframe the 2D UV view.
        if self.ui.mode == WorkspaceMode::Uv {
            if event.state == ElementState::Pressed && matches!(character.as_str(), "f" | "r") {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.reset_uv_camera();
                }
                self.redraw_requested = true;
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
            match character.as_str() {
                "a" => self.orbit_camera_step(ORBIT_STEP, 0.0),
                "d" => self.orbit_camera_step(-ORBIT_STEP, 0.0),
                "w" => self.orbit_camera_step(0.0, -ORBIT_STEP),
                "s" => self.orbit_camera_step(0.0, ORBIT_STEP),
                _ => return,
            }
            self.redraw_requested = true;
            return;
        }

        // Remaining shortcuts are instant toggles/commands on key-down.
        match character.as_str() {
            "`" => self.ui.debug.wireframe_overlay = !self.ui.debug.wireframe_overlay,
            "1" => self.ui.shading_mode = ShadingMode::Wireframe,
            "2" => self.ui.shading_mode = ShadingMode::Unlit,
            "3" => self.ui.shading_mode = ShadingMode::Shaded,
            "i" => self.ui.show_stats = !self.ui.show_stats,
            "g" => self.ui.show_grid = !self.ui.show_grid,
            "f" => {
                if let Some(renderer) = self.renderer.as_mut() {
                    frame_camera_to_model(renderer, &self.scene_model);
                }
            }
            "r" => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.animate_camera_to_home();
                }
            }
            _ => return,
        }

        self.redraw_requested = true;
    }

    /// Animate a relative 45° camera orbit (radians) for the WASD shortcuts.
    fn orbit_camera_step(&mut self, yaw_delta: f32, pitch_delta: f32) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_orbit_by(yaw_delta, pitch_delta);
        }
    }

    /// Return the viewer to its launch state (Ctrl+N): drop the loaded model so
    /// the empty viewport is shown again, reset the dependent UI, and animate the
    /// camera back to its home framing. Bumping the scene revision drops the
    /// previously-uploaded GPU geometry on the next paint.
    fn reset_to_start_state(&mut self) {
        let empty = Arc::new(ModelData::default());
        self.ui.stats = empty.stats;
        self.ui.bounds = empty.bounds;
        self.ui.uv_checker.uv_channel = 0;
        self.ui.uv_sets = empty.uv_set_labels();
        self.ui.uv_view_channel = 0;
        self.scene_model = empty;
        self.scene_revision = self.scene_revision.saturating_add(1);

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_camera_to_home();
            renderer.reset_uv_camera();
        }

        info!("reset to start state");
        self.redraw_requested = true;
    }

    fn should_open_on_double_click(&self) -> bool {
        if !self.scene_model.vertices.is_empty() {
            return false;
        }

        let Some((last_click_time, last_click_position)) = self.last_primary_click else {
            return false;
        };
        let Some(current_position) = self.last_pointer_position else {
            return false;
        };

        last_click_time.elapsed() <= Duration::from_millis(450)
            && current_position.distance(last_click_position) <= 6.0
    }

    fn apply_ui_output(&mut self, output: UiOutput) {
        let Some(action) = output.axis_gizmo_action else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        match action {
            AxisGizmoAction::Orbit(delta) => renderer.orbit_camera(delta),
            AxisGizmoAction::Snap(axis) => {
                renderer.animate_camera_to_offset_direction(axis.offset_direction());
            }
            AxisGizmoAction::ResetView => renderer.animate_camera_to_home(),
        }

        self.redraw_requested = true;
    }

    /// Snapshot the window's current outer position + inner size while it isn't
    /// maximized, so the persisted placement describes a real window. Skipped
    /// while maximized (the bounds then cover the whole monitor) and when winit
    /// can't report the outer position.
    fn record_windowed_bounds(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.is_maximized() {
            return;
        }
        let Ok(position) = window.outer_position() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.last_windowed_bounds = Some(((position.x, position.y), (size.width, size.height)));
    }

    /// Persist the current window placement to `%APPDATA%` on exit. Uses the last
    /// recorded non-maximized bounds (so un-maximize restores correctly) together
    /// with the live maximized state.
    fn save_window_placement(&mut self) {
        // Refresh from the live window first in case the latest move/resize hasn't
        // been recorded yet.
        self.record_windowed_bounds();

        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(((x, y), (width, height))) = self.last_windowed_bounds else {
            return;
        };

        window_state::save(WindowPlacement {
            x,
            y,
            width,
            height,
            maximized: window.is_maximized(),
        });
    }

    /// Advance the launch fade-in and return this frame's cover opacity (1 =
    /// startup black, 0 = fully revealed). Returns 0 and clears `fade` once the
    /// animation is done, so it's a cheap no-op every frame thereafter. The curve
    /// is an inverted smoothstep: the reveal starts gently from full black,
    /// accelerates, then eases out as the viewer settles in.
    ///
    /// Time is accumulated from a *capped* per-frame delta rather than absolute
    /// wall-clock: a slow first frame (pipeline warm-up, surface acquire, the OS
    /// window-open animation) advances the fade by one frame's worth instead of
    /// the whole stall, so the dissolve plays across real presented frames
    /// instead of being skipped — which read as a snap.
    fn advance_startup_fade(&mut self) -> f32 {
        let Some(fade) = self.fade.as_mut() else {
            return 0.0;
        };
        let now = Instant::now();
        let step = now.duration_since(fade.last_tick).min(MAX_FADE_STEP);
        fade.last_tick = now;
        fade.elapsed += step;

        if fade.elapsed >= STARTUP_FADE {
            self.fade = None;
            return 0.0;
        }
        let t = (fade.elapsed.as_secs_f32() / STARTUP_FADE.as_secs_f32()).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }

    fn update_camera_animation(&mut self) {
        let now = Instant::now();
        let delta_seconds = self
            .last_render_instant
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last_render_instant = Some(now);

        // The viewer redraws on demand, so FPS is only meaningful across
        // consecutive frames (camera animation / interaction). Ignore the long
        // gaps after an idle period and exponentially smooth the live rate.
        if (0.0..0.25).contains(&delta_seconds) && delta_seconds > 0.0 {
            let instant_fps = 1.0 / delta_seconds;
            self.ui.fps = if self.ui.fps > 0.0 {
                self.ui.fps * 0.9 + instant_fps * 0.1
            } else {
                instant_fps
            };
        }

        // Advance any live camera transition. The follow-up redraw is scheduled
        // by the paced `repaint_at` logic in `render` (which checks
        // `is_camera_animating`), so we don't request one directly here — doing so
        // would bypass the refresh-rate cap.
        //
        // Cap the step: the viewer redraws on demand, so after an idle period
        // `last_render_instant` is stale and the first frame's delta is the whole
        // idle gap. Advancing a transition by that would fast-forward it to the
        // end in one frame (skipping the animation entirely) — most visible on the
        // short 0.1 s WASD orbits, where almost any delta exceeds the duration.
        // One ~30 Hz frame is plenty to keep motion smooth.
        const MAX_ANIMATION_STEP: f32 = 1.0 / 30.0;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_camera_animation(delta_seconds.min(MAX_ANIMATION_STEP));
        }
    }
}

/// Whether a saved placement still lands on a connected monitor. Guards against
/// restoring a window onto a display that has since been unplugged or had its
/// layout changed, which would otherwise reopen the window off-screen. A
/// placement counts as visible if its rectangle overlaps any available monitor;
/// when winit reports no monitors we trust the placement rather than discard it.
fn placement_is_visible(event_loop: &ActiveEventLoop, placement: &WindowPlacement) -> bool {
    let win_left = placement.x;
    let win_top = placement.y;
    let win_right = placement.x + placement.width as i32;
    let win_bottom = placement.y + placement.height as i32;

    let mut any_monitor = false;
    for monitor in event_loop.available_monitors() {
        any_monitor = true;
        let pos = monitor.position();
        let size = monitor.size();
        let mon_left = pos.x;
        let mon_top = pos.y;
        let mon_right = pos.x + size.width as i32;
        let mon_bottom = pos.y + size.height as i32;

        let overlaps = win_left < mon_right
            && win_right > mon_left
            && win_top < mon_bottom
            && win_bottom > mon_top;
        if overlaps {
            return true;
        }
    }

    // No monitors enumerated (rare/headless): don't throw the placement away.
    !any_monitor
}

/// The active monitor's refresh interval, used to cap continuous redraw. Falls
/// back to 60 Hz when winit can't report a rate (some virtual/headless displays).
fn monitor_refresh_interval(window: &Window) -> Duration {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .filter(|millihertz| *millihertz > 0)
        .map(|millihertz| Duration::from_secs_f64(1000.0 / f64::from(millihertz)))
        .unwrap_or_else(|| Duration::from_secs_f64(1.0 / 60.0))
}

/// Fraction of the window framing should fill, leaving room for the chrome that
/// overlays the full-window 3D scene (toolbar on top, status bar on the bottom)
/// so a framed model doesn't hide under it. Width is left unconstrained — the
/// option panel floats and is transient.
fn framing_safe_area(height_px: u32, scale_factor: f32) -> (f32, f32) {
    use review_ui::theme::size::{STATUS_BAR_HEIGHT, TOOLBAR_HEIGHT};
    let logical_height = height_px as f32 / scale_factor.max(0.1);
    let chrome = TOOLBAR_HEIGHT + STATUS_BAR_HEIGHT;
    let height_fraction = if logical_height > chrome {
        (logical_height - chrome) / logical_height
    } else {
        1.0
    };
    (1.0, height_fraction.clamp(0.4, 1.0))
}

fn frame_camera_to_model(renderer: &mut Renderer, model: &ModelData) {
    if let Some(bounds) = model.bounds {
        renderer.animate_camera_to_bounds(bounds);
    }
}

/// Map the wgpu backend wgpu actually selected to a short label for the help
/// overlay title (e.g. `Backend::Dx12` → "DX12"). Falls back to the enum's debug
/// name for any backend without a custom label.
fn friendly_backend_name(backend: wgpu::Backend) -> String {
    match backend {
        wgpu::Backend::Dx12 => "DX12".to_string(),
        wgpu::Backend::Vulkan => "Vulkan".to_string(),
        wgpu::Backend::Metal => "Metal".to_string(),
        wgpu::Backend::Gl => "OpenGL".to_string(),
        wgpu::Backend::BrowserWebGpu => "WebGPU".to_string(),
        other => format!("{other:?}"),
    }
}

fn wgpu_configuration(renderer_config: RendererConfig) -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = renderer_config.preferred_backends;

    // Mirror egui_wgpu's default device descriptor, but additionally request
    // `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` when the adapter offers it. The
    // WebGPU spec only guarantees sample counts [1, 4] for our HDR/depth render
    // formats; the intermediate/high counts the adapter reports (2× / 8× here)
    // are only usable on the device once that feature is enabled. Without it,
    // building a scene pipeline at e.g. 2× MSAA fails validation. The feature is
    // masked against the adapter's own features so we never request something it
    // lacks (invariant 4); `supported_msaa_levels` mirrors this gating so the UI
    // only offers what the device will actually accept.
    setup.device_descriptor = std::sync::Arc::new(|adapter: &wgpu::Adapter| {
        let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };
        wgpu::DeviceDescriptor {
            label: Some("egui wgpu device"),
            required_features: adapter.features()
                & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            required_limits: wgpu::Limits {
                // Match egui's default: large enough for 4k+ surfaces with a depth
                // buffer.
                max_texture_dimension_2d: 8192,
                ..base_limits
            },
            memory_hints: wgpu::MemoryHints::default(),
        }
    });

    egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        // Vsync: present in FIFO so the swapchain paces frames to the monitor's
        // refresh and the viewer never renders faster than the display.
        present_mode: wgpu::PresentMode::AutoVsync,
        ..Default::default()
    }
}
