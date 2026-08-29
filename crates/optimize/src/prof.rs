//! Guarded Tracy zone helper — the same shape as `crates/import/src/prof.rs`.
//! `tracy_client`'s `span!` panics when no client is running, so it can't be
//! called directly on a normal (non-`--tracy`) launch. `zone!` yields
//! `Some(Span)` only while the client is up, so `let _z = zone!("…");` times the
//! enclosing scope when profiling and costs one atomic load when not.

/// Open a Tracy zone named by a string literal, scoped to the returned guard.
/// Evaluates to `Option<tracy_client::Span>` (`None` when no client is running).
macro_rules! zone {
    ($name:expr) => {
        ::tracy_client::Client::running()
            .map(|client| client.span(::tracy_client::span_location!($name), 0))
    };
}

pub(crate) use zone;
