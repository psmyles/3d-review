use std::{
    num::NonZeroU32,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use glam::Vec2;
use review_import::{load_model, LoadOptions};
use review_model::ModelData;
use review_render::{Renderer, RendererConfig, SCENE_DEPTH_FORMAT, SCENE_SAMPLE_COUNT};
use review_ui::{draw_overlay, draw_viewport_scene, AxisGizmoAction, UiOutput, UiState};
use tracing::{info, warn};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::Key,
    window::{Window, WindowAttributes, WindowId},
};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let event_loop = EventLoop::new().context("failed to create winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
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
    last_render_instant: Option<Instant>,
    scene_model: Arc<ModelData>,
    scene_revision: u64,
    ui: UiState,
}

impl Default for App {
    fn default() -> Self {
        let scene_model = Arc::new(ModelData::default());
        let ui = UiState {
            stats: scene_model.stats,
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
            last_render_instant: None,
            scene_model,
            scene_revision: 0,
            ui,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Orbit,
    Pan,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        };

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("3D Review")
                    .with_min_inner_size(winit::dpi::LogicalSize::new(960.0, 640.0)),
            )
            .expect("failed to create application window");
        let window = Arc::new(window);

        let renderer_config = RendererConfig::default();
        let mut renderer = Renderer::new(renderer_config);
        let size = window.inner_size();
        if size.height > 0 {
            renderer.set_camera_aspect_ratio(size.width as f32 / size.height as f32);
        }
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());

        let mut egui_painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            wgpu_configuration(renderer_config),
            SCENE_SAMPLE_COUNT,
            Some(SCENE_DEPTH_FORMAT),
            false,
            true,
        ));
        pollster::block_on(egui_painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .expect("failed to initialize wgpu surface");

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
        self.ui.status = "LMB orbit  RMB pan  Wheel zoom  F frame".to_owned();
        self.ui.stats = self.scene_model.stats;
        self.window = Some(window.clone());
        info!("application shell started");
        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };

        if window.id() != window_id {
            return;
        }

        let egui_response = self.egui_state.as_mut().map(|egui_state| {
            let response = egui_state.on_window_event(window, &event);
            if response.repaint {
                window.request_redraw();
            }
            response
        });

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                self.render();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if size.height > 0 {
                        renderer.set_camera_aspect_ratio(size.width as f32 / size.height as f32);
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
                } else if !egui_response.is_some_and(|response| response.consumed) {
                    match button {
                        MouseButton::Left => {
                            if self.should_open_on_double_click() {
                                self.open_model_from_dialog();
                                self.drag_mode = None;
                            } else {
                                self.drag_mode = Some(DragMode::Orbit);
                                if let Some(position) = self.last_pointer_position {
                                    self.last_primary_click = Some((Instant::now(), position));
                                }
                            }
                        }
                        MouseButton::Right => {
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
                    match mode {
                        DragMode::Orbit => renderer.orbit_camera(delta),
                        DragMode::Pan => {
                            let size = window.inner_size();
                            renderer.pan_camera(
                                delta,
                                Vec2::new(size.width as f32, size.height as f32),
                            );
                        }
                    }
                    window.request_redraw();
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
                        renderer.zoom_camera(amount);
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && matches!(
                        &event.logical_key,
                        Key::Character(character) if character.eq_ignore_ascii_case("f")
                    ) =>
            {
                if let Some(renderer) = self.renderer.as_mut() {
                    frame_camera_to_model(renderer, &self.scene_model);
                    window.request_redraw();
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.open_model_from_path(&path);
            }
            _ => {}
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

        window.set_title(&format!("3D Review - {}", self.ui.status));
        self.update_camera_animation(&window);

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

        let (full_output, clear, ui_output) = {
            let Some(egui_state) = self.egui_state.as_mut() else {
                return;
            };
            let Some(renderer) = self.renderer.as_ref() else {
                return;
            };

            let raw_input = egui_state.take_egui_input(&window);
            let camera = renderer.camera;
            let clear = renderer.config.clear_color;
            let scene_model = self.scene_model.clone();
            let scene_revision = self.scene_revision;
            let mut ui_output = UiOutput::default();
            let full_output = egui_ctx.run(raw_input, |ctx| {
                draw_viewport_scene(
                    ctx,
                    &self.ui,
                    camera,
                    scene_model.clone(),
                    scene_revision,
                    output_format,
                );
                ui_output = draw_overlay(ctx, &mut self.ui, camera);
            });

            egui_state.handle_platform_output(&window, full_output.platform_output.clone());
            (full_output, clear, ui_output)
        };

        self.apply_ui_output(ui_output);

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
            self.ui.status = "Open canceled".to_owned();
            return;
        };

        self.open_model_from_path(&path);
    }

    fn open_model_from_path(&mut self, path: &Path) {
        match load_model(path, LoadOptions { triangulate: true }) {
            Ok(model) => {
                let model = Arc::new(model);
                let display_name = if model.name.is_empty() {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Model")
                        .to_owned()
                } else {
                    model.name.clone()
                };

                if let Some(renderer) = self.renderer.as_mut() {
                    frame_camera_to_model(renderer, &model);
                }

                self.ui.stats = model.stats;
                self.ui.status = format!("Loaded {display_name}");
                self.scene_model = model;
                self.scene_revision = self.scene_revision.saturating_add(1);
                info!(path = %path.display(), "model loaded");
            }
            Err(error) => {
                self.ui.status = format!("Open failed: {error}");
                warn!(path = %path.display(), error = %error, "model load failed");
            }
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
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
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn update_camera_animation(&mut self, window: &Window) {
        let now = Instant::now();
        let delta_seconds = self
            .last_render_instant
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last_render_instant = Some(now);

        if let Some(renderer) = self.renderer.as_mut() {
            if renderer.update_camera_animation(delta_seconds) && renderer.is_camera_animating() {
                window.request_redraw();
            }
        }
    }
}

fn frame_camera_to_model(renderer: &mut Renderer, model: &ModelData) {
    if let Some(bounds) = model.bounds {
        renderer.animate_camera_to_bounds(bounds);
    }
}

fn wgpu_configuration(renderer_config: RendererConfig) -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = renderer_config.preferred_backends;

    egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}
