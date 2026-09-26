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

#![allow(
    unsafe_code,
    reason = "a GlobalAlloc wrapper, which is an unsafe trait by definition"
)]

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
// client. Memory events no longer take a `secure` flag: Tracy 0.14 dropped the
// unserialized variant, so `Profiler::MemAlloc`/`MemFree` always take the serial
// lock — which is the property a multi-threaded global allocator needs, and the
// one the old `secure = 1` argument used to ask for.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TracyAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards the caller's `layout` to the inner allocator unchanged;
        // the caller upholds `GlobalAlloc::alloc`'s contract on `layout`.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() && Client::running().is_some() {
            // SAFETY: `ptr` is the non-null block just returned and `layout.size()`
            // is its size; the Tracy C call only records the event for that block.
            unsafe { sys::___tracy_emit_memory_alloc(ptr.cast(), layout.size()) };
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !ptr.is_null() && Client::running().is_some() {
            // SAFETY: `ptr` is non-null and is the block being freed; the Tracy C
            // call only records the free, matching the alloc event for `ptr`.
            unsafe { sys::___tracy_emit_memory_free(ptr.cast()) };
        }
        // SAFETY: `ptr`/`layout` are the same pointer and layout the caller obtained
        // from this allocator, as required by `GlobalAlloc::dealloc`.
        unsafe { self.inner.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards the caller's `layout` to the inner allocator unchanged.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() && Client::running().is_some() {
            // SAFETY: as in `alloc` — records the event for the block just returned.
            unsafe { sys::___tracy_emit_memory_alloc(ptr.cast(), layout.size()) };
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Snapshot once so a free/alloc pair is reported consistently even if the
        // client's running state were to flip mid-call.
        let running = Client::running().is_some();
        if !ptr.is_null() && running {
            // SAFETY: `ptr` is the non-null block about to be reallocated; records
            // the free of the old block before the reallocation.
            unsafe { sys::___tracy_emit_memory_free(ptr.cast()) };
        }
        // SAFETY: `ptr`/`layout`/`new_size` satisfy `GlobalAlloc::realloc`'s contract
        // — they are the original pointer and layout the caller got from this
        // allocator and a valid new size.
        let new_ptr = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && running {
            // SAFETY: `new_ptr` is the non-null reallocated block of `new_size` bytes.
            unsafe { sys::___tracy_emit_memory_alloc(new_ptr.cast(), new_size) };
        }
        new_ptr
    }
}
