use anyhow::Context;
use review_render::{Renderer, RendererConfig};
use review_ui::UiState;
use tracing::info;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
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
    window: Option<Window>,
    renderer: Option<Renderer>,
    ui: UiState,
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

        self.renderer = Some(Renderer::new(RendererConfig::default()));
        self.ui.status = "Empty viewport".to_owned();
        self.window = Some(window);
        info!("application shell started");
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

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                window.set_title(&format!("3D Review - {}", self.ui.status));
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if size.height > 0 {
                        renderer.camera.aspect_ratio = size.width as f32 / size.height as f32;
                    }
                }
                window.request_redraw();
            }
            _ => {}
        }
    }
}
