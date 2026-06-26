//! Guarded Tracy helpers. Every `tracy_client` convenience macro (`span!`,
//! `plot!`, `frame_mark()`, `set_thread_name!`) panics when no client is running,
//! so none can be called directly on a normal (non-`--tracy`) launch. These
//! wrappers check `Client::running()` first and no-op when the client is down, so
//! the instrumentation is always compiled in but costs ~one atomic load when idle.

/// Open a Tracy zone named by a string literal, scoped to the returned guard.
/// Evaluates to `Option<tracy_client::Span>` (`None` when no client is running).
macro_rules! zone {
    ($name:expr) => {
        ::tracy_client::Client::running()
            .map(|client| client.span(::tracy_client::span_location!($name), 0))
    };
}

/// Record a value on a named Tracy plot (a string-literal name).
macro_rules! plot {
    ($name:expr, $value:expr) => {
        if let Some(client) = ::tracy_client::Client::running() {
            client.plot(::tracy_client::plot_name!($name), $value as f64);
        }
    };
}

pub(crate) use plot;
pub(crate) use zone;

/// Mark the end of a frame for Tracy's frame view.
pub(crate) fn frame_mark() {
    if let Some(client) = tracy_client::Client::running() {
        client.frame_mark();
    }
}

/// Name the current thread in the Tracy timeline.
pub(crate) fn thread_name(name: &str) {
    if let Some(client) = tracy_client::Client::running() {
        client.set_thread_name(name);
    }
}

/// Emit a one-off Tracy message (replaces the old `tracing` log lines). No-ops
/// when the client isn't running, so a normal launch logs nothing.
pub(crate) fn msg(text: &str) {
    if let Some(client) = tracy_client::Client::running() {
        client.message(text, 0);
    }
}
