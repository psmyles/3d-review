//! The offscreen attachments a frame draws into, and the backbuffer rect the
//! composite lands in.
//!
//! A [`TargetSet`] is the seven targets one view needs. The Opt split holds
//! *two* of them, each sized to half the viewport: the composite is a deferred
//! swapchain job, so both halves' passes have run before either composite does,
//! and one shared set would show the second view in both.

use crate::rhi::{ColorTarget, DepthTarget, Frame, GpuResult};

use super::pipelines::build_scene_pipelines;

use super::gpu::SceneGpu;

/// A rectangle of the backbuffer for the composite to write into: the whole thing
/// for a single view, one half for each side of the Opt workspace's split.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackbufferRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl BackbufferRect {
    pub(crate) fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// Everything one view renders through: the 2-MRT scene attachments with their depth,
/// and GTAO's own single-sample targets.
///
/// They live together because they are always sized together and always belong to one
/// view. The Opt split holds two of these — at half width each, so the pair costs what
/// one full-width set would — because its composites are deferred and both halves'
/// results have to still be there when they run.
pub(super) struct TargetSet {
    /// Offscreen linear-HDR attachments: location 0 scene colour, location 1 ambient.
    pub(super) color: ColorTarget,
    pub(super) ambient: ColorTarget,
    pub(super) depth: DepthTarget,
    /// GTAO's targets, all **single-sample**: the view-normal/Z G-buffer (HDR) with
    /// its own depth, then the raw and blurred occlusion (`R8`).
    pub(super) gtao_gbuffer: ColorTarget,
    pub(super) gtao_depth: DepthTarget,
    pub(super) gtao_raw: ColorTarget,
    pub(super) gtao_blur: ColorTarget,
}

impl TargetSet {
    pub(super) fn new(width: u32, height: u32, sample_count: u32) -> GpuResult<Self> {
        let (width, height) = (width.max(1), height.max(1));
        Ok(Self {
            color: ColorTarget::hdr(width, height, sample_count, c"scene colour")?,
            ambient: ColorTarget::hdr(width, height, sample_count, c"scene ambient")?,
            depth: DepthTarget::new(width, height, sample_count, c"scene depth")?,
            gtao_gbuffer: ColorTarget::hdr(width, height, 1, c"gtao gbuffer")?,
            gtao_depth: DepthTarget::new(width, height, 1, c"gtao depth")?,
            gtao_raw: ColorTarget::r8(width, height, c"gtao raw")?,
            gtao_blur: ColorTarget::r8(width, height, c"gtao blur")?,
        })
    }

    /// Recreate the set when the render size or the MSAA level moved. Steady-state
    /// frames allocate nothing.
    pub(super) fn sync(&mut self, size: (u32, u32), sample_count: u32) -> GpuResult<()> {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let size_changed = self.color.size() != (width, height);
        let samples_changed = self.color.sample_count() != sample_count;
        if !size_changed && !samples_changed {
            return Ok(());
        }
        self.color = ColorTarget::hdr(width, height, sample_count, c"scene colour")?;
        self.ambient = ColorTarget::hdr(width, height, sample_count, c"scene ambient")?;
        self.depth = DepthTarget::new(width, height, sample_count, c"scene depth")?;
        // GTAO's targets are single-sample by design (its own mesh-only pass, never
        // resolved), so only a size change touches them.
        if size_changed {
            self.gtao_gbuffer = ColorTarget::hdr(width, height, 1, c"gtao gbuffer")?;
            self.gtao_depth = DepthTarget::new(width, height, 1, c"gtao depth")?;
            self.gtao_raw = ColorTarget::r8(width, height, c"gtao raw")?;
            self.gtao_blur = ColorTarget::r8(width, height, c"gtao blur")?;
        }
        Ok(())
    }
}

impl SceneGpu {
    /// Reconcile the offscreen targets and the scene pipelines with the size the
    /// scene renders at and the live MSAA level. Steady-state frames allocate
    /// nothing.
    ///
    /// `size` is not always the backbuffer's: the Opt split renders each half at half
    /// width so the composite maps its target onto its half of the backbuffer
    /// one-to-one instead of squashing a full-width image into it.
    pub(super) fn sync_targets(
        &mut self,
        frame: &Frame<'_>,
        size: (u32, u32),
        sample_count: u32,
    ) -> GpuResult<()> {
        let sample_count = self.sync_sample_count(frame, sample_count)?;
        self.targets.sync(size, sample_count)
    }

    /// Reconcile the MSAA level the scene pipelines are built for with what the AA
    /// setting asks for, and hand back the level the targets must match.
    ///
    /// The pipelines are rebuilt here, *before* any target is: their sample count is
    /// baked at creation but they own nothing the targets depend on, so a failure
    /// leaves every resource and every field describing the level still in force, and
    /// the next frame simply tries again. Recreating the targets first would strand
    /// them at the new level with pipelines built for the old one.
    pub(super) fn sync_sample_count(
        &mut self,
        frame: &Frame<'_>,
        requested: u32,
    ) -> GpuResult<u32> {
        // Only re-ask the adapter when the *request* moved (invariant 4): an
        // unsupported level degrades to the nearest supported one rather than failing
        // target creation on every frame.
        let requested = requested.max(1);
        if requested == self.requested_sample_count {
            return Ok(self.sample_count);
        }
        let sample_count = frame.clamp_msaa(requested);
        if sample_count != self.sample_count {
            self.scene = build_scene_pipelines(sample_count)?;
            self.sample_count = sample_count;
        }
        // Written only once the rebuild succeeded: caching the request past a failure
        // would report the level as satisfied and leave it silently dropped until the
        // user changed it again.
        self.requested_sample_count = requested;
        Ok(sample_count)
    }

    /// Build (or resize) the split's second target set, and hand back both. Called
    /// only by the Opt split; every other path uses [`Self::targets`] alone.
    pub(super) fn sync_split_targets(&mut self, size: (u32, u32)) -> GpuResult<()> {
        match self.split_targets.as_mut() {
            Some(set) => set.sync(size, self.sample_count),
            None => {
                self.split_targets = Some(TargetSet::new(size.0, size.1, self.sample_count)?);
                Ok(())
            }
        }
    }

    /// Drop the split's second set (invariant 3) — no split is on screen.
    pub(super) fn release_split_targets(&mut self) {
        self.split_targets = None;
    }

    /// The primary target set, which every single-view path renders through.
    pub(super) fn targets(&self) -> &TargetSet {
        &self.targets
    }

    /// The split's second set, or the primary one if it somehow has not been built —
    /// a wrong-looking right half beats a blank frame.
    pub(super) fn split_targets(&self) -> &TargetSet {
        self.split_targets.as_ref().unwrap_or(&self.targets)
    }
}
