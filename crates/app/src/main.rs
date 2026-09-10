// Suppress the console window in release builds — a shipped GUI viewer should
// open as a window, not alongside a terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Fully safe (invariant 9): the D3D11 bootstrap moved behind `review-render`'s
// safe `Gpu` wrapper, so no `unsafe` may land in this crate again.
#![forbid(unsafe_code)]

mod animation;
mod dialog;
mod flycam;
mod frame;
mod gate;
mod input;
mod loading;
mod opt;
mod prof;
mod selection_flash;
mod shortcuts;
mod texture_manager;
mod ui_intents;
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
use review_model::{ModelData, SceneBvh};
use review_render::{DecodedImage, EguiRenderer, Gpu, GpuBringUp, Renderer, RendererConfig};
use review_ui::{MsaaSamples, Notifications, Selection, UiState, init_style};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::ModifiersState,
    window::{Window, WindowAttributes, WindowId},
};

/// Custom event posted from a background thread to the winit event loop, so work
/// done off the main thread is applied back on it (the redraw loop + all renderer
/// state stay in `app` — invariant 6).
///
/// Deliberately not `Clone`: a variant carries a whole LOD chain of meshes, and
/// an accidental clone would deep-copy every one of them (invariant 1).
#[derive(Debug)]
enum UserEvent {
    /// A watched directory reported a change to this path; if it's a bound texture,
    /// re-decode + re-upload it (posted by the file-watcher thread).
    TextureChanged(PathBuf),
    /// A background texture decode finished (posted by the decode thread). The
    /// result is uploaded + the slot/binding updated here on the main thread.
    TextureDecoded(TextureDecode),
    /// A background Opt processing run finished (posted by the optimize thread).
    /// Boxed because a `ProcessedResult` carries a mesh per LOD level, which
    /// would otherwise make every variant of this enum that large.
    OptProcessed(Box<opt::OptProcessed>),
    /// A background FBX export finished (posted by the export thread).
    OptExported(Box<Result<review_optimize::ExportReport, review_optimize::OptError>>),
    /// A background model import produced a drawable model (posted by the import
    /// thread). Boxed because it carries the whole parsed model.
    ModelLoaded(Box<loading::ModelLoaded>),
    /// A background model import reached a new stage, or moved within one
    /// (posted by the import thread, already throttled there — see `loading.rs`).
    /// Rewrites the loading card's stage line in place.
    ModelLoadProgress(loading::ModelLoadProgress),
    /// One of the measurements the import deferred until after the model was on
    /// screen has landed (posted by the same thread, which keeps measuring once
    /// it has published the mesh). Boxed because the draw-group table is one
    /// entry per (node, material) pair.
    ModelMeasured(Box<loading::ModelMeasured>),
    /// A native file dialog closed (posted by the thread that opened it —
    /// `mac-port-plan.md` D9). `None` when the user cancelled. Boxed because the
    /// export variant carries a whole LOD chain's worth of `Arc`s.
    DialogDone(Option<Box<dialog::DialogAnswer>>),
    /// The OS asked for a file to be opened (`mac-port-plan.md` D14): a Finder
    /// double-click, an `open(1)`, or a drop on the Dock icon. macOS only —
    /// Windows delivers the same intent as `argv[1]`, which `main` reads directly.
    OpenPath(PathBuf),
    /// A macOS menu item the viewer performs itself was chosen (D15). Routed
    /// through the loop rather than acted on in muda's callback so it lands on the
    /// main thread, in order with every other event, instead of racing the state
    /// it is about to change.
    MenuCommand(review_shell_macos::MenuCommand),
}

use animation::AnimationSubsystem;
use flycam::FlyCam;
use gate::Gate;
use selection_flash::FlashProgress;
use texture_manager::TextureDecode;
use undo::UndoStack;
use window_state::{
    default_windowed_placement, monitor_refresh_interval, placement_fills_monitor,
    placement_is_visible,
};

fn main() -> anyhow::Result<()> {
    // The GPU device and `sg_setup` need no window and are the longest single item on
    // the launch path, so they start here, before anything else, and run alongside
    // the window creation and the argument scan (`mac-port-plan.md` D8). The join is
    // in `App::start`.
    let gpu_bring_up = Gpu::start();

    // Tiny manual arg scan (the workspace has no arg parser and needs two flags):
    // `--tracy` turns profiling on, `--gate-out <file>` runs the D2 measurement and
    // writes its stamp there (`gate.rs`); the first non-flag argument is the model
    // path to open (Windows passes it for a double-clicked `.fbx` via the file
    // association). They coexist in any order, e.g. `3d-review --tracy a.fbx`.
    let mut tracy_enabled = false;
    let mut initial_model: Option<PathBuf> = None;
    let mut gate_out: Option<PathBuf> = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--tracy" {
            tracy_enabled = true;
        } else if arg == "--gate-out" {
            gate_out = args.next().map(PathBuf::from);
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

    // The two macOS shell leaves, installed here because both need the event loop
    // to exist (the delegate to extend, and `NSApplication` to hang a menu on) and
    // neither may run once it does. Both are no-op stubs on Windows, so there is no
    // `cfg` here — see `review_shell_macos`.
    //
    // A `.fbx` opened from Finder arrives as an Apple event, not as `argv[1]`
    // (D14): a fresh launch gets no arguments at all, and an already-running app
    // gets no new process. Without the hook the file association does nothing.
    let open_proxy = event_loop.create_proxy();
    review_shell_macos::install_open_handler(move |path| {
        // A closed event loop means the app is already exiting; a dropped open is
        // the right answer then.
        let _ = open_proxy.send_event(UserEvent::OpenPath(path));
    });

    // Kept alive for the life of the process: dropping the handle takes the menu
    // bar with it (D15). Its ⌘O / ⌘N accelerators intercept those chords before
    // winit sees them, which is why they route to the same two handlers
    // `shortcuts.rs` reaches rather than to anything of their own.
    let menu_proxy = event_loop.create_proxy();
    let _menu_bar = review_shell_macos::install_menu_bar(
        review_shell_macos::About {
            product: APP_NAME,
            version: env!("REVIEW_VERSION"),
            copyright: env!("REVIEW_COPYRIGHT"),
        },
        move |command| {
            let _ = menu_proxy.send_event(UserEvent::MenuCommand(command));
        },
    );

    let mut app = App {
        gpu_bring_up: Some(gpu_bring_up),
        initial_model,
        gate: gate_out.map(Gate::new),
        textures: TextureSubsystem {
            proxy: Some(texture_proxy),
            ..TextureSubsystem::default()
        },
        tracy_enabled,
        _tracy: tracy,
        ..App::default()
    };
    let outcome = event_loop
        .run_app(&mut app)
        .context("application event loop failed");

    // Report a startup failure now, from `main`'s own stack rather than from the
    // `resumed` callback that hit it: `rfd`'s message box runs a modal loop of its
    // own, and on macOS AppKit aborts the process rather than re-enter one from
    // inside an event callback (`mac-port-plan.md` D9). By here the event loop has
    // returned, so there is no loop to re-enter.
    if let Some(error) = app.startup_error.take() {
        report_startup_failure(&error);
    }
    outcome
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    egui_ctx: Option<egui::Context>,
    egui_state: Option<egui_winit::State>,
    /// Our own sokol_gfx egui renderer: tessellates the chrome and paints it into the
    /// swapchain pass the scene composited into (`mac-port-plan.md` D3).
    egui_renderer: Option<EguiRenderer>,
    drag_mode: Option<DragMode>,
    /// Whether the in-progress drag started in the *right* half of the Opt
    /// workspace's split view. Fixed at press time so a drag that wanders across
    /// the divider keeps moving the camera it began with.
    drag_in_opt_right_view: bool,
    /// The right-button WASD/QE flycam: held movement keys plus the
    /// wheel-adjusted speed (`flycam.rs`). Held here rather than in the renderer
    /// because it is input state — the renderer only ever sees the resulting
    /// per-frame move (invariant 2).
    flycam: FlyCam,
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
    /// Source of every model revision handed to the renderer, for the source mesh
    /// and each processed Opt level alike. One shared counter because the
    /// renderer caches mesh buffers by revision alone: two different meshes that
    /// ever drew the same number would leave one of them stale on screen.
    model_revision_counter: u64,
    /// Generation of the newest model-load request (imports run on a worker
    /// thread — see `loading.rs`). A finished import carrying an older
    /// generation was superseded by a newer open or a Ctrl+N and is dropped,
    /// so a slow parse can never overwrite what the user asked for since.
    model_load_generation: u64,
    /// The Opt workspace's processing state. `None` until the user first opens
    /// the workspace — a session that never does pays nothing for it.
    opt: Option<opt::OptSubsystem>,
    /// Per-mesh-part triangle BVH over [`Self::scene_model`], used to occlude the
    /// bounding-box dimension labels against the *visible* mesh. Built lazily the
    /// first frame the labels need it (the bounding-box view is on) and reused
    /// across frames; a heavy per-model structure we don't pay for unless the
    /// feature is used. `None` until built; [`Self::occlusion_bvh_revision`] tracks
    /// which model it covers so it rebuilds when a new model loads.
    occlusion_bvh: Option<SceneBvh>,
    occlusion_bvh_revision: u64,
    /// The animation clock + the pose it evaluates for the renderer; its logic
    /// lives in `animation.rs`.
    animation: AnimationSubsystem,
    ui: UiState,
    /// Model to load once the window/renderer exist, taken from the command line
    /// (file association / `3d-review.exe <path>`). Consumed in `resumed`.
    initial_model: Option<PathBuf>,
    /// The D2 gate run, when `--gate-out` was passed: it measures startup, frame
    /// time and (from the outside, during its hold) memory, then exits. `None` for
    /// every normal launch, which is every launch that is not `scripts/gate.ps1`.
    gate: Option<Gate>,
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
    /// (mirrored from [`review_ui::UiOutput::material_edit_active`] after the egui
    /// pass), so
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
    /// Whether a native file dialog is currently up on its worker thread
    /// (`dialog.rs`, `mac-port-plan.md` D9). The dialogs no longer block the event
    /// loop, which is what makes it possible to ask for a second one while the
    /// first is on screen — this is what says no.
    dialog_open: bool,
    /// The failure that stopped `start` from bringing the viewer up, held until
    /// `run_app` has returned so the error dialog is opened from `main` rather than
    /// from inside a winit callback (D9 again: on macOS a modal run loop entered
    /// from a callback aborts the process).
    startup_error: Option<anyhow::Error>,
    /// The toast notification system (egui-notify). `app` owns it because it owns
    /// the egui frame and triggers the notifications (texture decode start/finish);
    /// the UI crate only provides the themed type. Shown once per frame in `render`.
    notifications: Notifications,
    /// Whether `--tracy` was passed: arms the GPU profiler (via
    /// `review_render::enable_tracy_gpu`) in `resumed`, and turns on sokol's per-frame
    /// resource counters.
    tracy_enabled: bool,
    /// A GPU fault (scene/egui render failure, device lost) has already been
    /// surfaced as a toast this session. Faults repeat every frame once the
    /// device is wedged, so the toast fires once instead of stacking forever.
    gpu_fault_notified: bool,
    /// The live Tracy client handle, held for the whole process so the profiler
    /// session stays up (dropping the last handle disconnects). `None` on a normal
    /// launch — the client is never started, so all instrumentation no-ops.
    _tracy: Option<tracy_client::Client>,
    /// The GPU bring-up started on its own thread from the first line of `main`
    /// (D8), taken and joined by `start` once the window exists.
    gpu_bring_up: Option<GpuBringUp>,
    /// The GPU device and the window's swapchain.
    ///
    /// **Last on purpose**: dropping it shuts sokol_gfx down, and every field above
    /// that owns a GPU handle must be dropped before that happens. (Each of those
    /// also guards its own `Drop` on `sg::isvalid()`, so the order is belt and
    /// braces rather than the only thing holding it up.)
    gpu: Option<Gpu>,
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

/// The application's display name: the window title, the caption on a startup
/// failure dialog, the macOS About panel and menu, and the per-user config folder
/// the window placement is saved under (D17).
///
/// From `product.json` via `build.rs`, not a literal, so the binary, the installer
/// and the `.app` bundle cannot disagree about what this program is called.
pub(crate) const APP_NAME: &str = env!("REVIEW_PRODUCT");

/// Minimum window inner size (logical points), so the chrome never collapses.
const MIN_WINDOW_WIDTH: f64 = 960.0;
const MIN_WINDOW_HEIGHT: f64 = 640.0;

/// Camera zoom per pixel of a right-button zoom-drag (pointer-down zooms in).
const DRAG_ZOOM_SENSITIVITY: f32 = 0.01;

/// Camera zoom per wheel notch for line-based scroll deltas (mice).
const WHEEL_LINE_ZOOM_STEP: f32 = 0.5;

/// Pixel-precise scroll (trackpads) divided by this to match one wheel notch.
const WHEEL_PIXELS_PER_ZOOM_STEP: f32 = 120.0;

/// Camera zoom per unit of trackpad pinch scale (`mac-port-plan.md` D16). A pinch
/// delta is a scale *fraction* — a comfortable two-finger spread accumulates to
/// roughly 1.0 over its length — so this is the zoom that whole gesture is worth,
/// not a per-notch step like the wheel's.
const PINCH_ZOOM_STEP: f32 = 4.0;

impl Default for App {
    fn default() -> Self {
        let scene_model = Arc::new(ModelData::default());
        let mut ui = UiState {
            stats: scene_model.stats,
            ..UiState::default()
        };
        ui.capabilities.app_version = env!("CARGO_PKG_VERSION").to_string();

        Self {
            window: None,
            renderer: None,
            egui_ctx: None,
            egui_state: None,
            egui_renderer: None,
            drag_mode: None,
            drag_in_opt_right_view: false,
            flycam: FlyCam::default(),
            last_pointer_position: None,
            last_primary_click: None,
            modifiers: ModifiersState::empty(),
            redraw: RedrawScheduler::default(),
            scene_model,
            scene_revision: 0,
            model_revision_counter: 0,
            model_load_generation: 0,
            opt: None,
            occlusion_bvh: None,
            animation: AnimationSubsystem::default(),
            // A sentinel distinct from the initial `scene_revision` (0) so the BVH
            // is treated as stale until first built.
            occlusion_bvh_revision: u64::MAX,
            ui,
            initial_model: None,
            gate: None,
            placement: PlacementTracker::default(),
            selection_flash: None,
            flashed_selection: Selection::None,
            undo: UndoStack::new(),
            drag_in_progress: false,
            frame_showing_selection: false,
            textures: TextureSubsystem::default(),
            dialog_open: false,
            startup_error: None,
            notifications: Notifications::new(),
            tracy_enabled: false,
            gpu_fault_notified: false,
            _tracy: None,
            gpu_bring_up: None,
            gpu: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Orbit,
    /// Turn the camera in place (the right-button drag Unity and Unreal bind).
    /// Also what arms the WASD/QE flycam — see `flycam.rs`.
    Look,
    Pan,
    Zoom,
}

/// The image extensions the texture pool accepts (the picker filter + the
/// drag-drop routing). Anything else dropped on the window is treated as a model.
const TEXTURE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "tga", "tif", "tiff", "psd", "bmp", "gif",
];

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
    /// Build the whole shell: window, renderer, GPU, egui, then the first frame.
    /// Split out of `resumed` (which cannot fail) so each phase propagates its failure
    /// to one place that can report it.
    ///
    /// Startup phase timing rides along as a sequence of Tracy zones (replacing the
    /// old `StartupTimer` laps): `phase` holds the current zone and is ended by
    /// dropping its guard before the next begins, so they read as adjacent spans
    /// under the "main" thread. All no-op unless `--tracy` started the client.
    /// Each phase's work lives in its own helper below.
    fn start(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let mut phase = prof::zone!("Window Create");
        let window = self.create_startup_window(event_loop)?;

        drop(phase.take());
        phase = prof::zone!("Renderer Init");
        let renderer = build_startup_renderer(&window);
        let egui_ctx = egui::Context::default();
        // Install fonts + visuals once: the style is derived purely from the
        // central theme tokens (no per-frame state), so it never needs re-syncing.
        init_style(&egui_ctx);

        drop(phase.take());
        phase = prof::zone!("GPU Attach");
        let (gpu, egui_renderer) = self.create_startup_gpu(&window)?;

        drop(phase.take());
        phase = prof::zone!("Shell Init");
        self.init_shell(event_loop, window, renderer, egui_ctx, gpu, egui_renderer);
        prof::msg("application shell started");

        drop(phase.take());
        phase = prof::zone!("Queue Initial Model Load");
        // Queue the file the launch asked for, now that the renderer exists.
        // Reuses the same path as drag-drop — the parse runs on a worker thread and
        // lands via `ModelLoaded`, so the first frame below paints the chrome
        // without waiting on it.
        //
        // Two sources, one slot, because only one model is ever resident: the
        // command line (a Windows file association, or a CLI arg on either OS) and
        // the macOS opens that arrived before this window existed (D14). A
        // launch-by-open on a Mac carries *no* argv, so in practice exactly one of
        // them is ever non-empty; when both are, the Finder open is the more
        // specific intent and wins. Draining is also what switches the hook from
        // queueing to delivering, so it must happen even when nothing is queued.
        let opened_from_os = review_shell_macos::take_pending_opens();
        let initial = opened_from_os
            .into_iter()
            .next_back()
            .or_else(|| self.initial_model.take());
        if let Some(path) = initial {
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
        Ok(())
    }

    /// Startup phase 1: create the application window, honoring the OS maximize
    /// hint and restoring the persisted placement (dropped when it lands on a
    /// disconnected monitor, so the window can't open off-screen).
    fn create_startup_window(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> anyhow::Result<Arc<Window>> {
        // Honor the OS launch hint (e.g. a shortcut set to "Run: Maximized").
        // winit never consults `STARTUPINFO.wShowWindow`, so we query it and set
        // the initial state ourselves.
        self.placement.start_maximized = review_import::startup_show_maximized();

        // A gate run opens at a fixed size and ignores the saved placement: the two
        // builds have to render the same number of pixels for their frame times to
        // mean anything, and whatever this box's window happens to be is not that.
        let saved = (self.gate.is_none())
            .then(window_state::load)
            .flatten()
            .filter(|placement| placement_is_visible(event_loop, placement));
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
            .with_title(APP_NAME)
            .with_window_icon(load_window_icon())
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                MIN_WINDOW_WIDTH,
                MIN_WINDOW_HEIGHT,
            ))
            .with_maximized(maximized && self.gate.is_none());
        if self.gate.is_some() {
            attributes = attributes.with_inner_size(winit::dpi::PhysicalSize::new(
                gate::WINDOW_SIZE.0,
                gate::WINDOW_SIZE.1,
            ));
        }
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

        Ok(Arc::new(
            event_loop
                .create_window(attributes)
                .context("failed to create the application window")?,
        ))
    }

    /// Startup phase 3: join the GPU bring-up, put a swapchain on the window, and
    /// build the egui renderer.
    ///
    /// The device and `sg_setup` already happened on their own thread, started from
    /// the first line of `main` (D8); what is left here is the swapchain, which needs
    /// the window. It also kills the white startup flash by presenting one black frame
    /// before any scene exists — the ordinary swapchain pass with nothing drawn into
    /// it, not a special case (§3.2).
    ///
    /// Both failures are fatal and reported as such: a blocked or broken graphics
    /// driver leaves the viewer with nothing to draw through.
    fn create_startup_gpu(&mut self, window: &Window) -> anyhow::Result<(Gpu, EguiRenderer)> {
        let size = window.inner_size();
        let bring_up = self
            .gpu_bring_up
            .take()
            .context("the GPU bring-up was already consumed")?;
        let mut gpu = bring_up
            .attach(window, size.width, size.height)
            .context("failed to create the graphics device + swapchain")?;
        // A device lost at the very first present leaves a window that will never
        // paint; say so, since the frame loop's own fault toast only fires if a later
        // present fails too.
        if let Some(mut frame) = gpu.begin_frame() {
            frame.begin_swapchain_pass();
            if let review_render::PresentStatus::DeviceLost { reason } = frame.finish(false) {
                prof::msg(&format!(
                    "startup present failed: device lost ({reason:#x})"
                ));
                self.notifications.error(format!(
                    "Graphics device lost ({reason:#x}) while starting up - the viewport may stay blank; restart the viewer"
                ));
            }
        }
        let egui_renderer = EguiRenderer::new().context("failed to create the egui renderer")?;
        Ok((gpu, egui_renderer))
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
        egui_renderer: EguiRenderer,
    ) {
        // The viewer draws through sokol_gfx; the backend underneath it is the OS's.
        self.ui.capabilities.gpu_backend = if cfg!(windows) { "DX11" } else { "Metal" }.to_string();
        // Gate the Anti-Aliasing menu on the adapter's real MSAA support (the backend
        // leaf's `supported_sample_counts`, since sokol only reports MSAA as a yes/no
        // per format). IBL + AO stay enabled — every target the renderer supports has
        // the formats they need.
        let supported_counts = gpu.supported_msaa_counts();
        self.ui.capabilities.msaa_levels = MsaaSamples::ALL
            .into_iter()
            .filter(|level| supported_counts.contains(&level.sample_count()))
            .collect();

        // Under `--tracy`, arm the GPU profiler. A normal launch never calls this, so
        // nothing profiling-related is ever built.
        if self.tracy_enabled {
            review_render::enable_tracy_gpu();
        }

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            // The device's real limit, rather than the 16384 the D3D11 path assumed.
            Some(gpu.max_texture_size()),
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

    /// The next model revision, from the counter shared by the source mesh and
    /// every processed Opt level. The renderer keys its mesh-buffer cache on this
    /// number alone, so revisions must be unique across *all* models it is ever
    /// handed — not merely increasing within one of them.
    fn next_model_revision(&mut self) -> u64 {
        self.model_revision_counter = self.model_revision_counter.saturating_add(1);
        self.model_revision_counter
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

impl ApplicationHandler<UserEvent> for App {
    /// Handle a custom event from the texture file-watcher: re-decode + re-upload
    /// the changed texture (the disk-auto-reload path; redraw stays here).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TextureChanged(path) => self.reload_texture_file(&path),
            UserEvent::TextureDecoded(decode) => self.handle_texture_decoded(decode),
            UserEvent::OptProcessed(message) => self.handle_opt_processed(*message),
            UserEvent::OptExported(outcome) => self.handle_opt_exported(*outcome),
            UserEvent::ModelLoaded(message) => self.handle_model_loaded(*message),
            UserEvent::ModelLoadProgress(message) => self.handle_model_load_progress(message),
            UserEvent::ModelMeasured(message) => self.handle_model_measured(*message),
            UserEvent::DialogDone(answer) => self.handle_dialog_done(answer),
            UserEvent::OpenPath(path) => self.open_model_from_path(&path),
            UserEvent::MenuCommand(command) => self.handle_menu_command(command),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        };

        // A startup failure has no window to report itself in and, in a release
        // build (`windows_subsystem = "windows"`), no console either — so it is
        // held here and shown as a native dialog by `main`, once the loop has
        // returned. Opening it from inside this callback is exactly the re-entrant
        // modal D9 forbids.
        if let Err(error) = self.start(event_loop) {
            self.startup_error = Some(error);
            event_loop.exit();
        }
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
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The backend's business, and a no-op on Windows where the
                // swapchain's buffers *are* the window's pixels; on macOS it sets the
                // layer's `contentsScale`, without which a window dragged between
                // displays of different backing scale goes soft. A `Resized` follows,
                // which is what re-sizes the buffers.
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.set_scale_factor(scale_factor);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_input(state, button, egui_consumed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position, &window)
            }
            WindowEvent::CursorLeft { .. } => {
                self.drag_mode = None;
                self.flycam.release_all();
                self.last_pointer_position = None;
            }
            WindowEvent::MouseWheel { delta, .. } => self.handle_mouse_wheel(delta, egui_consumed),
            WindowEvent::PinchGesture { delta, .. } => {
                self.handle_pinch_gesture(delta, egui_consumed);
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
                    self.redraw.requested = true;
                }
                self.handle_keyboard_shortcut(&event);
            }
            WindowEvent::DroppedFile(path) => self.handle_dropped_file(path),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // A gate run drives itself: frames back to back until the stamp is written
        // (the pacer below would cap it at the refresh rate, which is the thing
        // being measured), then idle until the hold ends and the process exits.
        if self.gate_active() {
            if self.gate_should_exit() {
                event_loop.exit();
            } else if self.gate_wants_redraw() {
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + GATE_POLL));
            }
            return;
        }

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

/// How often the loop wakes during a gate run's idle hold, to notice the deadline.
const GATE_POLL: Duration = Duration::from_millis(100);

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
fn framing_safe_area(height_px: u32, scale_factor: f32) -> (f32, f32) {
    let logical_height = height_px as f32 / scale_factor.max(0.1);
    let chrome = review_ui::theme::chrome_height(scale_factor);
    let height_fraction = if logical_height > chrome {
        (logical_height - chrome) / logical_height
    } else {
        1.0
    };
    (1.0, height_fraction.clamp(0.4, 1.0))
}

/// Report a failure that stopped the viewer from starting, in the only channel a
/// shipped build has: a native modal dialog. A release build is a windows-subsystem
/// binary with no console, so a panic or a returned `Err` would end the process
/// with no diagnostic at all — the symptom being an icon that bounces and nothing
/// opening. The whole `anyhow` chain is shown (`{:#}`), so the dialog names both
/// the stage that failed and the underlying cause.
///
/// Called from `main` **after** `run_app` returns, never from a winit callback —
/// unlike the file dialogs in `dialog.rs` this one legitimately blocks, because at
/// that point there is nothing left for it to block.
fn report_startup_failure(error: &anyhow::Error) {
    let detail = format!("{error:#}");
    prof::msg(&format!("startup failed: {detail}"));
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(APP_NAME)
        .set_description(format!("{APP_NAME} couldn't start.\n\n{detail}"))
        .show();
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
