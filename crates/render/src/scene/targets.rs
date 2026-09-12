//! The offscreen attachments a frame draws into, and the backbuffer rect the
//! composite lands in.
//!
//! A [`TargetSet`] is the seven targets one view needs. The Opt split holds
//! *two* of them, each sized to half the viewport: the composite is a deferred
//! swapchain job, so both halves' passes have run before either composite does,
//! and one shared set would show the second view in both.

use std::ffi::CStr;

use crate::GtaoQuality;
use crate::rhi::{ColorTarget, DepthTarget, Format, Frame, GpuResult};

use super::ao_accum::{AoAccumState, AoPlan};
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
    /// its own depth, the depth prefilter chain, and the two occlusion ping-pongs.
    pub(super) gtao_gbuffer: ColorTarget,
    pub(super) gtao_depth: DepthTarget,
    /// The prefilter chain, level `k` at `max(1, size >> k)`. Level 0 is written by
    /// the G-buffer pass as its second attachment; the rest reduce the one above.
    ///
    /// Five separate images rather than one image's five mips, because sokol refuses
    /// to bind an image as a texture in the pass that attaches it — so a chain in one
    /// image could not be built a level at a time (`VALIDATE_ABND_TEXTURE_BINDING_
    /// VS_COLOR_ATTACHMENT`, silently black in a release build rather than an error).
    pub(super) gtao_depth_mips: [ColorTarget; DEPTH_MIP_LEVELS],
    /// The accumulated occlusion, ping-ponged: the occlusion pass reads the mean so
    /// far from one and writes the updated mean into the other.
    pub(super) ao_history: [ColorTarget; 2],
    /// The denoiser's ping-pong. The composite reads whichever one the last pass
    /// wrote ([`Self::ao_output`]).
    pub(super) ao_denoise: [ColorTarget; 2],
    /// How far this set's occlusion has converged, and what the next frame should do
    /// about it.
    pub(super) ao: AoAccumState,
    pub(super) ao_plan: AoPlan,
}

/// Levels in the GTAO depth prefilter chain, including the full-resolution level 0.
pub(super) const DEPTH_MIP_LEVELS: usize = 5;

impl TargetSet {
    pub(super) fn new(width: u32, height: u32, sample_count: u32) -> GpuResult<Self> {
        let (width, height) = (width.max(1), height.max(1));
        Ok(Self {
            color: ColorTarget::hdr(width, height, sample_count, c"scene colour")?,
            ambient: ColorTarget::hdr(width, height, sample_count, c"scene ambient")?,
            depth: DepthTarget::new(width, height, sample_count, c"scene depth")?,
            gtao_gbuffer: ColorTarget::hdr(width, height, 1, c"gtao gbuffer")?,
            gtao_depth: DepthTarget::new(width, height, 1, c"gtao depth")?,
            gtao_depth_mips: depth_mips(width, height)?,
            ao_history: occlusion_pair(width, height, c"gtao history")?,
            ao_denoise: occlusion_pair(width, height, c"gtao denoise")?,
            ao: AoAccumState::default(),
            ao_plan: AoPlan::default(),
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
            self.gtao_depth_mips = depth_mips(width, height)?;
            self.ao_history = occlusion_pair(width, height, c"gtao history")?;
            self.ao_denoise = occlusion_pair(width, height, c"gtao denoise")?;
            // The history buffers the average lived in are gone, so the average is
            // too — whatever the key says.
            self.ao.reset();
        }
        Ok(())
    }

    /// The target the last denoise pass wrote, which is what the composite reads.
    ///
    /// Derived from the pass count rather than remembered, so it stays right on a
    /// frame where the passes were skipped entirely (converged): the image is still
    /// sitting in the target the last run left it in, and nothing else writes there.
    pub(super) fn ao_output(&self, quality: GtaoQuality) -> &ColorTarget {
        &self.ao_denoise[last_denoise_index(quality)]
    }

    /// The size the occlusion passes run at, which the shader needs and sokol cannot
    /// tell it.
    pub(super) fn gtao_raw_size(&self) -> (u32, u32) {
        self.ao_history[0].size()
    }
}

/// Which of a [`SceneGpu`]'s target sets a call means. The Opt split is the only
/// thing with two, and each converges its occlusion independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetSetId {
    Primary,
    Split,
}

/// Which of the two denoise targets the final pass lands in.
pub(super) fn last_denoise_index(quality: GtaoQuality) -> usize {
    (quality.denoise_passes().max(1) as usize - 1) % 2
}

/// The prefilter chain at `width`×`height`, each level half the one before (never
/// below one texel).
///
/// Spelled out rather than built in a loop because the labels are `&CStr` literals
/// and the result is a fixed-size array: `array::try_from_fn` is still unstable, and
/// collecting into a `Vec` to unwrap back into an array would trade a compile-time
/// guarantee for a runtime one.
fn depth_mips(width: u32, height: u32) -> GpuResult<[ColorTarget; DEPTH_MIP_LEVELS]> {
    let level = |shift: u32, label: &CStr| {
        ColorTarget::single_channel(
            (width >> shift).max(1),
            (height >> shift).max(1),
            Format::R32F,
            label,
        )
    };
    Ok([
        level(0, c"gtao depth mip 0")?,
        level(1, c"gtao depth mip 1")?,
        level(2, c"gtao depth mip 2")?,
        level(3, c"gtao depth mip 3")?,
        level(4, c"gtao depth mip 4")?,
    ])
}

/// A ping-pong pair of occlusion targets.
fn occlusion_pair(width: u32, height: u32, label: &CStr) -> GpuResult<[ColorTarget; 2]> {
    Ok([
        ColorTarget::single_channel(width, height, Format::R16F, label)?,
        ColorTarget::single_channel(width, height, Format::R16F, label)?,
    ])
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

    /// A target set by name, for the reconciliation that has to write to it. Falls
    /// back to the primary set on the same terms as [`Self::split_targets`].
    pub(super) fn targets_mut(&mut self, set: TargetSetId) -> &mut TargetSet {
        match set {
            TargetSetId::Split if self.split_targets.is_some() => {
                self.split_targets.as_mut().unwrap_or(&mut self.targets)
            }
            _ => &mut self.targets,
        }
    }
}
