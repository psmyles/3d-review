//! Guarded Tracy helpers, shared by every crate that instruments itself.
//!
//! Every `tracy_client` convenience macro (`span!`, `plot!`, `frame_mark()`,
//! `set_thread_name!`) panics when no client is running, so none can be called
//! directly on a normal (non-`--tracy`) launch. These wrappers check
//! `Client::running()` first and no-op when the client is down, so the
//! instrumentation is always compiled in but costs ~one atomic load when idle.
//!
//! The macros reach `tracy_client` through the `$crate` re-export below rather
//! than through `::tracy_client`, so a crate that only opens zones does not need
//! a tracy dependency of its own.

#![forbid(unsafe_code)]

pub use tracy_client;

/// Open a Tracy zone named by a string literal, scoped to the returned guard.
/// Evaluates to `Option<tracy_client::Span>` (`None` when no client is running),
/// so `let _z = zone!("…");` times the enclosing scope when profiling.
#[macro_export]
macro_rules! zone {
    ($name:expr) => {
        $crate::tracy_client::Client::running()
            .map(|client| client.span($crate::tracy_client::span_location!($name), 0))
    };
}

/// Record a value on a named Tracy plot (a string-literal name).
#[macro_export]
macro_rules! plot {
    ($name:expr, $value:expr) => {
        if let Some(client) = $crate::tracy_client::Client::running() {
            client.plot($crate::tracy_client::plot_name!($name), $value as f64);
        }
    };
}

/// Mark the end of a frame for Tracy's frame view.
pub fn frame_mark() {
    if let Some(client) = tracy_client::Client::running() {
        client.frame_mark();
    }
}

/// Name the current thread in the Tracy timeline.
pub fn thread_name(name: &str) {
    if let Some(client) = tracy_client::Client::running() {
        client.set_thread_name(name);
    }
}

/// Emit a one-off Tracy message. No-ops when the client isn't running, so a
/// normal launch logs nothing.
pub fn msg(text: &str) {
    if let Some(client) = tracy_client::Client::running() {
        client.message(text, 0);
    }
}
