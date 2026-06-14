use std::{num::NonZeroU32, sync::Arc};

use anyhow::Context;
use glam::Vec2;
use review_render::{Renderer, RendererConfig, SCENE_DEPTH_FORMAT};
use review_ui::{draw_overlay, draw_viewport_scene, UiState};
use tracing::info;
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

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: Option<egui::Context>,
    egui_state: Option<egui_winit::State>,
    egui_painter: Option<egui_wgpu::winit::Painter>,
    drag_mode: Option<DragMode>,
    last_pointer_position: Option<Vec2>,
    ui: UiState,
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
        }

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
            renderer.camera.aspect_ratio = size.width as f32 / size.height as f32;
        }

        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());

        let mut egui_painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            wgpu_configuration(renderer_config),
            1,
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
        self.ui.status = "LMB orbit  RMB pan  Wheel zoom  F reset".to_owned();
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
                        renderer.camera.aspect_ratio = size.width as f32 / size.height as f32;
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
                    self.drag_mode = match button {
                        MouseButton::Left => Some(DragMode::Orbit),
                        MouseButton::Right => Some(DragMode::Pan),
                        _ => self.drag_mode,
                    };
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
                        DragMode::Orbit => renderer.camera.orbit(delta),
                        DragMode::Pan => {
                            let size = window.inner_size();
                            renderer.camera.pan_screen_delta(
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
                        renderer.camera.zoom(amount);
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
                    renderer.camera.reset();
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

impl App {
    fn render(&mut self) {
        let (Some(window), Some(renderer), Some(egui_ctx), Some(egui_state), Some(egui_painter)) = (
            self.window.as_ref(),
            self.renderer.as_ref(),
            self.egui_ctx.as_ref(),
            self.egui_state.as_mut(),
            self.egui_painter.as_mut(),
        ) else {
            return;
        };

        window.set_title(&format!("3D Review - {}", self.ui.status));

        let Some(output_format) = egui_painter
            .render_state()
            .map(|render_state| render_state.target_format)
        else {
            return;
        };

        let raw_input = egui_state.take_egui_input(window);
        let camera = renderer.camera;
        let full_output = egui_ctx.run(raw_input, |ctx| {
            draw_viewport_scene(ctx, &self.ui, camera, output_format);
            draw_overlay(ctx, &mut self.ui);
        });

        egui_state.handle_platform_output(window, full_output.platform_output);

        let pixels_per_point = full_output.pixels_per_point;
        let clipped_primitives = egui_ctx.tessellate(full_output.shapes, pixels_per_point);
        let clear = renderer.config.clear_color;

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
}

fn wgpu_configuration(renderer_config: RendererConfig) -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = renderer_config.preferred_backends;

    egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}
