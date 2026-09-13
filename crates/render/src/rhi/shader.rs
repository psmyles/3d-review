//! Shader creation: shdc's generated reflection, with this host's precompiled
//! bytecode swapped in.
//!
//! Two halves have to meet. `build.rs` compiles the generated per-backend source to
//! bytecode — DXBC via `fxc` on Windows, a `.metallib` via `xcrun metal` on macOS —
//! so nothing compiles a shader at run time and a broken shader is a build error
//! (D4/D5). The generated `ShaderDesc` carries what bytecode cannot: the uniform
//! block layouts, which view and sampler slot is which, which sampler pairs with
//! which texture, and the per-backend entry-point names. [`make`] is the join.
//!
//! The generated desc points its funcs at the *source* it embedded, because that file
//! is checked in and must be byte-identical whichever OS regenerates it. Every use
//! replaces those with the bytecode; every binding the desc set stays as generated.

use sokol::gfx as sg;

use super::error::GpuResult;
use super::make;

/// One program's bytecode for this host, as `build.rs` produced it.
///
/// A pair rather than two arguments so a call site cannot silently swap the stages:
/// both are opaque blobs of the same type, and the compiler would not notice.
pub(crate) struct ShaderBytecode {
    pub(crate) vertex: &'static [u8],
    pub(crate) fragment: &'static [u8],
}

/// Build a shader from its generated reflection and this host's bytecode.
///
/// `desc_fn` is the generated `<program>_shader_desc` — passed as a function rather
/// than a filled desc so the backend query happens here, in the one place that knows
/// which bytecode it is about to substitute.
pub(crate) fn make(
    desc_fn: fn(sg::Backend) -> sg::ShaderDesc,
    bytecode: &ShaderBytecode,
    label: &'static std::ffi::CStr,
) -> GpuResult<sg::Shader> {
    let mut desc = desc_fn(sg::query_backend());
    desc.label = label.as_ptr();
    desc.vertex_func.source = std::ptr::null();
    desc.vertex_func.bytecode = sg::slice_as_range(bytecode.vertex);
    desc.fragment_func.source = std::ptr::null();
    desc.fragment_func.bytecode = sg::slice_as_range(bytecode.fragment);
    make::shader(&desc, label.to_str().unwrap_or("shader"))
}

/// The `.dxbc` / `.metallib` pair `build.rs` wrote for one program, by its shdc
/// program name.
///
/// A macro because `include_bytes!` needs a literal path: the per-host extension and
/// the `hlsl5` / `metal_macos` infix are `cfg`-chosen here, once, instead of at every
/// pipeline that wants a shader.
macro_rules! bytecode {
    ($program:literal) => {{
        #[cfg(windows)]
        const PAIR: $crate::rhi::shader::ShaderBytecode = $crate::rhi::shader::ShaderBytecode {
            vertex: include_bytes!(concat!(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/generated/review_"),
                $program,
                "_hlsl5_vertex.dxbc"
            )),
            fragment: include_bytes!(concat!(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/generated/review_"),
                $program,
                "_hlsl5_fragment.dxbc"
            )),
        };
        #[cfg(target_os = "macos")]
        const PAIR: $crate::rhi::shader::ShaderBytecode = $crate::rhi::shader::ShaderBytecode {
            vertex: include_bytes!(concat!(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/generated/review_"),
                $program,
                "_metal_macos_vertex.metallib"
            )),
            fragment: include_bytes!(concat!(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/generated/review_"),
                $program,
                "_metal_macos_fragment.metallib"
            )),
        };
        &PAIR
    }};
}

pub(crate) use bytecode;
