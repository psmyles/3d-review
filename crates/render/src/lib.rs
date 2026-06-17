use glam::{Mat4, Vec2, Vec3};
use review_model::Bounds;

mod geometry;
mod post;
mod scene;
mod targets;

pub use scene::{EGUI_MSAA_SAMPLE_COUNT, SCENE_DEPTH_FORMAT, SceneCallback};

const CAMERA_TRANSITION_SECONDS: f32 = 0.3;
/// Shorter transition used for the WASD 45° orbit steps, which fire repeatedly
/// and want a snappier response than the default framing/snap animation.
const ORBIT_TRANSITION_SECONDS: f32 = 0.1;
/// Uniform breathing room left around a framed fit (4%), on top of any
/// safe-area inset, so content never sits hard against the viewport edges.
const FRAME_MARGIN: f32 = 1.04;
/// Fraction of the safe area the empty "home" grid view fills. Below 1.0 so the
/// reference grid sits comfortably back in the viewport with margin around it,
/// rather than filling the window edge-to-edge. Only affects the home/reset
/// view — loaded models still frame tight to the safe area.
const HOME_FILL_FRACTION: f32 = 0.68;

/// Largest far/near ratio we let the projection produce. The depth buffer
/// (`Depth24Plus`) only has so many distinguishable values; a huge range spends
/// almost all of them in empty space in front of the model, leaving close and
/// intersecting faces to flicker / swap draw order as you zoom. Bounding the
/// ratio keeps enough precision across the model. ~5000:1 is comfortable for a
/// 24-bit depth buffer.
const MAX_DEPTH_RATIO: f32 = 5_000.0;
/// Absolute floor for the near plane so it never collapses to zero.
const MIN_Z_NEAR: f32 = 0.01;
/// Half-extent of the static reference grid: a 2 m square floor (±1 m) ruled in
/// 10 cm cells. Shared with `geometry::scene_lines` and the home-view framing so
/// the grid's size is defined in exactly one place. World units are meters.
pub(crate) const GRID_HALF_EXTENT: f32 = 1.0;
/// Axis-aligned bounds of that flat grid, used to frame the empty "home" view so
/// the whole floor is visible on launch and on reset.
const GRID_BOUNDS: Bounds = Bounds {
    min: Vec3::new(-GRID_HALF_EXTENT, 0.0, -GRID_HALF_EXTENT),
    max: Vec3::new(GRID_HALF_EXTENT, 0.0, GRID_HALF_EXTENT),
};
/// Worst-case radius of the grid (its corner, ~1.41 m) with margin. The far
/// plane must reach it so the grid isn't clipped behind small models.
const GRID_FAR_RADIUS: f32 = GRID_HALF_EXTENT * 2.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShadingMode {
    /// Wireframe only: the filled surface is not drawn, just its edges.
    Wireframe,
    /// Filled faces showing the active material as flat emissive color (no light).
    Unlit,
    /// Filled faces lit and shaded with the active material color.
    #[default]
    Shaded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CameraProjection {
    #[default]
    Perspective,
    Orthographic,
}

/// Multisample level for the offscreen scene render (the geometry MSAA). Distinct
/// from egui's own framebuffer MSAA ([`EGUI_MSAA_SAMPLE_COUNT`], fixed): this is
/// the per-edge antialiasing of the 3D scene, chosen at runtime. `Off` renders
/// single-sample (no resolve); the rest render multisampled and resolve to a
/// single-sample texture the composite pass samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MsaaSamples {
    Off,
    X2,
    #[default]
    X4,
    X8,
    X16,
}

impl MsaaSamples {
    /// Every variant in ascending order, for building UI menus.
    pub const ALL: [MsaaSamples; 5] = [
        MsaaSamples::Off,
        MsaaSamples::X2,
        MsaaSamples::X4,
        MsaaSamples::X8,
        MsaaSamples::X16,
    ];

    /// The wgpu sample count this level maps to (`Off` = 1).
    pub fn sample_count(self) -> u32 {
        match self {
            MsaaSamples::Off => 1,
            MsaaSamples::X2 => 2,
            MsaaSamples::X4 => 4,
            MsaaSamples::X8 => 8,
            MsaaSamples::X16 => 16,
        }
    }
}

/// The viewer's antialiasing configuration: a master on/off plus the MSAA level
/// and an optional FXAA post-process pass. Read by [`SceneCallback`] to size the
/// offscreen targets / scene pipelines and to drive the composite shader.
///
/// `enabled` is the toolbar toggle (left-click): when off, the scene renders with
/// no antialiasing at all regardless of `msaa` / `fxaa`, but those settings are
/// retained so toggling back on restores them. The default (enabled, 4× MSAA,
/// FXAA off) matches the pre-Phase-2 fixed pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntiAliasing {
    pub enabled: bool,
    pub msaa: MsaaSamples,
    pub fxaa: bool,
}

impl Default for AntiAliasing {
    fn default() -> Self {
        Self {
            enabled: true,
            msaa: MsaaSamples::X4,
            fxaa: false,
        }
    }
}

impl AntiAliasing {
    /// The MSAA sample count to actually render at: the chosen level when AA is
    /// enabled, otherwise 1 (single-sample, no resolve).
    pub fn effective_sample_count(self) -> u32 {
        if self.enabled {
            self.msaa.sample_count()
        } else {
            1
        }
    }

    /// Whether the FXAA post pass should run: only when AA is enabled *and* FXAA
    /// is ticked.
    pub fn effective_fxaa(self) -> bool {
        self.enabled && self.fxaa
    }
}

/// The MSAA levels the active adapter can actually render the scene at, in
/// ascending order. Queried against both the HDR color and depth target formats
/// so a level is only offered when both support it (invariant 4: capability-gate,
/// never crash). `Off` (single-sample) is always included. The UI uses this to
/// drop unsupported entries from the antialiasing menu.
///
/// `Adapter::get_texture_format_features` reports the adapter's full
/// (adapter-specific) sample-count support regardless of which device features
/// are enabled. We only get those extra counts on the *device* when
/// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` is enabled (the app enables it iff
/// the adapter offers it). So unless that feature is present we clamp to the
/// WebGPU-guaranteed counts (1 and 4) for these render formats — otherwise we'd
/// offer a level whose pipeline build the device would reject.
pub fn supported_msaa_levels(adapter: &wgpu::Adapter) -> Vec<MsaaSamples> {
    let adapter_specific = adapter
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let color = adapter
        .get_texture_format_features(crate::targets::SCENE_HDR_FORMAT)
        .flags;
    let depth = adapter
        .get_texture_format_features(SCENE_DEPTH_FORMAT)
        .flags;
    MsaaSamples::ALL
        .into_iter()
        .filter(|level| {
            let count = level.sample_count();
            // Single-sample and the WebGPU-guaranteed 4× are always safe; any
            // other count requires the adapter to both report it and have the
            // adapter-specific feature enabled on the device.
            count == 1
                || count == 4
                || (adapter_specific
                    && color.sample_count_supported(count)
                    && depth.sample_count_supported(count))
        })
        .collect()
}

/// How the 2D UV viewport draws the model's UV layout. Mutually exclusive (the
/// toolbar's UV-shading group is a radio selection); the UV edges are always
/// drawn, the fill underneath them changes per mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UvShadingMode {
    /// UV edges over the reference grid, with no fill underneath (the default
    /// wire-only layout view).
    #[default]
    Wire,
    /// The UV islands filled with one solid shaded color, edges drawn on top.
    Shaded,
    /// Each UV island filled with its own unique color, edges drawn on top.
    Islands,
}

/// Which built-in checker texture the UV-checker view samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckerTexture {
    #[default]
    Greyscale,
    Color,
}

/// How the vertex-color view interprets the mesh's vertex-color attribute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VertexColorMode {
    /// Show the RGB channels only (alpha forced opaque).
    #[default]
    Rgb,
    /// Show the alpha channel as a 0..1 greyscale value.
    Alpha,
    /// Show RGB as color and alpha as surface opacity.
    RgbAlpha,
}

/// Which material the filled faces display. The choices are mutually exclusive
/// (the toolbar's material group is a radio selection). [`Source`] is the
/// model's imported material; the other two replace it for inspection and apply
/// in every filled-face mode (unlit / shaded).
///
/// [`Source`]: ActiveMaterial::Source
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveMaterial {
    /// The material as imported from the model.
    #[default]
    Source,
    /// A built-in UV checker pattern sampled through the model's UVs.
    UvChecker,
    /// The mesh's per-vertex color attribute.
    VertexColors,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDebugOptions {
    pub shading_mode: ShadingMode,
    /// Draw the wireframe edges on top of the filled surface. Independent of
    /// `shading_mode` (it is an overlay), so it combines with the unlit and
    /// shaded modes; it is also implied when `shading_mode` is
    /// [`ShadingMode::Wireframe`] (which draws the edges as the only geometry).
    pub wireframe_overlay: bool,
    /// Which material the filled faces show (source / UV checker / vertex colors).
    /// Mutually exclusive; applies in every filled-face mode.
    pub active_material: ActiveMaterial,
    pub uv_checker_texture: CheckerTexture,
    /// Which vertex-color channels the view shows when `active_material` is
    /// [`ActiveMaterial::VertexColors`].
    pub vertex_color_mode: VertexColorMode,
    /// Checker repeats across the 0..1 UV range; clamped to 1..=16 by the UI.
    pub uv_checker_tiling: u32,
    /// Which model UV set the checker view samples (0-based). Only meaningful
    /// when the model carries more than one UV set.
    pub uv_channel: u32,
    pub show_grid: bool,
    /// Whether the model's axis-aligned bounding box is drawn as a wireframe box.
    pub show_bounding_box: bool,
    pub face_normals: bool,
    pub vertex_normals: bool,
    pub face_normal_length: f32,
    pub vertex_normal_length: f32,
    pub face_normal_color: [f32; 4],
    pub vertex_normal_color: [f32; 4],
    /// Color of the wireframe lines (wireframe / shaded-wireframe modes), baked
    /// into the line vertex buffer and rebuilt when it changes.
    pub wireframe_color: [f32; 4],
    /// Color of the bounding-box edges, baked into its line buffer and rebuilt
    /// when it changes.
    pub bounding_box_color: [f32; 4],
}

impl Default for SceneDebugOptions {
    fn default() -> Self {
        Self {
            shading_mode: ShadingMode::Shaded,
            wireframe_overlay: false,
            active_material: ActiveMaterial::Source,
            uv_checker_texture: CheckerTexture::Greyscale,
            vertex_color_mode: VertexColorMode::Rgb,
            uv_checker_tiling: 4,
            uv_channel: 0,
            show_grid: true,
            show_bounding_box: false,
            face_normals: false,
            vertex_normals: false,
            face_normal_length: 0.03,
            vertex_normal_length: 0.03,
            face_normal_color: [1.0, 0.1, 0.1, 0.95],
            vertex_normal_color: [0.14, 0.92, 0.96, 0.95],
            wireframe_color: [0.6, 0.6, 0.6, 1.0],
            bounding_box_color: [1.0, 0.803_921_6, 0.250_980_4, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RendererConfig {
    pub preferred_backends: wgpu::Backends,
    pub clear_color: wgpu::Color,
}

impl Default for RendererConfig {
    fn default() -> Self {
        // On Windows, request DX12 only: adapter/device creation is ~100 ms
        // cheaper than bringing up the Vulkan loader + ICD (measured on an RTX
        // 4080), and DX12 is guaranteed on Windows 10+. Other platforms keep
        // Vulkan/Metal. This is the "prefer DX12 on Windows" decision in
        // CLAUDE.md §4, now enforced rather than left to adapter selection.
        #[cfg(windows)]
        let preferred_backends = wgpu::Backends::DX12;
        #[cfg(not(windows))]
        let preferred_backends = wgpu::Backends::VULKAN | wgpu::Backends::METAL;

        Self {
            preferred_backends,
            clear_color: wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }
    }
}

/// Default half-height (in UV units) of the UV viewport, so the unit square is
/// shown with comfortable margin around it. The visible vertical span is twice
/// this; `> 0.5` leaves the 0..1 square framed back from the edges.
const UV_DEFAULT_HALF_HEIGHT: f32 = 0.72;
/// Clamp range for the UV camera's half-height so zoom can't invert or run away.
const UV_MIN_HALF_HEIGHT: f32 = 0.02;
const UV_MAX_HALF_HEIGHT: f32 = 50.0;

/// A 2D pan/zoom camera for the UV viewport. Maps UV space (the 0..1 unit square
/// the model's UVs live in) to the screen with an aspect-corrected orthographic
/// projection, so the unit square always stays square regardless of window
/// shape. `center` is the UV point shown at the viewport center; `half_height`
/// is half the visible vertical span in UV units (smaller = zoomed in).
#[derive(Debug, Clone, Copy)]
pub struct UvCamera {
    pub center: Vec2,
    pub half_height: f32,
    pub aspect_ratio: f32,
}

impl Default for UvCamera {
    fn default() -> Self {
        Self {
            // Center on the middle of the 0..1 unit square.
            center: Vec2::new(0.5, 0.5),
            half_height: UV_DEFAULT_HALF_HEIGHT,
            aspect_ratio: 16.0 / 9.0,
        }
    }
}

impl UvCamera {
    /// Reset to the default framing, preserving the live aspect ratio.
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    fn half_width(self) -> f32 {
        self.half_height * self.aspect_ratio.max(0.1)
    }

    /// Pan the view by a pointer drag (pixels), keeping the grabbed UV point
    /// under the cursor: the content follows the drag direction.
    pub fn pan_screen_delta(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 {
            return;
        }
        let du = delta_pixels.x / viewport_size.x * (2.0 * self.half_width());
        let dv = delta_pixels.y / viewport_size.y * (2.0 * self.half_height);
        // Drag right (+x) shows lower-u content at center; drag down (+y, with v
        // up) shows higher-v content at center.
        self.center.x -= du;
        self.center.y += dv;
    }

    /// Zoom about the view center. Positive `amount` zooms in (matches the orbit
    /// camera's wheel/zoom-drag sign), shrinking the visible span.
    pub fn zoom(&mut self, amount: f32) {
        let scale = (1.0 - amount * 0.1).clamp(0.2, 5.0);
        self.half_height = (self.half_height * scale).clamp(UV_MIN_HALF_HEIGHT, UV_MAX_HALF_HEIGHT);
    }

    /// Aspect-corrected orthographic view-projection mapping UV-plane points
    /// `(u, v, 0)` to clip space, with v pointing up like a UV editor.
    pub fn view_projection(self) -> Mat4 {
        let half_w = self.half_width();
        Mat4::orthographic_rh(
            self.center.x - half_w,
            self.center.x + half_w,
            self.center.y - self.half_height,
            self.center.y + self.half_height,
            -1.0,
            1.0,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub aspect_ratio: f32,
    pub fov_y_radians: f32,
    /// Bounding-sphere radius of the framed content around `target`. The near /
    /// far planes are fit to this each frame (see `near_far`) instead of being
    /// stored, so depth precision stays optimal as `distance` changes on zoom.
    pub scene_radius: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // A neutral "review" home view: a front-right isometric angle, framed so
        // the whole 2 m reference grid is visible when the scene is empty.
        let base = Self {
            target: Vec3::ZERO,
            // +45° yaw parks the eye in the +X/+Y/+Z octant (front-right iso): +X
            // reads lower-right, +Z lower-left, both facing the viewer. (-45° is
            // the mirror image and shows the -X side instead.)
            yaw: 45.0_f32.to_radians(),
            pitch: -35.264_39_f32.to_radians(),
            // `distance` / `scene_radius` are recomputed by `framed_to_bounds`.
            distance: 7.5,
            aspect_ratio: 16.0 / 9.0,
            fov_y_radians: 50.0_f32.to_radians(),
            scene_radius: 1.0,
        };
        base.framed_to_bounds(GRID_BOUNDS, Vec2::ONE)
    }
}

impl OrbitCamera {
    pub fn reset(&mut self) {
        let aspect_ratio = self.aspect_ratio;
        *self = Self::default();
        self.aspect_ratio = aspect_ratio;
    }

    pub fn frame_bounds(&mut self, bounds: Bounds) {
        *self = self.framed_to_bounds(bounds, Vec2::ONE);
    }

    /// Frame the camera so `bounds` fills the viewport, tight and centred.
    ///
    /// `safe_area` is the fraction of the viewport (x = width, y = height) that
    /// framing should aim to fill — `Vec2::ONE` is the whole window. The 3D
    /// scene is painted full-window with the toolbar / status-bar chrome drawn
    /// *over* its top and bottom, so passing the visible fraction there keeps
    /// the model out from under the chrome.
    pub fn framed_to_bounds(mut self, bounds: Bounds, safe_area: Vec2) -> Self {
        let center = bounds.center();
        let half_size = bounds.size() * 0.5;
        let rotation = self.rotation();
        // Camera basis. `forward` points from the eye toward (and past) the
        // target, so a corner's depth from the eye is `distance + forward·c`.
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let forward = self.forward_dir();
        // Loosen the usable FOV by the safe-area fractions and the uniform
        // margin, so the silhouette is fit *inside* the visible band rather than
        // the full window.
        let safe_h = (safe_area.x / FRAME_MARGIN).clamp(0.05, 1.0);
        let safe_v = (safe_area.y / FRAME_MARGIN).clamp(0.05, 1.0);
        let half_fov = (self.fov_y_radians * 0.5).clamp(0.01, 1.5).tan();
        let tan_v = half_fov * safe_v;
        let tan_h = half_fov * self.aspect_ratio.max(0.1) * safe_h;

        // Per-corner offsets from the box centre projected onto the camera
        // basis: `(u, v, w)` = (right·c, up·c, forward·c). `w` (depth along
        // forward) is unaffected by a lateral re-centring pan, so it's computed
        // once; `u`/`v` shift uniformly as the target pans.
        let mut corners = [(0.0_f32, 0.0_f32, 0.0_f32); 8];
        let mut i = 0;
        for cx in [-1.0_f32, 1.0] {
            for cy in [-1.0_f32, 1.0] {
                for cz in [-1.0_f32, 1.0] {
                    let c = Vec3::new(cx * half_size.x, cy * half_size.y, cz * half_size.z);
                    corners[i] = (right.dot(c), up.dot(c), forward.dot(c));
                    i += 1;
                }
            }
        }

        // Solve the fit distance *and* a lateral re-centring pan together. The
        // per-corner screen-fill constraint is
        //   |u| <= tan_h * (distance + w)   and   |v| <= tan_v * (distance + w)
        // so distance >= |u|/tan_h - w (and likewise for v). A pure box-centre
        // fit (pan = 0) makes the *nearest* extreme corner touch the edge while
        // the far corner leaves a gap — perspective magnifies near geometry, so
        // the silhouette is not actually centred. Each pass fits the smallest
        // distance for the current pan, then shifts the pan so the projected
        // silhouette straddles the centre. The depth term makes the re-centre
        // non-linear, so iterate to a fixed point (cheap: 8 corners, 6 passes).
        let mut pan_u = 0.0_f32;
        let mut pan_v = 0.0_f32;
        let mut distance = 0.05_f32;
        for _ in 0..6 {
            distance = 0.0;
            for &(u, v, w) in &corners {
                distance = distance
                    .max((u + pan_u).abs() / tan_h - w)
                    .max((v + pan_v).abs() / tan_v - w);
            }
            distance = distance.max(0.05);
            pan_u +=
                silhouette_recenter(corners.iter().map(|&(u, _, w)| (u + pan_u, distance + w)));
            pan_v +=
                silhouette_recenter(corners.iter().map(|&(_, v, w)| (v + pan_v, distance + w)));
        }

        // Pan the target laterally by the solved offset (eye follows, so depth
        // along forward is unchanged) to centre the projected silhouette.
        self.target = center - right * pan_u - up * pan_v;
        self.distance = distance;
        // Bounding-sphere radius around the box centre (corner distance). Drives
        // the per-frame near/far fit in `near_far`.
        self.scene_radius = half_size.length().max(0.001);
        self
    }

    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-1.5, 1.5);
    }

    pub fn set_offset_direction(&mut self, direction: Vec3) {
        *self = self.with_offset_direction(direction);
    }

    pub fn with_offset_direction(mut self, direction: Vec3) -> Self {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() <= f32::EPSILON {
            return self;
        }

        self.pitch = (-direction.y).asin().clamp(-1.5, 1.5);
        self.yaw = direction.x.atan2(direction.z);
        self
    }

    pub fn zoom(&mut self, amount: f32) {
        let scale = (1.0 - amount * 0.1).clamp(0.2, 5.0);
        self.distance = (self.distance * scale).max(0.05);
    }

    pub fn pan_screen_delta(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 {
            return;
        }

        let rotation = self.rotation();
        let right = rotation.transform_vector3(Vec3::X);
        let up = rotation.transform_vector3(Vec3::Y);
        let view_height = 2.0 * self.distance * (self.fov_y_radians * 0.5).tan();
        let view_width = view_height * self.aspect_ratio;
        let delta_x = delta_pixels.x / viewport_size.x * view_width;
        let delta_y = delta_pixels.y / viewport_size.y * view_height;

        self.target += (-right * delta_x) + (up * delta_y);
    }

    pub fn eye_position(self) -> Vec3 {
        self.target - self.forward_dir() * self.distance
    }

    pub fn forward_dir(self) -> Vec3 {
        self.rotation().transform_vector3(Vec3::NEG_Z)
    }

    pub fn view_space_direction(self, direction: Vec3) -> Vec3 {
        self.rotation().inverse().transform_vector3(direction)
    }

    pub fn view_matrix(self) -> Mat4 {
        let world_from_camera = Mat4::from_translation(self.eye_position()) * self.rotation();
        world_from_camera.inverse()
    }

    /// Near / far planes fit to the current view each frame. The far plane
    /// reaches past the content (model *and* the reference grid); the near plane
    /// is pushed as far forward as a bounded far/near ratio allows so the depth
    /// buffer keeps its precision across the model regardless of zoom. This is
    /// what prevents close / intersecting faces from flickering and swapping
    /// draw order — a fixed tiny near plane with a huge far plane does not.
    pub fn near_far(self) -> (f32, f32) {
        // Far must clear the grid even when the model is tiny.
        let content_radius = self.scene_radius.max(GRID_FAR_RADIUS);
        let z_far = (self.distance + content_radius).max(MIN_Z_NEAR * 2.0);
        let z_near = (z_far / MAX_DEPTH_RATIO).max(MIN_Z_NEAR);
        (z_near, z_far)
    }

    pub fn view_projection(self, projection_mode: CameraProjection) -> Mat4 {
        let view = self.view_matrix();
        let (z_near, z_far) = self.near_far();
        let projection = match projection_mode {
            CameraProjection::Perspective => {
                Mat4::perspective_rh(self.fov_y_radians, self.aspect_ratio, z_near, z_far)
            }
            CameraProjection::Orthographic => {
                let half_height = self.orthographic_half_height();
                let half_width = half_height * self.aspect_ratio.max(0.1);
                Mat4::orthographic_rh(
                    -half_width,
                    half_width,
                    -half_height,
                    half_height,
                    z_near,
                    z_far,
                )
            }
        };

        projection * view
    }

    fn orthographic_half_height(self) -> f32 {
        (self.distance * (self.fov_y_radians * 0.5).tan()).max(0.001)
    }

    fn rotation(self) -> Mat4 {
        Mat4::from_rotation_y(self.yaw) * Mat4::from_rotation_x(self.pitch)
    }
}

#[derive(Debug, Clone, Copy)]
struct CameraTransition {
    start: OrbitCamera,
    end: OrbitCamera,
    elapsed_seconds: f32,
    duration_seconds: f32,
}

impl CameraTransition {
    fn new(start: OrbitCamera, end: OrbitCamera) -> Self {
        Self::with_duration(start, end, CAMERA_TRANSITION_SECONDS)
    }

    fn with_duration(start: OrbitCamera, end: OrbitCamera, duration_seconds: f32) -> Self {
        Self {
            start,
            end,
            elapsed_seconds: 0.0,
            duration_seconds,
        }
    }

    fn step(&mut self, delta_seconds: f32) -> (OrbitCamera, bool) {
        self.elapsed_seconds = (self.elapsed_seconds + delta_seconds).min(self.duration_seconds);
        let t = if self.duration_seconds <= 0.0 {
            1.0
        } else {
            self.elapsed_seconds / self.duration_seconds
        };
        let eased = ease_in_out_cubic(t);
        let finished = self.elapsed_seconds >= self.duration_seconds;
        (lerp_camera(self.start, self.end, eased), finished)
    }
}

fn lerp_camera(start: OrbitCamera, end: OrbitCamera, t: f32) -> OrbitCamera {
    OrbitCamera {
        target: start.target.lerp(end.target, t),
        yaw: lerp_angle(start.yaw, end.yaw, t),
        pitch: start.pitch + (end.pitch - start.pitch) * t,
        distance: start.distance + (end.distance - start.distance) * t,
        aspect_ratio: end.aspect_ratio,
        fov_y_radians: start.fov_y_radians + (end.fov_y_radians - start.fov_y_radians) * t,
        scene_radius: start.scene_radius + (end.scene_radius - start.scene_radius) * t,
    }
}

fn lerp_angle(start: f32, end: f32, t: f32) -> f32 {
    let delta = (end - start + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    start + delta * t
}

/// Additional lateral pan that makes the two screen-space extreme corners on one
/// axis straddle the viewport centre symmetrically. Each item is `(lateral,
/// depth)` for a corner — `lateral` already includes the running pan, `depth` is
/// `distance + forward·offset`. Returns the extra pan to apply on that axis.
fn silhouette_recenter(corners: impl Iterator<Item = (f32, f32)>) -> f32 {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let mut lo_corner = (0.0_f32, 1.0_f32);
    let mut hi_corner = (0.0_f32, 1.0_f32);
    for (lateral, depth) in corners {
        let depth = depth.max(1e-3);
        let screen = lateral / depth;
        if screen < lo {
            lo = screen;
            lo_corner = (lateral, depth);
        }
        if screen > hi {
            hi = screen;
            hi_corner = (lateral, depth);
        }
    }
    let (lat_lo, depth_lo) = lo_corner;
    let (lat_hi, depth_hi) = hi_corner;
    // Solve Δ so (lat_lo + Δ)/depth_lo = -(lat_hi + Δ)/depth_hi, i.e. the two
    // extreme corners project to equal-and-opposite screen offsets.
    -(lat_lo * depth_hi + lat_hi * depth_lo) / (depth_lo + depth_hi)
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

#[derive(Debug)]
pub struct Renderer {
    pub config: RendererConfig,
    pub camera: OrbitCamera,
    /// The 2D camera for the UV viewport, independent of the 3D orbit camera.
    pub uv_camera: UvCamera,
    camera_transition: Option<CameraTransition>,
    /// Fraction of the viewport (x = width, y = height) framing should fill,
    /// leaving room for the chrome that overlays the full-window 3D scene. Set
    /// by `app` from the live window + chrome sizes; `Vec2::ONE` = whole window.
    framing_safe_area: Vec2,
}

impl Renderer {
    pub fn new(config: RendererConfig) -> Self {
        Self {
            config,
            camera: OrbitCamera::default(),
            uv_camera: UvCamera::default(),
            camera_transition: None,
            framing_safe_area: Vec2::ONE,
        }
    }

    pub fn set_uv_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.uv_camera.aspect_ratio = aspect_ratio;
    }

    pub fn pan_uv_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.uv_camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_uv_camera(&mut self, amount: f32) {
        self.uv_camera.zoom(amount);
    }

    pub fn reset_uv_camera(&mut self) {
        self.uv_camera.reset();
    }

    /// Set the fraction of the viewport that subsequent framing should fill, so
    /// the model lands inside the band left visible by the toolbar / status bar.
    pub fn set_framing_safe_area(&mut self, width_fraction: f32, height_fraction: f32) {
        self.framing_safe_area = Vec2::new(width_fraction, height_fraction);
    }

    pub fn orbit_camera(&mut self, delta: Vec2) {
        self.camera_transition = None;
        self.camera.orbit(delta);
    }

    pub fn set_camera_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.camera.aspect_ratio = aspect_ratio;
        if let Some(transition) = self.camera_transition.as_mut() {
            transition.start.aspect_ratio = aspect_ratio;
            transition.end.aspect_ratio = aspect_ratio;
        }
    }

    pub fn pan_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.camera_transition = None;
        self.camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_camera(&mut self, amount: f32) {
        self.camera_transition = None;
        self.camera.zoom(amount);
    }

    pub fn animate_camera_to(&mut self, end: OrbitCamera) {
        self.camera_transition = Some(CameraTransition::new(self.camera, end));
    }

    pub fn animate_camera_to_bounds(&mut self, bounds: Bounds) {
        self.animate_camera_to(self.camera.framed_to_bounds(bounds, self.framing_safe_area));
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
    }

    /// Animate a relative orbit by the given yaw / pitch deltas (radians). Based
    /// off any in-flight transition's target (not the mid-flight camera) so
    /// repeated key presses chain into successive 45° steps. Pitch is clamped to
    /// match interactive [`OrbitCamera::orbit`]. Uses the shorter
    /// [`ORBIT_TRANSITION_SECONDS`] so each step feels snappy.
    pub fn animate_orbit_by(&mut self, yaw_delta: f32, pitch_delta: f32) {
        let mut end = self.camera_transition.map_or(self.camera, |t| t.end);
        end.yaw += yaw_delta;
        end.pitch = (end.pitch + pitch_delta).clamp(-1.5, 1.5);
        self.camera_transition = Some(CameraTransition::with_duration(
            self.camera,
            end,
            ORBIT_TRANSITION_SECONDS,
        ));
    }

    /// The default "home" view, re-framed for the live aspect ratio and the
    /// chrome-aware safe area so the whole grid stays visible regardless of
    /// window shape. Shared by the animated reset and the instant startup frame
    /// so both land on exactly the same view.
    fn home_camera(&self) -> OrbitCamera {
        let home = OrbitCamera {
            aspect_ratio: self.camera.aspect_ratio,
            ..OrbitCamera::default()
        };
        // Fill only a fraction of the safe area so the grid sits back from the
        // edges (see HOME_FILL_FRACTION) instead of filling the window.
        home.framed_to_bounds(GRID_BOUNDS, self.framing_safe_area * HOME_FILL_FRACTION)
    }

    /// Animate back to the home view.
    pub fn animate_camera_to_home(&mut self) {
        self.animate_camera_to(self.home_camera());
    }

    /// Snap (no animation) to the home view. Used at startup once the real
    /// window size / safe area are known, so the initial frame matches the
    /// reset view rather than the full-window `OrbitCamera::default` framing.
    pub fn reset_camera_to_home(&mut self) {
        self.camera_transition = None;
        self.camera = self.home_camera();
    }

    pub fn update_camera_animation(&mut self, delta_seconds: f32) -> bool {
        let Some(transition) = self.camera_transition.as_mut() else {
            return false;
        };

        let (camera, finished) = transition.step(delta_seconds);
        self.camera = camera;
        if finished {
            self.camera_transition = None;
        }
        true
    }

    pub fn is_camera_animating(&self) -> bool {
        self.camera_transition.is_some()
    }
}
