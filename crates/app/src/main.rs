// Suppress the console window in release builds — a shipped GUI viewer should
// open as a window, not alongside a terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Fully safe (invariant 9): the D3D11 bootstrap moved behind `review-render`'s
// safe `Gpu` wrapper, so no `unsafe` may land in this crate again.
#![forbid(unsafe_code)]

mod frame;
mod input;
mod prof;
mod texture_manager;
mod undo;
mod window_state;

/// Stream allocations to Tracy, but only while the client is running (started by
/// `--tracy`). On a normal launch this never starts the client and costs one
/// atomic load per allocation; the `unsafe` lives in `review_import` (invariant 9).
#[global_allocator]
static GLOBAL: review_import::TracyAllocator<std::alloc::System> =
    review_import::TracyAllocator::new(std::alloc::System);

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use glam::Vec2;
use notify::RecommendedWatcher;
use review_import::load_model;
use review_model::{ModelData, SceneBvh};
use review_render::{
    DecodedImage, Gpu, Renderer, RendererConfig, ShadingMode, TextureSlot, selection_bounds,
};
use review_ui::{
    AxisGizmoAction, MsaaSamples, Notifications, Selection, TexViewRequest, TextureIntent,
    UiOutput, UiState, WorkspaceMode, init_style,
};
use windows::Win32::Foundation::HWND;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
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
use undo::UndoStack;
use window_state::WindowPlacement;

fn main() -> anyhow::Result<()> {
    // Tiny manual arg scan (the workspace has no arg parser and needs exactly one
    // flag): `--tracy` turns profiling on; the first non-flag argument is the model
    // path to open (Windows passes it for a double-clicked `.fbx` via the file
    // association). The two coexist in any order, e.g. `3d-review --tracy a.fbx`.
    let mut tracy_enabled = false;
    let mut initial_model: Option<PathBuf> = None;
    for arg in std::env::args_os().skip(1) {
        if arg == "--tracy" {
            tracy_enabled = true;
        } else if initial_model.is_none() && !arg.to_string_lossy().starts_with("--") {
            initial_model = Some(PathBuf::from(arg));
        }
    }

    // Start the Tracy client only when asked, and keep the handle alive for the
    // whole process (dropping the last handle disconnects). With `manual-lifetime`
    // the client never auto-starts, so a normal launch opens no socket and every
    // zone/plot/message/alloc hook no-ops. `ondemand` means even a started client
    // buffers nothing until a Tracy server connects.
    let tracy = tracy_enabled.then(|| {
        let client = tracy_client::Client::start();
        prof::thread_name("main");
        prof::msg("3d-review: Tracy profiling enabled (--tracy)");
        client
    });

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);

    // The texture file-watcher posts reload events back through this proxy.
    let texture_proxy = event_loop.create_proxy();

    let mut app = App {
        initial_model,
        textures: TextureSubsystem {
            proxy: Some(texture_proxy),
            ..TextureSubsystem::default()
        },
        tracy_enabled,
        _tracy: tracy,
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
    /// The Direct3D 11 device + immediate context + window swapchain.
    gpu: Option<Gpu>,
    /// egui's Direct3D 11 renderer (replaces egui-wgpu). Draws the chrome on top of
    /// the scene each frame.
    egui_renderer: Option<egui_directx11::Renderer>,
    drag_mode: Option<DragMode>,
    last_pointer_position: Option<Vec2>,
    last_primary_click: Option<(Instant, Vec2)>,
    /// Latest keyboard modifier state, tracked from `ModifiersChanged` so
    /// per-key events (which don't carry modifiers in winit) can test for
    /// chords like Ctrl+N.
    modifiers: ModifiersState,
    /// The on-demand redraw scheduler (invariant 6): pacing state that decides
    /// when the next frame draws.
    redraw: RedrawScheduler,
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
    /// The window-placement tracker: startup maximize + the windowed bounds
    /// persisted on exit.
    placement: PlacementTracker,
    /// Live selection-highlight flash, or `None` when none is playing. Started when
    /// [`Self::flashed_selection`] no longer matches the UI's current selection, and
    /// advanced each frame by [`Self::update_selection_flash`], which writes the
    /// fade into [`UiState::selection_fade`] for the scene callback.
    selection_flash: Option<FlashProgress>,
    /// The selection the flash is currently animating (or last animated), so a
    /// change to a *different* node/material restarts the flash and selecting
    /// nothing ends it.
    flashed_selection: Selection,
    /// The unified undo/redo history for all document edits (selection, hide/
    /// unhide, material params, texture slot bindings, and the texture pool). Fed
    /// once per frame by [`Self::observe_edit_state`]; `Ctrl+Z` / `Ctrl+Y` restore
    /// a step. Lives in `app` because it coordinates the UI + renderer + pool state
    /// (invariant 2) and the redraw loop / keyboard routing are here. See
    /// `undo.rs`.
    undo: UndoStack,
    /// Whether a material editor widget is being actively dragged this frame
    /// (mirrored from [`UiOutput::material_edit_active`] after the egui pass), so
    /// the undo observer coalesces a continuous drag into a single step.
    drag_in_progress: bool,
    /// Whether the camera is currently framed on the selection rather than the
    /// whole model, so `F` alternates between the two while a mesh part is
    /// selected. Reset whenever the selection changes (the next `F` frames the
    /// part first).
    frame_showing_selection: bool,
    /// The scene texture pool + decode cache + disk-auto-reload subsystem; its
    /// logic lives in `texture_manager.rs`.
    textures: TextureSubsystem,
    /// The toast notification system (egui-notify). `app` owns it because it owns
    /// the egui frame and triggers the notifications (texture decode start/finish);
    /// the UI crate only provides the themed type. Shown once per frame in `render`.
    notifications: Notifications,
    /// Whether `--tracy` was passed: arms the hand-rolled D3D11 GPU timestamp
    /// profiler (via `review_render::enable_tracy_gpu`) in `resumed`. The scene
    /// renderer then builds the profiler lazily once a Tracy client connects.
    tracy_enabled: bool,
    /// A GPU fault (scene/egui render failure, device lost) has already been
    /// surfaced as a toast this session. Faults repeat every frame once the
    /// device is wedged, so the toast fires once instead of stacking forever.
    gpu_fault_notified: bool,
    /// The live Tracy client handle, held for the whole process so the profiler
    /// session stays up (dropping the last handle disconnects). `None` on a normal
    /// launch — the client is never started, so all instrumentation no-ops.
    _tracy: Option<tracy_client::Client>,
}

/// The on-demand redraw scheduler (invariant 6): everything that decides *when*
/// the next frame is drawn, grouped out of [`App`].
struct RedrawScheduler {
    last_render_instant: Option<Instant>,
    /// When egui has asked to be repainted at a future time (e.g. a UI fade
    /// animation). Drives `ControlFlow::WaitUntil` so the loop sleeps until then
    /// instead of spinning. `None` = wait for the next input/redraw event.
    repaint_at: Option<Instant>,
    /// An interactive event (drag, hover, wheel, key) has requested a redraw.
    /// Folded into the paced `repaint_at` schedule in `about_to_wait` rather than
    /// triggering an immediate `request_redraw`, so a high-polling-rate mouse or
    /// key auto-repeat can't drive rendering faster than the monitor refresh.
    requested: bool,
    /// Remaining startup "warmup" frames to pump (Phase B). The first frame builds
    /// only the cheap core scene resources; the deferred scene pipelines + GTAO
    /// pass then compile one stage per subsequent frame. While this is non-zero,
    /// `render` keeps scheduling the next frame so the build drains behind the
    /// already-shown grid, then stops. Seeded once in `resumed`; `app` can't see
    /// the render-side build state (invariant 2), so it pumps a fixed count.
    warmup_frames: u32,
    /// Minimum spacing between continuously-rendered frames, derived from the
    /// active monitor's refresh rate. Caps redraw to the display so animation
    /// doesn't render faster than it can be shown. Defaults to 60 Hz until a
    /// monitor is known.
    refresh_interval: Duration,
}

impl Default for RedrawScheduler {
    fn default() -> Self {
        Self {
            last_render_instant: None,
            repaint_at: None,
            requested: false,
            warmup_frames: 0,
            refresh_interval: Duration::from_secs_f64(1.0 / FALLBACK_REFRESH_HZ),
        }
    }
}

/// The window-placement tracker: the startup maximize hint plus the windowed
/// bounds sampled for session persistence, grouped out of [`App`].
#[derive(Default)]
struct PlacementTracker {
    /// The process was launched with a request to start maximized (e.g. a
    /// shortcut set to **Run: Maximized**). Set in `resumed` and applied at
    /// window creation, since winit doesn't honor the OS hint on its own.
    start_maximized: bool,
    /// The most recent *non-maximized* window placement (outer position + inner
    /// size), tracked from `Moved`/`Resized` events so it's available to persist
    /// on exit. Recorded only while the window isn't maximized, so un-maximizing
    /// a restored session returns to a real window rather than a fullscreen rect.
    last_windowed_bounds: Option<((i32, i32), (u32, u32))>,
    /// Set when a `Moved`/`Resized` event arrives; the windowed bounds are then
    /// sampled once in `about_to_wait`, after the event burst has settled. This
    /// deferral matters for maximize: winit dispatches `Moved` (from
    /// `WM_WINDOWPOSCHANGED`) *before* the `WM_SIZE` that sets its maximized flag,
    /// so sampling eagerly in the `Moved` handler would record the maximized
    /// geometry as if it were windowed. By `about_to_wait` the flag is set, so
    /// `record_windowed_bounds`'s `is_maximized()` guard sees the real state.
    bounds_dirty: bool,
}

/// The scene texture pool + decode cache + disk-auto-reload subsystem's state,
/// grouped out of [`App`]; the logic lives in `texture_manager.rs`.
#[derive(Default)]
struct TextureSubsystem {
    /// Proxy used by the texture file-watcher thread to post reload events to the
    /// event loop (set in `main` before the loop runs).
    proxy: Option<EventLoopProxy<UserEvent>>,
    /// The disk-auto-reload watcher, created lazily on the first texture
    /// assignment. Dropping it stops watching (done on model load / reset).
    watcher: Option<RecommendedWatcher>,
    /// Directories the watcher is registered on (the parents of assigned textures),
    /// so each directory is watched at most once.
    watched_dirs: HashSet<PathBuf>,
    /// Decoded-image cache keyed by source path, so a packed map assigned to
    /// several slots / materials decodes once. Cleared on model load / reset.
    cache: HashMap<PathBuf, Arc<DecodedImage>>,
    /// The scene-wide texture pool: imported source paths in insertion order. The
    /// decoded pixels live in [`Self::cache`]; this is just the ordered set the
    /// Inspector's Texture files list + property dropdowns draw from (mirrored
    /// into `UiState::texture_pool` by `App::refresh_texture_pool`). Cleared on
    /// model load / reset.
    pool: Vec<PathBuf>,
    /// Monotonic change tag for the pool + cache, bumped on every mutation. Lets
    /// `App::capture_edit_state` detect pool changes (and share the pool snapshot
    /// `Arc` when unchanged) as cheaply as the renderer's `material_revision`
    /// does for the material table.
    revision: u64,
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

/// Redraw cadence used until the real monitor refresh rate is known (and as the
/// fallback when it can't be queried) — a conventional 60 Hz.
const FALLBACK_REFRESH_HZ: f64 = 60.0;

/// Minimum window inner size (logical points), so the chrome never collapses.
const MIN_WINDOW_WIDTH: f64 = 960.0;
const MIN_WINDOW_HEIGHT: f64 = 640.0;

/// Camera zoom per pixel of a right-button zoom-drag (pointer-down zooms in).
const DRAG_ZOOM_SENSITIVITY: f32 = 0.01;

/// Camera zoom per wheel notch for line-based scroll deltas (mice).
const WHEEL_LINE_ZOOM_STEP: f32 = 0.5;

/// Pixel-precise scroll (trackpads) divided by this to match one wheel notch.
const WHEEL_PIXELS_PER_ZOOM_STEP: f32 = 120.0;

/// A second primary click counts as a double-click only within this interval…
const DOUBLE_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(450);
/// …and only if the pointer stayed within this many physical pixels of the first.
const DOUBLE_CLICK_MAX_DISTANCE_PX: f32 = 6.0;

/// Hermite smoothstep `3t² − 2t³` on a clamped `t ∈ [0, 1]` — an ease with zero
/// slope at both ends.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

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
            gpu: None,
            egui_renderer: None,
            drag_mode: None,
            last_pointer_position: None,
            last_primary_click: None,
            modifiers: ModifiersState::empty(),
            redraw: RedrawScheduler::default(),
            scene_model,
            scene_revision: 0,
            occlusion_bvh: None,
            // A sentinel distinct from the initial `scene_revision` (0) so the BVH
            // is treated as stale until first built.
            occlusion_bvh_revision: u64::MAX,
            ui,
            initial_model: None,
            placement: PlacementTracker::default(),
            selection_flash: None,
            flashed_selection: Selection::None,
            undo: UndoStack::new(),
            drag_in_progress: false,
            frame_showing_selection: false,
            textures: TextureSubsystem::default(),
            notifications: Notifications::new(),
            tracy_enabled: false,
            gpu_fault_notified: false,
            _tracy: None,
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

impl App {
    /// Startup phase 1: create the application window, honoring the OS maximize
    /// hint and restoring the persisted placement (dropped when it lands on a
    /// disconnected monitor, so the window can't open off-screen).
    fn create_startup_window(&mut self, event_loop: &ActiveEventLoop) -> Arc<Window> {
        // Honor the OS launch hint (e.g. a shortcut set to "Run: Maximized").
        // winit never consults `STARTUPINFO.wShowWindow`, so we query it and set
        // the initial state ourselves.
        self.placement.start_maximized = review_import::startup_show_maximized();

        let saved =
            window_state::load().filter(|placement| placement_is_visible(event_loop, placement));
        let maximized =
            self.placement.start_maximized || saved.is_some_and(|placement| placement.maximized);

        // The position/size in a maximized placement are meant to be the *restored*
        // (pre-maximize) bounds, used as the un-maximize target. Pre-fix builds could
        // instead record the maximized geometry itself; applying that as the restore
        // target leaves un-maximize landing on a full-screen rect. When the saved
        // bounds fill a monitor, substitute a sane centered window so un-maximize and
        // the re-saved placement describe a real window (not the full screen).
        let restore_bounds = saved.map(|placement| {
            if placement.maximized && placement_fills_monitor(event_loop, &placement) {
                default_windowed_placement(event_loop, &placement)
            } else {
                placement
            }
        });

        let mut attributes = WindowAttributes::default()
            .with_title("3D Review")
            .with_window_icon(load_window_icon())
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                MIN_WINDOW_WIDTH,
                MIN_WINDOW_HEIGHT,
            ))
            .with_maximized(maximized);
        if let Some(placement) = restore_bounds {
            // Position/size are the restored (non-maximized) bounds; setting them
            // even when maximized gives un-maximize a sensible target.
            attributes = attributes
                .with_position(winit::dpi::PhysicalPosition::new(placement.x, placement.y))
                .with_inner_size(winit::dpi::PhysicalSize::new(
                    placement.width,
                    placement.height,
                ));
            self.placement.last_windowed_bounds = Some((
                (placement.x, placement.y),
                (placement.width, placement.height),
            ));
        }

        Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create application window"),
        )
    }

    /// Startup phase 4: wire the built pieces onto `self`, seed the UI's initial
    /// stats + capability gates, and arm the optional GPU profiler.
    fn init_shell(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
        renderer: Renderer,
        egui_ctx: egui::Context,
        gpu: Gpu,
        egui_renderer: egui_directx11::Renderer,
    ) {
        // The viewer talks to the GPU through Direct3D 11.
        self.ui.gpu_backend = "DX11".to_string();
        // Gate the Anti-Aliasing menu on the adapter's real MSAA support (D3D11
        // `CheckMultisampleQualityLevels` for the scene HDR + depth formats). IBL + AO
        // stay enabled — the device requires `TEXTURE_COMPRESSION_BC` and feature
        // level 11_0+ guarantees the `Rgba16Float`/`Rg16Float` + compute they need, so
        // both are universal on the desktop DX11 targets.
        let supported_counts = gpu.supported_msaa_counts();
        self.ui.supported_msaa = MsaaSamples::ALL
            .into_iter()
            .filter(|level| supported_counts.contains(&level.sample_count()))
            .collect();

        // Under `--tracy`, arm the hand-rolled D3D11 GPU timestamp profiler. The scene
        // renderer builds it lazily on the first frame once a Tracy client connects; a
        // normal launch never calls this, so the scene passes record no timestamps.
        if self.tracy_enabled {
            review_render::enable_tracy_gpu();
        }

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            // D3D11 feature level 11_0+ guarantees 16384 max 2D texture dimension.
            Some(16384),
        );

        self.renderer = Some(renderer);
        self.egui_ctx = Some(egui_ctx);
        self.egui_state = Some(egui_state);
        self.gpu = Some(gpu);
        self.egui_renderer = Some(egui_renderer);
        self.ui.stats = self.scene_model.stats;
        self.ui.bounds = self.scene_model.bounds;
        self.ui.uv_sets = self.scene_model.uv_set_labels();
        self.redraw.refresh_interval = monitor_refresh_interval(&window);
        self.window = Some(window);
    }
}

/// Startup phase 2: the renderer with its cameras seeded for the real window
/// size / safe area, so the startup view matches what reset
/// (`animate_camera_to_home`) produces instead of the full-window default.
fn build_startup_renderer(window: &Window) -> Renderer {
    let mut renderer = Renderer::new(RendererConfig::default());
    let size = window.inner_size();
    if size.height > 0 {
        renderer.set_camera_aspect_ratio(size.width as f32 / size.height as f32);
        renderer.set_uv_aspect_ratio(size.width as f32 / size.height as f32);
        let (safe_w, safe_h) = framing_safe_area(size.height, window.scale_factor() as f32);
        renderer.set_framing_safe_area(safe_w, safe_h);
        renderer.reset_camera_to_home();
    }
    renderer
}

/// Startup phase 3: the Direct3D 11 device + swapchain and egui's D3D11
/// renderer. A single `D3D11CreateDevice` on the default adapter — no DX12
/// multi-adapter probing, no naga — so this is far cheaper than the old
/// egui-wgpu `set_window` path. Also kills the white startup flash by clearing
/// and presenting the fresh backbuffer black before any scene exists (this
/// replaced the old GDI startup paint).
fn create_startup_gpu(window: &Window) -> (Gpu, egui_directx11::Renderer) {
    let size = window.inner_size();
    let gpu = Gpu::new(win32_hwnd(window), size.width, size.height)
        .expect("failed to create the Direct3D 11 device + swapchain");
    gpu.clear_backbuffer([0.0, 0.0, 0.0, 1.0]);
    // A device lost at the very first present is unrecoverable startup
    // failure territory; note it and let the frame loop surface the toast.
    if let review_render::PresentStatus::DeviceLost { reason } = gpu.present(false) {
        prof::msg(&format!(
            "startup present failed: device lost ({reason:#x})"
        ));
    }
    // egui renders through egui-directx11 on the same device/context.
    let egui_renderer = egui_directx11::Renderer::new(gpu.device())
        .expect("failed to create the egui Direct3D 11 renderer");
    (gpu, egui_renderer)
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

        // Startup phase timing, as a sequence of Tracy zones (replacing the old
        // `StartupTimer` laps): `phase` holds the current zone and is ended by
        // dropping its guard before the next begins, so they read as adjacent spans
        // under the "main" thread. All no-op unless `--tracy` started the client.
        // Each phase's work lives in its own helper below.
        let mut phase = prof::zone!("Window Create");
        let window = self.create_startup_window(event_loop);

        drop(phase.take());
        phase = prof::zone!("Renderer Init");
        let renderer = build_startup_renderer(&window);
        let egui_ctx = egui::Context::default();
        // Install fonts + visuals once: the style is derived purely from the
        // central theme tokens (no per-frame state), so it never needs re-syncing.
        init_style(&egui_ctx);

        drop(phase.take());
        phase = prof::zone!("D3D11 Device + Swapchain");
        let (gpu, egui_renderer) = create_startup_gpu(&window);

        drop(phase.take());
        phase = prof::zone!("Shell Init");
        self.init_shell(event_loop, window, renderer, egui_ctx, gpu, egui_renderer);
        prof::msg("application shell started");

        drop(phase.take());
        phase = prof::zone!("Initial Model Load");
        // Load a file passed on the command line (file association / CLI arg)
        // now that the renderer exists. Reuses the same path as drag-drop, so
        // framing/stats/redraw behave identically.
        if let Some(path) = self.initial_model.take() {
            self.open_model_from_path(&path);
        }

        // Paint the first frame directly rather than waiting on the first
        // `RedrawRequested`, so the window shows the rendered (grid-only) scene as
        // soon as it appears instead of an unpainted surface. Only the cheap core
        // resources build here; the deferred scene pipelines + GTAO pass compile
        // over the next few frames, which the startup warmup keeps pumping until
        // the build drains.
        self.redraw.warmup_frames = STARTUP_WARMUP_FRAMES;
        drop(phase.take());
        phase = prof::zone!("First Frame");
        self.render();
        drop(phase);
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
            self.redraw.requested = true;
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
                // Defer recording: a maximize delivers `Moved` before the window's
                // maximized flag is set, so sample in `about_to_wait` instead.
                self.placement.bounds_dirty = true;
            }
            WindowEvent::Resized(size) => self.handle_resized(size, &window),
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(state, button, egui_consumed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position, &window)
            }
            WindowEvent::CursorLeft { .. } => {
                self.drag_mode = None;
                self.last_pointer_position = None;
            }
            WindowEvent::MouseWheel { delta, .. } => self.handle_mouse_wheel(delta, egui_consumed),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } if !egui_consumed => {
                // Any key press dismisses the startup help overlay (and still
                // performs its shortcut). Request a redraw so it clears even for
                // keys that aren't bound to a shortcut.
                if event.state == ElementState::Pressed && self.ui.show_help_overlay {
                    self.ui.show_help_overlay = false;
                    self.redraw.requested = true;
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

        // Sample windowed bounds once the event burst has settled, so a maximize
        // (whose `Moved` arrives before the maximized flag is set) doesn't poison
        // the saved placement with fullscreen geometry. `record_windowed_bounds`
        // skips while maximized, so the pre-maximize bounds survive.
        if self.placement.bounds_dirty {
            self.placement.bounds_dirty = false;
            self.record_windowed_bounds();
        }

        // Fold a pending interactive redraw (drag, hover, wheel, key) into the
        // paced schedule. The earliest we'll draw is one refresh interval after
        // the last frame, so a burst of high-frequency input events coalesces
        // into a single redraw capped at the monitor refresh rate.
        if self.redraw.requested {
            self.redraw.requested = false;
            let earliest = self
                .redraw
                .last_render_instant
                .map_or(now, |last| last + self.redraw.refresh_interval);
            self.redraw.repaint_at = Some(
                self.redraw
                    .repaint_at
                    .map_or(earliest, |at| at.min(earliest)),
            );
        }

        // Sleep until the next scheduled repaint (if any), otherwise block until
        // the next input event. When the scheduled time arrives, fire one redraw
        // and fall back to waiting.
        match self.redraw.repaint_at {
            Some(wake) if now >= wake => {
                self.redraw.repaint_at = None;
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

        // Loading into the empty viewport (first load, or after Ctrl+N) shows
        // the model already framed — a fly-in from the home view would only
        // delay it. Replacing an already-loaded model keeps the animated
        // re-frame so the view change reads as a transition.
        let animate_framing = !self.scene_model.vertices.is_empty();

        match load_model(path) {
            Ok(model) => {
                let model = Arc::new(model);

                let (materials_snapshot, material_revision) =
                    if let Some(renderer) = self.renderer.as_mut() {
                        frame_camera_to_model(renderer, &model, animate_framing);
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
                self.ui.reset_skeletal_state(&model);
                // The previous model's texture watches / decode cache no longer
                // apply (the fresh materials carry no slots).
                self.reset_texture_state();
                // The undo history references the old model's indices / materials /
                // pool; drop it and rebaseline to this freshly-loaded state.
                self.reset_undo_history();

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
                prof::msg(&format!("model loaded: {}", path.display()));
            }
            Err(error) => {
                // Surface the cause, not just the file name — without `--tracy`
                // the prof channel below is the user's only *hidden* diagnostic.
                self.notifications
                    .error(format!("Couldn't load {}: {error}", file_label(path)));
                prof::msg(&format!("model load failed: {} ({error})", path.display()));
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
    ///
    /// Adding/changing a binding here? Update the startup help card's tables in
    /// `crates/ui/src/help.rs` (`LEFT_SHORTCUTS` / `RIGHT_SHORTCUTS` /
    /// `CHORD_SHORTCUTS`) in the same change — they are the user-facing mirror
    /// of this dispatch.
    fn handle_keyboard_shortcut(&mut self, event: &KeyEvent) {
        // Escape clears any Outliner selection (mesh part or material). It's a Named
        // key, so handle it before the Character extraction below.
        if event.state == ElementState::Pressed
            && matches!(&event.logical_key, Key::Named(NamedKey::Escape))
        {
            if self.ui.selection.is_active() {
                self.ui.selection = Selection::None;
                self.redraw.requested = true;
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
                // Ctrl+Z undoes; Ctrl+Shift+Z redoes (the common alt-redo chord).
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
                self.redraw.requested = true;
            }
            return;
        }

        // The Tex workspace is a pure-egui 2D viewer; only F / R apply, refitting
        // the image. The request makes the texture view ease to the fitted view on
        // its next paint (the animated counterpart of the zoom-readout toggle).
        if self.ui.mode == WorkspaceMode::Texture {
            if event.state == ElementState::Pressed && matches!(character.as_str(), "f" | "r") {
                self.ui.texture_view.request = Some(TexViewRequest::Fit);
                self.redraw.requested = true;
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
            self.redraw.requested = true;
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

        self.redraw.requested = true;
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
        self.ui.reset_skeletal_state(&self.scene_model);
        self.reset_texture_state();
        // Drop the undo history (it references the previous model) and rebaseline
        // to the empty start state.
        self.reset_undo_history();

        prof::msg("reset to start state");
        self.redraw.requested = true;
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

        last_click_time.elapsed() <= DOUBLE_CLICK_MAX_INTERVAL
            && current_position.distance(last_click_position) <= DOUBLE_CLICK_MAX_DISTANCE_PX
    }

    fn apply_ui_output(&mut self, output: UiOutput) {
        // Mirror this frame's material-drag state for the undo observer (read at the
        // top of the next frame), so a continuous slider / color drag coalesces into
        // a single undo step. Tracked even when the renderer isn't ready yet.
        self.drag_in_progress = output.material_edit_active;

        if self.renderer.is_none() {
            return;
        }

        let mut redraw = false;

        // Camera + scalar material edits need the renderer borrow; scope it so the
        // texture intents below can call `&mut self` helpers.
        {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
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
            self.redraw.requested = true;
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
        self.placement.last_windowed_bounds =
            Some(((position.x, position.y), (size.width, size.height)));
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
        let Some(((x, y), (width, height))) = self.placement.last_windowed_bounds else {
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
                    let t = flash.elapsed.as_secs_f32() / SELECTION_FLASH.as_secs_f32();
                    1.0 - smoothstep(t)
                }
            }
            None => 0.0,
        };
        self.ui.selection_fade = fade;
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
            .redraw
            .last_render_instant
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.redraw.last_render_instant = Some(now);

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

        // Tracy plots (no-op unless `--tracy`): the live smoothed frame rate plus
        // the measured model stats, so they read alongside the timeline.
        prof::plot!("FPS", self.ui.fps);
        prof::plot!("Triangles", self.ui.stats.triangle_count as f64);
        prof::plot!("Draw Calls", self.ui.stats.draw_count as f64);

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

/// Whether a placement's rect covers essentially a whole monitor — the signature
/// of *maximized* geometry rather than real restored bounds. A maximized window's
/// outer position sits at (or a few px past) the monitor's top-left and its client
/// spans the work area, so the saved rect starts at/left-of the monitor origin and
/// is nearly as large as the monitor. Used to reject restore bounds that are really
/// maximized geometry recorded by a pre-fix build, so un-maximize doesn't land on a
/// full-screen rect. The fraction thresholds (not pixel counts) absorb the taskbar
/// and DPI-dependent frame overhang without misflagging a normal or snapped window.
fn placement_fills_monitor(event_loop: &ActiveEventLoop, placement: &WindowPlacement) -> bool {
    // Maximized client width tracks the work area exactly (≈100% with a bottom
    // taskbar, a bit less with a side taskbar); height loses the taskbar band.
    const MIN_WIDTH_FRACTION: f32 = 0.9;
    const MIN_HEIGHT_FRACTION: f32 = 0.85;

    for monitor in event_loop.available_monitors() {
        let pos = monitor.position();
        let size = monitor.size();
        let at_origin = placement.x <= pos.x && placement.y <= pos.y;
        let fills = placement.width as f32 >= size.width as f32 * MIN_WIDTH_FRACTION
            && placement.height as f32 >= size.height as f32 * MIN_HEIGHT_FRACTION;
        if at_origin && fills {
            return true;
        }
    }
    false
}

/// A comfortable centered restored window, used as the un-maximize target when the
/// saved bounds are unusable (they described maximized geometry — see
/// [`placement_fills_monitor`]). Sized to a fraction of the primary monitor and
/// centered on it, keeping `maximized` so the window still opens maximized. Falls
/// back to the original placement if no monitor can be enumerated.
fn default_windowed_placement(
    event_loop: &ActiveEventLoop,
    fallback: &WindowPlacement,
) -> WindowPlacement {
    /// Fraction of the monitor a default restored window occupies.
    const SIZE_FRACTION: f32 = 0.7;

    let Some(monitor) = event_loop
        .primary_monitor()
        .or_else(|| event_loop.available_monitors().next())
    else {
        return *fallback;
    };

    let pos = monitor.position();
    let size = monitor.size();
    let width = (size.width as f32 * SIZE_FRACTION) as u32;
    let height = (size.height as f32 * SIZE_FRACTION) as u32;
    WindowPlacement {
        x: pos.x + (size.width.saturating_sub(width) / 2) as i32,
        y: pos.y + (size.height.saturating_sub(height) / 2) as i32,
        width,
        height,
        maximized: fallback.maximized,
    }
}

/// The active monitor's refresh interval, used to cap continuous redraw. Falls
/// back to 60 Hz when winit can't report a rate (some virtual/headless displays).
fn monitor_refresh_interval(window: &Window) -> Duration {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .filter(|millihertz| *millihertz > 0)
        .map(|millihertz| Duration::from_secs_f64(1000.0 / f64::from(millihertz)))
        .unwrap_or_else(|| Duration::from_secs_f64(1.0 / FALLBACK_REFRESH_HZ))
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

fn frame_camera_to_model(renderer: &mut Renderer, model: &ModelData, animate: bool) {
    if let Some(bounds) = model.bounds {
        if animate {
            renderer.animate_camera_to_bounds(bounds);
        } else {
            renderer.snap_camera_to_bounds(bounds);
        }
    }
}

/// Extract the Win32 `HWND` from a winit window, for DXGI swapchain creation. The
/// viewer is Windows-only, so a non-Win32 handle is an unrecoverable error.
fn win32_hwnd(window: &Window) -> HWND {
    match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => HWND(handle.hwnd.get() as *mut core::ffi::c_void),
        other => panic!("expected a Win32 window handle, got {other:?}"),
    }
}
