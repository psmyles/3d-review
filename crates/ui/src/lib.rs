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
//! - `notifications` — the egui-notify toast system (`app`-owned, themed here).
//! - `toolbar` / `status_bar` / `stats` / `gizmo` / `panels` — the chrome.

// The UI is plain values + intents (invariant 2) and fully safe (invariant 9):
// no `unsafe` may ever land in this crate.
#![forbid(unsafe_code)]

mod assets;
mod dimensions;
mod gizmo;
mod help;
mod notifications;
mod overlay;
mod panels;
mod prof;
mod state;
mod stats;
mod status_bar;
mod texture_view;
pub mod theme;
mod toolbar;
mod units;
mod widgets;

// Root re-exports carry exactly what `app` (the sole consumer) uses; everything
// else stays reachable under its own module path.
pub use notifications::Notifications;
pub use overlay::draw_overlay;
pub use review_render::{MsaaSamples, Selection};
pub use state::{
    AxisGizmoAction, TexViewRequest, TextureBackground, TextureIntent, TexturePoolEntry,
    TextureSlotRef, UiOutput, UiState, WorkspaceMode,
};
pub use theme::init_style;
