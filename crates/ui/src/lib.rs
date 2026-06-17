//! review-ui: the egui overlay for the viewer — toolbar, option panels, axis
//! gizmo, stats overlay and status bar.
//!
//! Per invariant 2 this crate holds plain values + displayed stats and emits
//! [`UiOutput`] *intents*; it never owns or mutates renderer/model internals.
//! Per invariant 8 every visual value comes from the central [`theme`] module.
//!
//! Module map:
//! - [`theme`] — semantic color / size / font tokens + egui visuals.
//! - [`state`] — [`UiState`], per-tool panel state, the public enums, intents.
//! - [`assets`] — embedded icons + fonts and lazy texture loading.
//! - [`widgets`] — reusable theme-driven primitives.
//! - [`overlay`] — per-frame orchestration (the public entry points).
//! - `toolbar` / `status_bar` / `stats` / `gizmo` / `panels` — the chrome.

mod assets;
mod gizmo;
mod help;
mod overlay;
mod panels;
mod state;
mod stats;
mod status_bar;
pub mod theme;
mod toolbar;
mod widgets;

pub use assets::install_fonts;
pub use overlay::{draw_overlay, draw_startup_fade, draw_viewport_scene};
pub use review_render::{
    AntiAliasing, BloomSettings, EnvironmentMap, EnvironmentSettings, MsaaSamples, ibl_supported,
    supported_msaa_levels,
};
pub use state::{
    AxisGizmoAction, BoundingBoxPanelState, NormalPanelState, OptionPanel, UiOutput, UiState,
    UvCheckerPanelState, VertexColorPanelState, ViewAxis, ViewProjectionMode, WireframePanelState,
    WorkspaceMode,
};
