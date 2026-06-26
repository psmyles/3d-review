//! Guarded Tracy zone helper. `tracy_client`'s `span!` macro panics when no client
//! is running, so we can't call it directly on a normal (non-`--tracy`) launch.
//! `zone!` wraps it: it yields `Some(Span)` only while the client is up and `None`
//! otherwise, so holding the returned guard (`let _z = zone!("…");`) times the
//! enclosing scope when profiling and costs a single atomic load when not.

/// Open a Tracy zone named by a string literal, scoped to the returned guard.
/// Evaluates to `Option<tracy_client::Span>` (`None` when no client is running).
macro_rules! zone {
    ($name:expr) => {
        ::tracy_client::Client::running()
            .map(|client| client.span(::tracy_client::span_location!($name), 0))
    };
}

pub(crate) use zone;
