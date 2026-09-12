//! Font sizes (logical px). The proportional face is Inter, monospace is
//! JetBrains Mono; see [`crate::assets`].

pub const STATS: f32 = 12.0;
/// Clickable zoom-percentage readout in the Tex status bar (monospace).
pub const STATUS_ZOOM: f32 = 16.0;
/// Option-panel body text (labels, slider value boxes, combo text, the reset
/// button). Rendered in the monospace face one step smaller than egui's
/// default body size so numeric readouts line up on a fixed grid.
pub const PANEL_BODY: f32 = 12.0;
/// Bounding-box dimension-label text (monospace).
pub const DIMENSION_LABEL: f32 = 18.0;
pub const GIZMO_LABEL: f32 = 16.0;
pub const GIZMO_LABEL_NEG: f32 = 15.0;
pub const MODE_SEGMENT: f32 = 18.0;
/// Section-heading font in the Inspector side panel.
pub const PANEL_LABEL: f32 = 14.0;
/// egui `TextStyle::Heading` size — used by native window title bars and the
/// Inspector's `ui.heading` section headings. Trimmed from egui's stock 18 so
/// the option-window title bars are a touch shorter.
pub const PANEL_HEADING: f32 = 14.0;

/// Centered "nothing loaded yet" hint shown in an empty viewport (e.g. the Tex
/// view's "No textures loaded…" prompt). Larger than panel body text so it
/// reads clearly across the otherwise-empty canvas.
pub const VIEWPORT_EMPTY_HINT: f32 = 20.0;

// ── Startup help overlay (sized to match the app's other overlay text) ──
pub const HELP_TITLE: f32 = 14.0;
pub const HELP_META: f32 = 11.5;
pub const HELP_SUBTITLE: f32 = 12.0;
pub const HELP_BODY: f32 = 12.0;
pub const HELP_KEYCAP: f32 = 11.0;
/// Backtick key-cap glyph: bumped up because `` ` `` renders tiny and
/// top-aligned at the regular key-cap size.
pub const HELP_KEYCAP_BACKTICK: f32 = 18.0;
pub const HELP_FOOTER: f32 = 11.0;
