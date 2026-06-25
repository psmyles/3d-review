// Suppress the console window in release builds — a shipped GUI viewer should
// open as a window, not alongside a terminal. Debug builds keep the console so
// `tracing` output stays visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod startup_paint;
mod startup_timing;
mod texture_manager;
mod window_state;

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use glam::Vec2;
use notify::RecommendedWatcher;
use review_import::{LoadOptions, load_model};
use review_model::{ModelData, SceneBvh};
use review_render::{
    DecodedImage, EGUI_DEPTH_FORMAT, EGUI_MSAA_SAMPLE_COUNT, Renderer, RendererConfig, ShadingMode,
    TextureSlot, gtao_supported, ibl_supported, selection_bounds, supported_msaa_levels,
};
use review_ui::{
    AxisGizmoAction, Notifications, Selection, TexViewRequest, TextureIntent, UiOutput, UiState,
    WorkspaceMode, draw_overlay, draw_viewport_scene, init_style,
};
use tracing::{info, warn};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};

/// Custom event posted from a background thread to the winit event loop, so work
/// done off the main thread is applied back on it (the redraw loop + all renderer
/// state stay in `app` — invariant 6).
#[derive(Debug, Clone)]
enum UserEvent {
    /// A watched directory reported a change to this path; if it's a bound texture,
    /// re-decode + re-upload it (posted by the file-watcher thread).
    TextureChanged(PathBuf),
    /// A background texture decode finished (posted by the decode thread). The
    /// result is uploaded + the slot/binding updated here on the main thread.
    TextureDecoded(TextureDecode),
}

use texture_manager::TextureDecode;
use window_state::WindowPlacement;

fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    // With `--features startup-trace`, egui-wgpu's + wgpu's `profiling::scope!`
    // setup scopes become INFO spans; logging their CLOSE events prints each
    // span's `time.busy`, breaking down the opaque `set_window` startup cost
    // (adapter enumerate / request_device / surface configure / egui renderer).
    #[cfg(feature = "startup-trace")]
    let subscriber = subscriber.with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE);
    subscriber.init();

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);

    // A file path passed on the command line (e.g. when Windows launches the exe
    // for a double-clicked `.fbx` via the registered file association) is loaded
    // once the window is up. See `resumed`.
    let initial_model = std::env::args_os().nth(1).map(PathBuf::from);

    // The texture file-watcher posts reload events back through this proxy.
    let texture_proxy = event_loop.create_proxy();

    let mut app = App {
        initial_model,
        texture_proxy: Some(texture_proxy),
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
    /// Remaining startup "warmup" frames to pump (Phase B). The first frame builds
    /// only the cheap core scene resources; the deferred scene pipelines + GTAO
    /// pass then compile one stage per subsequent frame (in the scene callback's
    /// `prepare`). While this is non-zero, `render` keeps scheduling the
    /// next frame so the build drains behind the already-shown grid, then stops.
    /// Seeded once in `resumed`; `app` can't see the render-side build state
    /// (invariant 2), so it pumps a fixed, generous count rather than polling.
    warmup_frames: u32,
    /// Minimum spacing between continuously-rendered frames, derived from the
    /// active monitor's refresh rate. Caps redraw to the display so animation
    /// doesn't render faster than it can be shown (the swapchain doesn't pace us
    /// on the Vulkan path). Defaults to 60 Hz until a monitor is known.
    refresh_interval: Duration,
    scene_model: Arc<ModelData>,
    scene_revision: u64,
    /// Per-mesh-part triangle BVH over [`Self::scene_model`], used to occlude the
    /// bounding-box dimension labels against the *visible* mesh. Built lazily the
    /// first frame the labels need it (the bounding-box view is on) and reused
    /// across frames; a heavy per-model structure we don't pay for unless the
    /// feature is used. `None` until built; [`Self::occlusion_bvh_revision`] tracks
    /// which model it covers so it rebuilds when a new model loads.
    occlusion_bvh: Option<SceneBvh>,
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
    /// Live selection-highlight flash, or `None` when none is playing. Started when
    /// [`Self::flashed_selection`] no longer matches the UI's current selection, and
    /// advanced each frame by [`Self::update_selection_flash`], which writes the
    /// fade into [`UiState::selection_fade`] for the scene callback.
    selection_flash: Option<FlashProgress>,
    /// The selection the flash is currently animating (or last animated), so a
    /// change to a *different* node/material restarts the flash and selecting
    /// nothing ends it.
    flashed_selection: Selection,
    /// Undo stack of past Outliner selections, newest last. Each committed change
    /// (from an Outliner click or `Esc` deselect) pushes the prior selection here;
    /// `Ctrl+Z` pops it back. Tracked in `app` (like the flash) because the redraw
    /// loop and keyboard routing live here. Capped at [`SELECTION_HISTORY_LIMIT`].
    selection_history: Vec<Selection>,
    /// The last selection recorded into [`Self::selection_history`], so a change is
    /// detected once per frame (an Outliner click mutates the selection during the
    /// egui run; the change is recorded on the next frame). Set to the restored
    /// value on undo so the undo itself isn't re-recorded as a new change.
    recorded_selection: Selection,
    /// Whether the camera is currently framed on the selection rather than the
    /// whole model, so `F` alternates between the two while a mesh part is
    /// selected. Reset whenever the selection changes (the next `F` frames the
    /// part first).
    frame_showing_selection: bool,
    /// Proxy used by the texture file-watcher thread to post reload events to the
    /// event loop (set in `main` before the loop runs).
    texture_proxy: Option<EventLoopProxy<UserEvent>>,
    /// The disk-auto-reload watcher, created lazily on the first texture
    /// assignment. Dropping it stops watching (done on model load / reset).
    texture_watcher: Option<RecommendedWatcher>,
    /// Directories the watcher is registered on (the parents of assigned textures),
    /// so each directory is watched at most once.
    watched_dirs: HashSet<PathBuf>,
    /// Decoded-image cache keyed by source path, so a packed map assigned to
    /// several slots / materials decodes once. Cleared on model load / reset.
    texture_cache: HashMap<PathBuf, Arc<DecodedImage>>,
    /// The scene-wide texture pool: imported source paths in insertion order. The
    /// decoded pixels live in [`Self::texture_cache`]; this is just the ordered set
    /// the Inspector's Texture files list + property dropdowns draw from (mirrored
    /// into [`UiState::texture_pool`] by [`Self::refresh_texture_pool`]). Cleared on
    /// model load / reset.
    texture_pool: Vec<PathBuf>,
    /// The toast notification system (egui-notify). `app` owns it because it owns
    /// the egui frame and triggers the notifications (texture decode start/finish);
    /// the UI crate only provides the themed type. Shown once per frame in `render`.
    notifications: Notifications,
}

/// State of the selection-highlight flash: a brief bright fill over a newly
/// selected node/material that fades out. Same capped-per-frame accumulation as
/// camera transitions, so an idle gap before the flash can't fast-forward it to
/// the end. `None` when no flash is playing.
struct FlashProgress {
    elapsed: Duration,
    last_tick: Instant,
}

/// How long the selection-highlight flash takes to fade from full to gone.
const SELECTION_FLASH: Duration = Duration::from_millis(500);

/// Cap on the selection undo stack, so a long session of selection changes can't
/// grow it unbounded. Older entries fall off the bottom once the cap is reached.
const SELECTION_HISTORY_LIMIT: usize = 128;

/// Cap on how much the selection flash advances in one frame (≈30 Hz), so an idle
/// gap before a selection change doesn't skip the flash. Matches the camera
/// transition step guard.
const MAX_FLASH_STEP: Duration = Duration::from_millis(33);

/// Startup warmup frames to pump after the first (core-only) frame (Phase B), so
/// the deferred GPU-resource build drains behind the already-shown grid. The
/// build completes in two stages (scene pipelines, then GTAO) — i.e. by the
/// third frame — so the extra frames are a safety margin and cost only a few cheap
/// grid redraws. `app` can't observe the render-side build state (invariant 2), so
/// this is a fixed count rather than a completion signal.
const STARTUP_WARMUP_FRAMES: u32 = 6;

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
            warmup_frames: 0,
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
            selection_flash: None,
            flashed_selection: Selection::None,
            selection_history: Vec::new(),
            recorded_selection: Selection::None,
            frame_showing_selection: false,
            texture_proxy: None,
            texture_watcher: None,
            watched_dirs: HashSet::new(),
            texture_cache: HashMap::new(),
            texture_pool: Vec::new(),
            notifications: Notifications::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Orbit,
    Pan,
    Zoom,
}

/// Whether two paths point at the same file, tolerating differences in form
/// (relative vs absolute, separators, `\\?\` prefixes) by falling back to
/// canonicalization — used to match a watcher event path against a bound texture.
/// The image extensions the texture pool accepts (the picker filter + the
/// drag-drop routing). Anything else dropped on the window is treated as a model.
const TEXTURE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "tga", "tif", "tiff", "psd", "bmp", "gif",
];

/// Whether `path`'s extension is one the texture pool accepts (case-insensitive).
fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| TEXTURE_EXTENSIONS.contains(&ext.as_str()))
}

/// A short human label for a texture path — its file name, or the full path when
/// it has no file-name component. Used in the notification toast captions.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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

impl ApplicationHandler<UserEvent> for App {
    /// Handle a custom event from the texture file-watcher: re-decode + re-upload
    /// the changed texture (the disk-auto-reload path; redraw stays here).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TextureChanged(path) => self.reload_texture_file(&path),
            UserEvent::TextureDecoded(decode) => self.handle_texture_decoded(decode),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        };

        let mut timer = startup_timing::StartupTimer::start();

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
        timer.lap("window_create");

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
        // Install fonts + visuals once: the style is derived purely from the
        // central theme tokens (no per-frame state), so it never needs re-syncing.
        init_style(&egui_ctx);
        timer.lap("renderer_init");

        let mut egui_painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            wgpu_configuration(renderer_config),
            EGUI_MSAA_SAMPLE_COUNT,
            Some(EGUI_DEPTH_FORMAT),
            false,
            true,
        ));
        timer.lap("adapter_device");
        // This first `set_window` is the largest remaining startup chunk (~265ms on
        // the RTX 4080 / DX12 dev box) and is INTRINSIC, not our overhead — do not
        // re-investigate without new evidence. `Painter::new` above only built the
        // wgpu *instance*; egui-wgpu defers all real GPU init to the first
        // `set_window`, which runs `RenderState::create`: adapter enumerate +
        // `request_device` + egui's `Renderer::new` + the first swapchain configure.
        // Measured split (via `--features startup-trace`, which turns egui-wgpu's +
        // wgpu's `profiling::scope!`s into logged spans): ~197ms `enumerate_adapters`
        // + ~47ms `request_device` + ~13ms egui `Renderer::new` + ~4ms swapchain.
        // The ~197ms is DX12 creating an `ID3D12Device` per adapter to probe its
        // features (4 adapters on this box: the 4080 cold-loads the NVIDIA driver
        // DLL, then 3 more probes). Phase D ruled out every angle: a background
        // driver *pre-warm* can't help (the cold driver-DLL load ~130ms dwarfs the
        // ~31ms head-start the main thread has before reaching here); *bypassing*
        // egui to call `request_adapter` ourselves pays the SAME per-adapter probing
        // cold (~200ms, measured), so it saves only noise; and it is unfixed upstream
        // through wgpu 29 / egui-wgpu 0.34 + wgpu-hal trunk (wgpu #3332, closed
        // "external: driver-bug"). Only a wgpu-hal fork skipping the 3 junk adapters
        // could trim it (~61ms), not worth the maintenance. Do NOT trade away
        // steady-state `AutoVsync` to chase it.
        pollster::block_on(egui_painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .expect("failed to initialize wgpu surface");
        timer.lap("surface_config");

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
            // Gate the AO toggle on the adapter being able to run GTAO (invariant 4).
            self.ui.gtao_supported = gtao_supported(&render_state.adapter);
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
        timer.lap("gating_state");

        // Load a file passed on the command line (file association / CLI arg)
        // now that the renderer exists. Reuses the same path as drag-drop, so
        // framing/stats/redraw behave identically.
        let had_initial_model = self.initial_model.is_some();
        if let Some(path) = self.initial_model.take() {
            self.open_model_from_path(&path);
        }
        if had_initial_model {
            timer.lap("initial_model_load");
        }

        // Paint the first frame directly rather than waiting on the first
        // `RedrawRequested`, so the window shows the rendered (grid-only) scene as
        // soon as it appears instead of an unpainted surface. Only the cheap core
        // resources build here (see `SceneResources::new_core`); the deferred scene
        // pipelines + GTAO pass compile over the next few frames, which the
        // startup warmup keeps pumping until the build drains.
        self.warmup_frames = STARTUP_WARMUP_FRAMES;
        self.render();
        timer.lap("first_frame");
        timer.finish();
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
                // An image dropped anywhere joins the scene texture pool (the
                // Inspector's "drop to add"); anything else is treated as a model.
                if is_image_path(&path) {
                    self.import_texture_path(path);
                } else {
                    self.open_model_from_path(&path);
                }
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
        // Advance the selection-highlight flash and feed this frame's fade into the
        // UI snapshot the scene callback reads. Done before the egui run below so the
        // viewport reflects the current fade; a change of selection (set by the
        // Outliner last frame) restarts it here.
        self.update_selection_flash();
        // Record any selection change the Outliner committed last frame into the
        // undo stack (and reset the F-frame toggle) before this frame's egui pass.
        self.record_selection_history();

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
            self.occlusion_bvh = Some(SceneBvh::build(&self.scene_model));
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
            // Borrowed as a disjoint field so the egui closure can show the toasts
            // alongside its `&mut self.ui` borrow (the toast system lives in `app`).
            let notifications = &mut self.notifications;
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
                ui_output = draw_overlay(
                    ctx,
                    &mut self.ui,
                    camera,
                    &scene_model,
                    occlusion_bvh,
                    output_format,
                );
                // Toasts paint on the egui Foreground layer, above the chrome.
                notifications.show(ctx);
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
        // The selection flash animates over ~0.5s; keep pacing frames until it
        // finishes so the highlight fades smoothly rather than freezing partway.
        let flash_active = self.selection_flash.is_some();
        // Pump startup warmup frames (Phase B) until the deferred GPU-resource
        // build drains, so the scene pipelines + GTAO pass compile behind
        // the already-shown grid. Paced like the other continuous-redraw sources.
        let warming_up = self.warmup_frames > 0;
        self.warmup_frames = self.warmup_frames.saturating_sub(1);
        self.repaint_at =
            if repaint_delay.is_zero() || camera_animating || flash_active || warming_up {
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

                let (materials_snapshot, material_revision) =
                    if let Some(renderer) = self.renderer.as_mut() {
                        frame_camera_to_model(renderer, &model);
                        renderer.reset_uv_camera();
                        // Seed the editable material table from the import defaults.
                        renderer.set_model_materials(&model.materials);
                        (renderer.material_snapshot(), renderer.material_revision())
                    } else {
                        (Vec::new(), 0)
                    };
                self.ui.materials_snapshot = materials_snapshot;
                self.ui.material_revision = material_revision;
                // A new model invalidates any Outliner selection (node / material
                // indices no longer apply); clear it, drop solo, and unhide every
                // mesh (the hidden node indices belong to the old model).
                self.ui.selection = Selection::None;
                self.ui.solo = false;
                self.ui.hidden_meshes.clear();
                // The undo stack holds the old model's selection indices; drop it.
                self.clear_selection_history();
                // The previous model's texture watches / decode cache no longer
                // apply (the fresh materials carry no slots).
                self.reset_texture_state();

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
                self.notifications
                    .success(format!("Loaded {}", file_label(path)));
                info!(path = %path.display(), "model loaded");
            }
            Err(error) => {
                self.notifications
                    .error(format!("Couldn't load {}", file_label(path)));
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
        // Escape clears any Outliner selection (mesh part or material). It's a Named
        // key, so handle it before the Character extraction below.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::Escape))
        {
            if self.ui.selection.is_active() {
                self.ui.selection = Selection::None;
                self.redraw_requested = true;
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
                self.undo_selection();
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

        // The Tex workspace is a pure-egui 2D viewer; only F / R apply, refitting
        // the image. The request makes the texture view ease to the fitted view on
        // its next paint (the animated counterpart of the zoom-readout toggle).
        if self.ui.mode == WorkspaceMode::Texture {
            if event.state == ElementState::Pressed && matches!(character.as_str(), "f" | "r") {
                self.ui.texture_view.request = Some(TexViewRequest::Fit);
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
            "f" => self.frame_camera_on_key(),
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

        let material_revision = if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_camera_to_home();
            renderer.reset_uv_camera();
            // Drop the previous model's editable materials.
            renderer.set_model_materials(&[]);
            renderer.material_revision()
        } else {
            0
        };
        self.ui.materials_snapshot = Vec::new();
        self.ui.material_revision = material_revision;
        self.ui.selection = Selection::None;
        self.ui.solo = false;
        self.ui.hidden_meshes.clear();
        self.clear_selection_history();
        self.reset_texture_state();

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
        if self.renderer.is_none() {
            return;
        }

        let mut redraw = false;

        // Camera + scalar material edits need the renderer borrow; scope it so the
        // texture intents below can call `&mut self` helpers.
        {
            let renderer = self.renderer.as_mut().unwrap();
            if let Some(action) = output.axis_gizmo_action {
                match action {
                    AxisGizmoAction::Orbit(delta) => renderer.orbit_camera(delta),
                    AxisGizmoAction::Snap(axis) => {
                        renderer.animate_camera_to_offset_direction(axis.offset_direction());
                    }
                    AxisGizmoAction::ResetView => renderer.animate_camera_to_home(),
                }
                redraw = true;
            }
            if let Some(edit) = output.material_edit {
                renderer.set_material_param(edit);
            }
        }

        // Refresh the UI snapshot + revision so the editor reflects the new value
        // and the scene callback re-uploads the table.
        if output.material_edit.is_some() {
            self.refresh_materials();
            redraw = true;
        }

        // One texture-pool command per frame (the Inspector emits at most one).
        if let Some(intent) = output.texture {
            match intent {
                // Clear a property's slot back to its fallback ("select texture").
                TextureIntent::Clear(slot_ref) => {
                    if let Some(slot) = TextureSlot::from_index(slot_ref.slot) {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.clear_texture_slot(slot_ref.material, slot);
                        }
                        self.refresh_materials();
                        redraw = true;
                    }
                }
                // Bind an already-decoded pooled texture to a property (applies now).
                TextureIntent::Assign(assign) => {
                    self.assign_pooled_texture(assign.slot, assign.path);
                    redraw = true;
                }
                // Remove a pooled texture (the ✕) — also unbinds every slot using it.
                TextureIntent::Remove(path) => {
                    self.remove_texture(&path);
                    redraw = true;
                }
                // Import textures into the pool ("Add textures…"). The modal picker
                // blocks the loop, so it does not schedule a redraw itself.
                TextureIntent::Import => self.import_textures(),
            }
        }

        if redraw {
            self.redraw_requested = true;
        }
    }

    /// Pull the renderer's editable-material snapshot + revision back into the UI
    /// (after any material edit / texture assignment / reload).
    fn refresh_materials(&mut self) {
        if let Some(renderer) = self.renderer.as_ref() {
            self.ui.materials_snapshot = renderer.material_snapshot();
            self.ui.material_revision = renderer.material_revision();
        }
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

    /// Advance the selection-highlight flash and write this frame's fade factor (1
    /// → 0 over [`SELECTION_FLASH`]) into [`UiState::selection_fade`], where the
    /// scene callback reads it. A change to a different node/material (set by the
    /// Outliner) restarts the flash; selecting nothing ends it. The fade lives in
    /// `app` because the redraw loop does (invariant 6) — the flash keeps requesting
    /// frames via `selection_flash.is_some()` in `render`, the same way camera
    /// transitions do. Like those, time accumulates from a capped per-frame delta so
    /// an idle gap before the selection can't fast-forward the flash.
    fn update_selection_flash(&mut self) {
        let now = Instant::now();

        // (Re)start the flash whenever the selection changes to a different target;
        // clear it when nothing is selected.
        if self.ui.selection != self.flashed_selection {
            self.flashed_selection = self.ui.selection;
            self.selection_flash = self.ui.selection.is_active().then_some(FlashProgress {
                elapsed: Duration::ZERO,
                last_tick: now,
            });
        }

        let fade = match self.selection_flash.as_mut() {
            Some(flash) => {
                let step = now.duration_since(flash.last_tick).min(MAX_FLASH_STEP);
                flash.last_tick = now;
                flash.elapsed += step;
                if flash.elapsed >= SELECTION_FLASH {
                    // Flash done: drop it so the pacing loop stops requesting frames.
                    self.selection_flash = None;
                    0.0
                } else {
                    // Ease-out (smoothstep complement): full at the start of the
                    // flash, easing smoothly to 0 — a blink that settles rather than
                    // a linear cut.
                    let t = (flash.elapsed.as_secs_f32() / SELECTION_FLASH.as_secs_f32())
                        .clamp(0.0, 1.0);
                    1.0 - t * t * (3.0 - 2.0 * t)
                }
            }
            None => 0.0,
        };
        self.ui.selection_fade = fade;
    }

    /// Record a committed selection change into the undo stack, run once per frame
    /// before the egui pass. The Outliner mutates the selection during the egui run,
    /// so the change is observed (and the prior value pushed) on the following frame.
    /// Also resets the `F`-frame toggle so a fresh selection frames the part first.
    fn record_selection_history(&mut self) {
        if self.ui.selection == self.recorded_selection {
            return;
        }
        if self.selection_history.len() >= SELECTION_HISTORY_LIMIT {
            self.selection_history.remove(0);
        }
        self.selection_history.push(self.recorded_selection);
        self.recorded_selection = self.ui.selection;
        self.frame_showing_selection = false;
    }

    /// Drop the selection undo stack and reset its change tracker, called when a
    /// new model loads (or the viewer resets): the stored node / material indices
    /// belong to the previous model. The caller has already set the live selection
    /// to `None`, so the tracker is synced to that.
    fn clear_selection_history(&mut self) {
        self.selection_history.clear();
        self.recorded_selection = Selection::None;
        self.frame_showing_selection = false;
    }

    /// Undo the most recent selection change (`Ctrl+Z`): pop the prior selection and
    /// restore it. Setting `recorded_selection` to the restored value keeps
    /// [`Self::record_selection_history`] from re-recording the undo as a new change.
    fn undo_selection(&mut self) {
        if let Some(previous) = self.selection_history.pop() {
            self.ui.selection = previous;
            self.recorded_selection = previous;
            self.frame_showing_selection = false;
            self.redraw_requested = true;
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
            // BC is OR'd in unmasked (not `& adapter.features()`): the baked IBL
            // cubes ship as BC6H, so the renderer hard-requires `TEXTURE_COMPRESSION_BC`.
            // It's universal on the desktop DX12/Vulkan/Metal targets; if some
            // adapter lacked it, device creation fails loudly here rather than
            // later at the IBL upload.
            required_features: (adapter.features()
                & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                | wgpu::Features::TEXTURE_COMPRESSION_BC,
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
