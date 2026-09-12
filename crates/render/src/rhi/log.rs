//! sokol_gfx's logger callback.
//!
//! Without it a validation failure is silent, which is the worst way to debug a
//! GPU problem. Note that sokol's validation layer is compiled *out* of a release
//! build, so a misbehaving call has to be reproduced in debug to say so.

use super::*;

/// sokol_gfx's log/validation channel.
///
/// Everything sokol has to say about a resource that came back invalid, a pass that
/// was mis-configured or a binding that was left unbound arrives here and nowhere
/// else, so it goes to stderr *and* to the Tracy message channel — a windowed release
/// build has no console, and `--tracy` is the only place a running session can be
/// watched. Level 0 is sokol's own "panic": the library cannot continue, so neither
/// do we.
pub(super) extern "C" fn log_sokol(
    tag: *const c_char,
    level: u32,
    item: u32,
    message: *const c_char,
    line: u32,
    file: *const c_char,
    _user_data: *mut c_void,
) {
    // SAFETY: sokol passes NUL-terminated C string literals from its own static
    // storage, or null. `from_ptr` is only reached for a non-null pointer, and the
    // borrow ends inside this call.
    let text = |ptr: *const c_char| -> &str {
        if ptr.is_null() {
            ""
        } else {
            unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
        }
    };
    let severity = match level {
        0 => "panic",
        1 => "error",
        2 => "warning",
        _ => "info",
    };
    let line = format!(
        "{}: {severity} [id {item}] {} ({}:{line})",
        text(tag),
        text(message),
        text(file),
    );
    eprintln!("{line}");
    gpu_profiler::note(&line);
    if level == 0 {
        panic!("{line}");
    }
}
