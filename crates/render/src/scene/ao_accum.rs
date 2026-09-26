//! When the ambient occlusion is allowed to keep averaging, and what noise pattern
//! each accumulated frame uses.
//!
//! GTAO estimates a pixel's visibility from a handful of horizon samples, and at any
//! affordable sample count one frame's estimate carries visible noise. Every shipped
//! implementation answers that by averaging over time — usually by reprojecting a
//! history buffer through the camera's motion, which needs velocity vectors and a
//! disocclusion heuristic, and which is really there to serve a game that is never
//! still.
//!
//! A model viewer is the opposite case: it is still most of the time, and when it
//! moves the user is orbiting rather than reading detail. So there is no
//! reprojection here at all. While anything that could change a pixel's occlusion
//! changes, the estimate is used as-is; the moment everything holds still, each
//! frame's estimate is folded into a running mean and the image converges. After
//! [`AO_CONVERGED_FRAMES`] frames it stops: the result cannot improve further, the
//! passes are skipped entirely, and the viewer goes back to being idle (invariant 6
//! — this is a real animation that ends on its own, not a spin loop).
//!
//! Everything here is plain data with no GPU types, so the decision is unit-testable
//! and the one place it is made.

use crate::{GtaoSettings, OrbitCamera, SceneFrame};

use super::slot::SlotId;

/// How many frames are averaged before the result is called converged.
///
/// 24 because that is exactly one pass through the temporal pattern set below —
/// 6 slice rotations × 4 step offsets — so every accumulated frame contributes a
/// distinct sampling pattern and none is counted twice. At 60 Hz it is 0.4 s after
/// the camera stops.
pub(super) const AO_CONVERGED_FRAMES: u32 = 24;

/// Jimenez 2016's temporal slice rotations, in turns. Six evenly spread rotations,
/// ordered so consecutive frames are maximally far apart rather than adjacent —
/// which matters because the image is on screen during the sequence, not only after
/// it.
const TEMPORAL_ROTATIONS: [f32; 6] = [
    60.0 / 360.0,
    300.0 / 360.0,
    180.0 / 360.0,
    240.0 / 360.0,
    120.0 / 360.0,
    0.0,
];

/// Jimenez 2016's temporal step offsets, in fractions of a step. Advances once per
/// full cycle of the rotations, so the two sequences do not repeat together until
/// all 24 combinations have been used.
const TEMPORAL_OFFSETS: [f32; 4] = [0.0, 0.5, 0.25, 0.75];

/// This frame's place in the temporal sequence, as `(slice rotation, step offset)`
/// in turns / steps. Index 0 is the plain spatial pattern, which is what a moving
/// frame gets.
pub(super) fn temporal_pattern(index: u32) -> (f32, f32) {
    let rotation = TEMPORAL_ROTATIONS[(index % 6) as usize];
    let offset = TEMPORAL_OFFSETS[((index / 6) % 4) as usize];
    (rotation, offset)
}

/// Everything that decides what the occlusion of a pixel is. If any of it moves, the
/// frames already averaged describe a different image and have to be thrown away.
///
/// Compared by value every frame, so it is deliberately all `Copy` scalars — no
/// slice, no `Vec`, nothing that would allocate or walk. The hidden-mesh set reaches
/// it as [`super::slot::ModelSlot::visibility_generation`] for exactly that reason.
///
/// What is *not* in here matters as much as what is. The selection highlight's colour
/// and opacity never touch the G-buffer, so including them would restart the average
/// for nothing. The MSAA level does not either: the GTAO targets are single-sample by
/// design and an AA change does not recreate them.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct AoKey {
    /// World → view and view → clip. Between them they pin the camera completely:
    /// position, orientation, zoom, and the perspective/orthographic switch.
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    gtao: GtaoSettings,
    model_revision: u64,
    /// The evaluated pose. While a clip plays this moves every frame, which keeps
    /// the average at a single sample — correct, since each frame is a different
    /// pose and blending them would smear.
    pose_revision: u64,
    visibility_generation: u64,
    slot: SlotId,
    size: (u32, u32),
}

impl AoKey {
    /// Read the key out of the frame's own inputs, so there is no second list of
    /// what matters to keep in step with [`SceneFrame`].
    pub(super) fn new(
        camera: OrbitCamera,
        scene: &SceneFrame<'_>,
        visibility_generation: u64,
        slot: SlotId,
        size: (u32, u32),
    ) -> Self {
        Self {
            view: camera.view_matrix().to_cols_array_2d(),
            proj: camera
                .projection_matrix(scene.projection)
                .to_cols_array_2d(),
            gtao: scene.gtao,
            model_revision: scene.model_revision,
            pose_revision: scene.pose_revision,
            visibility_generation,
            slot,
            size,
        }
    }
}

/// One target set's accumulation state. The Opt split holds two sets and therefore
/// two of these, so its halves converge independently.
#[derive(Clone, Copy, Default)]
pub(super) struct AoAccumState {
    /// What the frames already averaged were computed for. `None` before the first.
    key: Option<AoKey>,
    /// How many frames are in the running mean.
    frames: u32,
    /// Which of the two history targets holds that mean.
    current: usize,
    /// Whether GTAO ran at all this frame — so an idle viewer with AO switched off
    /// does not report itself as converging forever.
    active: bool,
}

/// What [`AoAccumState::advance`] decided for this frame.
#[derive(Clone, Copy, Default)]
pub(super) struct AoPlan {
    /// Whether the GTAO passes run at all. `false` once converged: the last denoised
    /// result is still in its target and nothing else writes there, so the composite
    /// keeps reading the same image.
    pub(super) run: bool,
    /// The history target holding the mean so far, and the one this frame writes.
    pub(super) read: usize,
    pub(super) write: usize,
    /// This frame's weight in the running mean: `1/(n+1)`. Exactly `1.0` means the
    /// history was discarded, which the shader reads as "do not sample it" — the
    /// target may never have been written.
    pub(super) weight: f32,
    /// Which entry of the temporal pattern set this frame uses.
    pub(super) temporal_index: u32,
}

impl AoAccumState {
    /// Decide this frame's accumulation and advance the state.
    ///
    /// `None` means GTAO is not running this frame (switched off, no mesh, or one of
    /// the flat data-inspection views). That neither advances nor resets: coming back
    /// to a view you left is the common case, and the key comparison already catches
    /// anything that actually changed while you were away.
    pub(super) fn advance(&mut self, key: Option<AoKey>) -> AoPlan {
        let Some(key) = key else {
            self.active = false;
            return AoPlan {
                run: false,
                read: self.current,
                write: self.current,
                weight: 0.0,
                temporal_index: self.frames,
            };
        };
        self.active = true;
        if self.key != Some(key) {
            self.key = Some(key);
            self.frames = 0;
        }
        if self.frames >= AO_CONVERGED_FRAMES {
            return AoPlan {
                run: false,
                read: self.current,
                write: self.current,
                weight: 0.0,
                temporal_index: self.frames,
            };
        }
        let plan = AoPlan {
            run: true,
            read: self.current,
            write: 1 - self.current,
            // 1/(n+1): frame 0 takes the whole result, frame 1 half, and so on, which
            // is a plain running mean rather than an exponential one. A mean is right
            // here because the sequence is finite and every frame is equally valid;
            // an exponential blend would keep the earliest patterns weighted forever.
            weight: 1.0 / (self.frames + 1) as f32,
            temporal_index: self.frames,
        };
        self.current = plan.write;
        self.frames += 1;
        plan
    }

    /// Whether more frames would still improve this set's result — the term that
    /// keeps the app redrawing (invariant 6).
    pub(super) fn converging(&self) -> bool {
        self.active && self.frames < AO_CONVERGED_FRAMES
    }

    /// Throw the average away. Called when the targets themselves are recreated, so
    /// the history buffers no longer hold what the key says they do.
    pub(super) fn reset(&mut self) {
        self.key = None;
        self.frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GtaoQuality;

    /// A key that differs from another only in the AO radius, which stands in for
    /// "the user changed something" without needing a whole `SceneFrame`.
    fn key(radius: f32) -> Option<AoKey> {
        let camera = OrbitCamera::default();
        Some(AoKey {
            view: camera.view_matrix().to_cols_array_2d(),
            proj: camera
                .projection_matrix(crate::CameraProjection::Perspective)
                .to_cols_array_2d(),
            gtao: GtaoSettings {
                enabled: true,
                radius,
                intensity: 1.0,
                thickness: 0.25,
                quality: GtaoQuality::Medium,
            },
            model_revision: 1,
            pose_revision: 0,
            visibility_generation: 0,
            slot: SlotId::Source,
            size: (800, 600),
        })
    }

    /// The first frame under a key must not read the history: nothing has written it.
    #[test]
    fn the_first_frame_replaces_rather_than_blends() {
        let mut state = AoAccumState::default();
        let plan = state.advance(key(0.35));
        assert!(plan.run);
        assert_eq!(plan.weight, 1.0);
    }

    /// Each further still frame takes a smaller share, which is a running mean.
    #[test]
    fn each_frame_takes_its_share_of_the_mean() {
        let mut state = AoAccumState::default();
        state.advance(key(0.35));
        assert_eq!(state.advance(key(0.35)).weight, 0.5);
        assert_eq!(state.advance(key(0.35)).weight, 1.0 / 3.0);
    }

    /// Consecutive frames must alternate targets, or a pass would read the target it
    /// is writing.
    #[test]
    fn the_history_targets_alternate() {
        let mut state = AoAccumState::default();
        let first = state.advance(key(0.35));
        let second = state.advance(key(0.35));
        assert_ne!(first.read, first.write);
        assert_eq!(second.read, first.write);
        assert_eq!(second.write, first.read);
    }

    /// Anything that changes the image discards the average — here, a slider drag.
    #[test]
    fn a_changed_key_starts_the_average_over() {
        let mut state = AoAccumState::default();
        state.advance(key(0.35));
        state.advance(key(0.35));
        let plan = state.advance(key(0.4));
        assert_eq!(
            plan.weight, 1.0,
            "a new key must not blend into the old mean"
        );
    }

    /// The whole point of the counter: it stops, and the viewer can go idle.
    ///
    /// Note where `converging` goes false — right after the *last* sample is taken,
    /// not a frame later. That frame has already been drawn with the full average in
    /// it, so asking for one more would only redraw an identical image.
    #[test]
    fn the_average_stops_once_converged() {
        let mut state = AoAccumState::default();
        for frame in 1..=AO_CONVERGED_FRAMES {
            assert!(state.advance(key(0.35)).run, "frame {frame} must run");
            assert_eq!(
                state.converging(),
                frame < AO_CONVERGED_FRAMES,
                "frame {frame} should only ask for another while samples remain"
            );
        }
        assert!(!state.advance(key(0.35)).run);
        assert!(!state.converging());
    }

    /// A frame with AO switched off neither advances nor resets, and never reports
    /// itself as still converging — that would spin the redraw loop forever.
    #[test]
    fn an_inactive_frame_holds_its_place() {
        let mut state = AoAccumState::default();
        state.advance(key(0.35));
        state.advance(key(0.35));
        let idle = state.advance(None);
        assert!(!idle.run);
        assert!(!state.converging());
        // Back into the same view: it picks up where it left off.
        assert_eq!(state.advance(key(0.35)).weight, 1.0 / 3.0);
    }

    /// 24 frames must be 24 *different* patterns, or the extra frames average the
    /// same noise over and over and the image never cleans up.
    #[test]
    fn every_accumulated_frame_uses_a_distinct_pattern() {
        let mut seen: Vec<(u32, u32)> = Vec::new();
        for index in 0..AO_CONVERGED_FRAMES {
            let (rotation, offset) = temporal_pattern(index);
            // Compared as scaled integers: these are exact sixths/quarters, but the
            // test should not depend on float equality to say so.
            seen.push(((rotation * 360.0) as u32, (offset * 4.0) as u32));
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), AO_CONVERGED_FRAMES as usize);
    }

    /// A target recreation invalidates the history buffers themselves, so the next
    /// frame has to write rather than blend.
    #[test]
    fn a_reset_forces_the_next_frame_to_replace() {
        let mut state = AoAccumState::default();
        state.advance(key(0.35));
        state.advance(key(0.35));
        state.reset();
        assert_eq!(state.advance(key(0.35)).weight, 1.0);
    }
}
