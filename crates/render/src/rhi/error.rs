//! The renderer's GPU error type — what every `rhi` call returns instead of the
//! backend's own.
//!
//! `windows::core::Error` used to travel all the way out of `Renderer::render_*`,
//! which made `render`'s public API name Direct3D. Nothing outside [`crate::rhi`]
//! sees a backend error now: the failure is turned into a [`GpuError`] at the
//! wrapper that produced it, and `app` only ever formats the `Display`.
//!
//! There are two shapes of failure to convert. sokol_gfx reports one by handing back
//! an id whose `sg_query_*_state` is not `Valid` (the *reason* goes to its logger,
//! which [`crate::rhi::Gpu`] installs), so [`require_valid`] is what every wrapper
//! ends in. The backend leaf still speaks COM `HRESULT`s, so the `windows`
//! conversions below stay — `cfg(windows)`, since they are that leaf's alone.
//!
//! The four variants are the four things a caller can actually distinguish:
//! creating a resource failed ([`GpuError::Resource`] — with the kind and a label
//! saying *which* one, since an `HRESULT` alone never did), the caller handed us
//! something unusable ([`GpuError::InvalidArg`]), the backend refused for a reason
//! of its own ([`GpuError::Backend`]), or the device is gone
//! ([`GpuError::DeviceLost`], which is terminal — every later frame fails too).

use sokol::gfx as sg;
use thiserror::Error;

/// The result type every `rhi` operation returns.
pub type GpuResult<T> = Result<T, GpuError>;

/// What kind of GPU resource failed to be created. Only ever used inside
/// [`GpuError::Resource`]; the point is that the message names the thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Device,
    Swapchain,
    Pipeline,
    Buffer,
    Texture,
    Target,
    Sampler,
    Query,
}

impl ResourceKind {
    /// The lowercase noun used in the error message.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Swapchain => "swapchain",
            Self::Pipeline => "pipeline",
            Self::Buffer => "buffer",
            Self::Texture => "texture",
            Self::Target => "render target",
            Self::Sampler => "sampler",
            Self::Query => "query",
        }
    }
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a GPU operation failed.
#[derive(Debug, Clone, Error)]
pub enum GpuError {
    /// A GPU resource could not be created. `label` names the specific one (e.g.
    /// `"scene color target"`), `detail` is the backend's own message.
    #[error("could not create the {kind} '{label}': {detail}")]
    Resource {
        kind: ResourceKind,
        label: String,
        detail: String,
    },
    /// The caller handed `rhi` something it cannot use — a payload larger than the
    /// buffer it is written to, a vertex layout that doesn't match the shader. A
    /// bug on our side, caught before the driver sees it.
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    /// The backend refused an operation for a reason of its own (a failed map, a
    /// failed present setup) that isn't a creation or a device loss.
    #[error("graphics backend error: {0}")]
    Backend(String),
    /// The device was removed or reset — a driver crash/TDR, a GPU hang, an adapter
    /// change. Terminal: the swapchain is dead and every later frame fails too, so
    /// the only recovery is a relaunch. `reason` is the driver's own removal code.
    #[error("the graphics device was lost ({reason:#x})")]
    DeviceLost { reason: i32 },
}

impl GpuError {
    /// An [`GpuError::InvalidArg`] — for `rhi`-side validation failures (bad payload
    /// sizes, mismatched layouts) that should surface as render errors rather than
    /// be fed to the driver.
    pub(crate) fn invalid_arg(message: impl Into<String>) -> Self {
        Self::InvalidArg(message.into())
    }
}

/// Turn "sokol handed back an invalid id" into a [`GpuError::Resource`] naming the
/// resource — the sokol-side twin of [`ResourceContext::resource`], and what every
/// `sg::make_*` wrapper ends in.
///
/// sokol reports *why* through its logger rather than through a return value, so the
/// message here can only name the resource; [`crate::rhi::Gpu`] installs a logger at
/// setup precisely so the reason is not lost.
pub(crate) fn require_valid(
    state: sg::ResourceState,
    kind: ResourceKind,
    label: &str,
) -> GpuResult<()> {
    if state == sg::ResourceState::Valid {
        return Ok(());
    }
    Err(GpuError::Resource {
        kind,
        label: label.to_owned(),
        detail: format!("sokol_gfx reports the resource as {state:?}"),
    })
}

#[cfg(windows)]
impl From<windows::core::Error> for GpuError {
    /// The default conversion, so an internal `?` on a COM call inside `rhi` still
    /// just works. Constructors upgrade this to a [`GpuError::Resource`] naming the
    /// resource via [`ResourceContext::resource`], which is the more useful message
    /// — this catch-all covers everything else.
    fn from(error: windows::core::Error) -> Self {
        Self::Backend(backend_message(&error))
    }
}

/// Attach "which resource this was" to a failing COM call.
///
/// The `windows` crate gives back an `HRESULT` and a driver string, neither of
/// which says whether the thing that failed was a 4K texture or a 96-byte cbuffer.
/// Every `rhi` constructor ends in `.resource(ResourceKind::…, "…")`, which is the
/// one place that context exists.
#[cfg(windows)]
pub(crate) trait ResourceContext<T> {
    fn resource(self, kind: ResourceKind, label: &str) -> GpuResult<T>;
}

#[cfg(windows)]
impl<T> ResourceContext<T> for windows::core::Result<T> {
    fn resource(self, kind: ResourceKind, label: &str) -> GpuResult<T> {
        self.map_err(|error| GpuError::Resource {
            kind,
            label: label.to_owned(),
            detail: backend_message(&error),
        })
    }
}

/// The backend's message plus its raw code — the code is what a driver bug report
/// is looked up by, and the message alone often omits it.
#[cfg(windows)]
fn backend_message(error: &windows::core::Error) -> String {
    let message = error.message();
    let code = error.code().0;
    if message.is_empty() {
        format!("{code:#010x}")
    } else {
        format!("{message} ({code:#010x})")
    }
}
