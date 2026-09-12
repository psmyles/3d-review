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
//! - `notifications` — the notice column (`app`-owned, drawn + themed here).
//! - `toolbar` / `status_bar` / `stats` / `gizmo` / `panels` — the chrome.

// The UI is plain values + intents (invariant 2) and fully safe (invariant 9):
// no `unsafe` may ever land in this crate.
#![forbid(unsafe_code)]

/// The primary modifier's user-facing name, as a **literal** so it can be
/// `concat!`ed into the `const` shortcut tables that show it.
///
/// `app`'s `shortcuts::primary_held` is the dispatch side of the same decision
/// (`mac-port-plan.md` D10); this is only what the chrome calls the key. Written
/// as a macro rather than a `const` because most uses sit inside a longer const
/// string, and there is no const string concatenation.
#[cfg(target_os = "macos")]
macro_rules! primary_key {
    () => {
        "Cmd"
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! primary_key {
    () => {
        "Ctrl"
    };
}

/// The primary modifier's user-facing name for the places that build their label
/// at run time: `Ctrl` on Windows and Linux, `Cmd` on macOS.
pub(crate) const PRIMARY_MODIFIER: &str = primary_key!();

mod assets;
mod dimensions;
mod gizmo;
mod help;
mod notifications;
mod opt_state;
mod overlay;
mod panels;
mod prof;
mod state;
mod stats;
mod status_bar;
mod texture_view;
pub mod theme;
mod toolbar;
mod transport;
mod units;
mod widgets;

// Root re-exports carry exactly what `app` (the sole consumer) uses; everything
// else stays reachable under its own module path.
pub use notifications::{NoticeKind, Notifications};
pub use opt_state::{
    ComparisonSide, GhostStyle, OptIntent, OptLayout, OptLevelView, OptResultView, OptUiState,
    StackItem,
};
pub use overlay::{OptOverlayLevel, OptOverlayView, draw_overlay};
pub use review_render::{MsaaSamples, Selection};
pub use state::{
    AnimationUiState, AxisGizmoAction, ChromeInsets, PlaybackSpeed, TexViewRequest,
    TextureBackground, TextureIntent, TexturePoolEntry, TextureSlotRef, UiOutput, UiState,
    WorkspaceMode,
};
pub use theme::init_style;
