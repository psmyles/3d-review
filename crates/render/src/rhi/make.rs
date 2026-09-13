//! Creating a sokol resource, validated, with the handle freed if it failed.
//!
//! ## Why this is its own module
//!
//! `sg_make_*` allocates a pool slot *before* it tries to create the resource, so
//! a failure hands back a live id in the `FAILED` state rather than nothing. The
//! slot goes back to the pool only when the handle is destroyed —
//! `sg_destroy_*` accepts a `FAILED` resource precisely so a caller can do that —
//! and the constructors here used to `require_valid(…)?` straight past it. Each
//! failed creation therefore consumed a slot for the life of the process, which
//! is worst exactly when it matters: a device under memory pressure fails
//! repeatedly, and every attempt made the next one likelier to fail for a second
//! reason.
//!
//! Wrapping it once, here, is what makes that structural: a constructor that
//! returns on failure gets the cleanup from the call it already makes, rather
//! than from a rule to remember at each of a dozen sites — which is how the leaks
//! arrived in the first place.
//!
//! These helpers free only the handle that failed, which is the one thing no
//! caller can do for itself — a `?` has already returned by then. A constructor
//! that has already made *other* resources still owns those, and there are two
//! patterns for it: build `Self` up front so one `Drop` covers everything already
//! made (the bake's `CubeTarget`, whose attachment loop was the largest leak
//! here; a handle these helpers freed is never the one `Drop` sees, since it
//! never reached `Self`), or unwind explicitly on the error arm (`DepthTarget`,
//! `Texture`, `StorageBuffer`, `Pipeline`).
//!
//! `ColorTarget` is the one constructor that still calls `sg::make_*` directly:
//! it validates *after* building `Self`, so its `Drop` is already the cleanup for
//! every handle including a failed one, and routing it through here would destroy
//! that handle twice.

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};

/// Create a buffer, or free the failed handle and report why.
pub(crate) fn buffer(desc: &sg::BufferDesc, label: &str) -> GpuResult<sg::Buffer> {
    let buffer = sg::make_buffer(desc);
    if let Err(error) = require_valid(sg::query_buffer_state(buffer), ResourceKind::Buffer, label) {
        sg::destroy_buffer(buffer);
        return Err(error);
    }
    Ok(buffer)
}

/// Create an image, or free the failed handle and report why.
pub(crate) fn image(desc: &sg::ImageDesc, label: &str) -> GpuResult<sg::Image> {
    let image = sg::make_image(desc);
    if let Err(error) = require_valid(sg::query_image_state(image), ResourceKind::Texture, label) {
        sg::destroy_image(image);
        return Err(error);
    }
    Ok(image)
}

/// Create a view, or free the failed handle and report why.
///
/// `kind` is the thing the view is *of* — a storage-buffer view reports as a
/// buffer, an attachment or texture view as a texture — so the message names what
/// the caller was building rather than "view".
pub(crate) fn view(desc: &sg::ViewDesc, kind: ResourceKind, label: &str) -> GpuResult<sg::View> {
    let view = sg::make_view(desc);
    if let Err(error) = require_valid(sg::query_view_state(view), kind, label) {
        sg::destroy_view(view);
        return Err(error);
    }
    Ok(view)
}

/// Create a sampler, or free the failed handle and report why.
pub(crate) fn sampler(desc: &sg::SamplerDesc, label: &str) -> GpuResult<sg::Sampler> {
    let sampler = sg::make_sampler(desc);
    if let Err(error) = require_valid(
        sg::query_sampler_state(sampler),
        ResourceKind::Sampler,
        label,
    ) {
        sg::destroy_sampler(sampler);
        return Err(error);
    }
    Ok(sampler)
}

/// Create a shader, or free the failed handle and report why.
pub(crate) fn shader(desc: &sg::ShaderDesc, label: &str) -> GpuResult<sg::Shader> {
    let shader = sg::make_shader(desc);
    if let Err(error) = require_valid(
        sg::query_shader_state(shader),
        ResourceKind::Pipeline,
        label,
    ) {
        sg::destroy_shader(shader);
        return Err(error);
    }
    Ok(shader)
}

/// Create a pipeline, or free the failed handle and report why.
///
/// The shader is the caller's: a pipeline owns its shader in this crate, so the
/// error arm there destroys both.
pub(crate) fn pipeline(desc: &sg::PipelineDesc, label: &str) -> GpuResult<sg::Pipeline> {
    let pipeline = sg::make_pipeline(desc);
    if let Err(error) = require_valid(
        sg::query_pipeline_state(pipeline),
        ResourceKind::Pipeline,
        label,
    ) {
        sg::destroy_pipeline(pipeline);
        return Err(error);
    }
    Ok(pipeline)
}
