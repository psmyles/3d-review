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
//! - `help` — the in-app manual, over the generated [`docs`] page table.
//! - `notifications` — the notice column (`app`-owned, drawn + themed here).
//! - `toolbar` / `status_bar` / `stats` / `gizmo` / `panels` — the chrome.

// The UI is plain values + intents (invariant 2) and fully safe (invariant 9):
// no `unsafe` may ever land in this crate.
#![forbid(unsafe_code)]

/// The typed message keys, generated from `crates/l10n/locales/en/*.ftl` by this
/// crate's build script (invariant 12).
///
/// Private on purpose: consts in a library crate that nothing uses warn about
/// nothing, but private ones are `dead_code`, and `-D warnings` turns that into a
/// failed build. That is what stops the catalog filling up with messages no
/// screen shows.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

/// The manual's page table, generated from `docs/book/src` by this crate's build
/// script: one [`docs::Page`] per markdown file, its text, and the spans the Help
/// window resolves in it.
pub mod docs {
    include!(concat!(env!("OUT_DIR"), "/docs_pages.rs"));
}

/// The platform's own file-command modifier, as the chrome names it.
///
/// `app`'s `shortcuts::primary_held` is the dispatch side of the same decision
/// (`mac-port-plan.md` D10); this is only what the chrome calls the key. It is
/// resolved at run time and passed *into* a message as a variable, rather than
/// `concat!`ed into a const string as it used to be — a literal spliced between
/// two halves of a sentence is exactly what a translation cannot move.
///
/// `cfg!` rather than `#[cfg]` so both keys stay referenced on both platforms;
/// under `#[cfg]` the unused one would be `dead_code` and fail the build.
pub(crate) fn primary_modifier() -> std::borrow::Cow<'static, str> {
    if cfg!(target_os = "macos") {
        review_l10n::tr(keys::common::MODIFIER_CMD)
    } else {
        review_l10n::tr(keys::common::MODIFIER_CTRL)
    }
}

mod assets;
mod dimensions;
mod gizmo;
mod help;
mod labels;
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
pub use help::{HelpState, option_panel_at, workspace_help_page};
pub use labels::{buffer_view_name, material_mode_name};
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
