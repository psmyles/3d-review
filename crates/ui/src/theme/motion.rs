//! How the chrome *moves*: how long a transient state stays up, and how fast a
//! gesture drives the value it steers. Feel is as much a themed decision as color
//! or size (invariant 8), so these live here rather than beside the widget that
//! happens to consume them.

use std::time::Duration;

/// How long a transient result toast stays up before it dismisses itself.
pub const NOTIFICATION_EVENT: Duration = Duration::from_secs(2);

/// How long (seconds) a stats row acknowledges a copy. Long enough to read,
/// short enough that the row's explanation is back by the time the pointer
/// returns to it.
pub const STATS_COPIED_FEEDBACK_SECS: f64 = 1.2;

/// Tex-viewport zoom sensitivities: `ZOOM_SPEED` scales a scroll delta, and
/// `DRAG_ZOOM_SPEED` a right-drag vertical delta, into the exponent of the
/// multiplicative zoom step.
pub const TEXTURE_ZOOM_SPEED: f32 = 0.0015;
pub const TEXTURE_DRAG_ZOOM_SPEED: f32 = 0.01;
/// Duration (seconds) of the eased pan/zoom transition run when the Tex view
/// snaps to a target — the zoom-readout toggle (100% ↔ fit) and the `F` / `R`
/// frame reset. Continuous wheel/drag zoom is not eased.
pub const TEXTURE_ZOOM_ANIM_SECS: f32 = 0.1;
