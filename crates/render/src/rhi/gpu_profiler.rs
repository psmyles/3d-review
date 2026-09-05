//! The `--tracy` GPU profiling arming flag and the Tracy message channel.
//!
//! The timestamp-query machinery this used to hold — the `Zone` enum, the per-frame
//! query ring, the Tracy GPU context — is D3D11 code that has not been ported yet;
//! it is in `src/port_pending/gpu_profiler_zones.rs` and returns as a per-OS leaf in
//! Phase 1 step 6 (`mac-port-plan.md` D18). What stays here is what the rest of the
//! renderer reads *outside* a pass, and it is unchanged by the port: whether
//! profiling was armed, and how to say something to a watching Tracy session.

use std::sync::atomic::{AtomicBool, Ordering};

use tracy_client::Client;

/// Set by `app` on a `--tracy` launch. Read by the renderer, which has no channel
/// from `app`, to decide whether to build profiling state.
static TRACY_GPU_ENABLED: AtomicBool = AtomicBool::new(false);

/// Arm GPU profiling. Called once from `app` on a `--tracy` launch; never called on
/// a normal one, so nothing profiling-related is ever built.
pub fn enable_tracy_gpu() {
    TRACY_GPU_ENABLED.store(true, Ordering::Relaxed);
}

/// Whether to build profiling state: it was armed *and* a Tracy client is actually
/// running, so a `--tracy` launch with no connected server still pays nothing.
pub(crate) fn should_enable() -> bool {
    TRACY_GPU_ENABLED.load(Ordering::Relaxed) && Client::running().is_some()
}

/// Send a plain message to the running Tracy client (a no-op without one) — the only
/// diagnostics channel a `--tracy` session watches, and where sokol_gfx's validation
/// output goes alongside stderr.
pub(crate) fn note(text: &str) {
    if let Some(client) = Client::running() {
        client.message(text, 0);
    }
}
