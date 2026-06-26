//! A `#[global_allocator]` wrapper that streams allocations to Tracy — but only
//! while the Tracy client is actually running (i.e. after `--tracy` started it).
//!
//! Unlike `tracy_client::ProfiledAllocator`, this **never starts** the client:
//! that allocator calls `Client::start()` on its first allocation (which happens
//! before `main`), which would force Tracy on for *every* launch and defeat the
//! `--tracy`-only gate. Here each hook checks `Client::running()` and no-ops when
//! the client isn't up, so a normal launch opens no socket and reports nothing;
//! the per-alloc cost on that path is a single relaxed atomic load.
//!
//! The `unsafe` `GlobalAlloc` impl and the raw `sys` memory-event FFI live here in
//! `crates/import` — the sanctioned home for all `unsafe`/FFI (invariant 9). `app`
//! only *declares* the `#[global_allocator]` static (safe code) over this type.

use std::alloc::{GlobalAlloc, Layout};

use tracy_client::Client;
use tracy_client::sys;

/// Wraps an inner allocator `A`, emitting Tracy alloc/free events for each
/// allocation while (and only while) the Tracy client is running.
pub struct TracyAllocator<A> {
    inner: A,
}

impl<A> TracyAllocator<A> {
    /// Wrap `inner`. `const` so it can initialize a `#[global_allocator]` static.
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

// SAFETY: every method forwards to the inner allocator with identical arguments
// and return values; the only added work is emitting a Tracy memory event for the
// exact pointer/size the inner allocator just (de)allocated, gated on a running
// client. The `secure = 1` argument matches `tracy_client::ProfiledAllocator`
// (serialized memory events, safe for a multi-threaded global allocator).
unsafe impl<A: GlobalAlloc> GlobalAlloc for TracyAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() && Client::running().is_some() {
            unsafe { sys::___tracy_emit_memory_alloc(ptr.cast(), layout.size(), 1) };
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !ptr.is_null() && Client::running().is_some() {
            unsafe { sys::___tracy_emit_memory_free(ptr.cast(), 1) };
        }
        unsafe { self.inner.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() && Client::running().is_some() {
            unsafe { sys::___tracy_emit_memory_alloc(ptr.cast(), layout.size(), 1) };
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Snapshot once so a free/alloc pair is reported consistently even if the
        // client's running state were to flip mid-call.
        let running = Client::running().is_some();
        if !ptr.is_null() && running {
            unsafe { sys::___tracy_emit_memory_free(ptr.cast(), 1) };
        }
        let new_ptr = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && running {
            unsafe { sys::___tracy_emit_memory_alloc(new_ptr.cast(), new_size, 1) };
        }
        new_ptr
    }
}
