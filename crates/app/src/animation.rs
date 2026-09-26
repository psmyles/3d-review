//! The animation clock and the pose it evaluates for the renderer.
//!
//! Split out of `main.rs` as its own concern, like the undo history: `frame.rs`
//! calls [`App::tick_animation`] once per redraw and keeps pacing frames while
//! [`App::animation_playing`]; the evaluated [`DeformPose`] rides into the
//! `SceneFrame` with a revision the renderer re-uploads the palette on. The UI
//! owns *which* clip is selected and whether playback is requested
//! ([`review_ui::AnimationUiState`], edited in place by the Outliner tab and the
//! transport); the clock, the pose and the redraw pacing live here (invariants 2
//! and 6). Playback is view state and never enters the undo history.

use std::time::Instant;

use review_model::{AnimContext, DeformPose, Pose, anim};

use crate::App;

/// Cap on one frame's advance of the clip clock. Playback keeps the redraw loop
/// continuous, so consecutive frames are one refresh interval apart; a larger gap
/// means the window was hidden or the thread stalled, and jumping the clip by that
/// gap would read as a skip. Unlike the camera transition's 33 ms guard this is
/// generous, so a real-time clip never runs slow on a slow frame.
const MAX_STEP_SECONDS: f64 = 0.25;

/// The clock + evaluated pose, grouped out of [`App`].
pub(crate) struct AnimationSubsystem {
    /// Per-model evaluation context, rebuilt when the scene revision moves.
    context: Option<AnimContext>,
    /// The scene revision `context` (and `pose`) describe; `u64::MAX` = none.
    model_revision: u64,
    pose: Pose,
    /// The palette + shape weights the renderer deforms with — valid while
    /// [`Self::active`].
    pub(crate) deform: DeformPose,
    /// Whether `deform` describes the current model (a model needing the deform
    /// path has had a pose evaluated). `frame.rs` hands the renderer the pose only
    /// while this is set.
    pub(crate) active: bool,
    /// Bumped on every re-evaluation; the renderer keys its palette upload on it.
    pub(crate) pose_revision: u64,
    /// Wall-clock instant of the previous playing frame; `None` while paused.
    last_tick: Option<Instant>,
    /// The `(clip, time bits)` the current pose was evaluated for, so a paused
    /// clip re-evaluates nothing.
    evaluated: Option<(Option<usize>, u64)>,
}

impl Default for AnimationSubsystem {
    fn default() -> Self {
        Self {
            context: None,
            model_revision: u64::MAX,
            pose: Pose::default(),
            deform: DeformPose::default(),
            active: false,
            pose_revision: 0,
            last_tick: None,
            evaluated: None,
        }
    }
}

impl AnimationSubsystem {
    /// The per-model evaluation context the pose was built against, or `None`
    /// before one exists. Handed out for the CPU deform the viewport pick runs,
    /// which has to mirror exactly what the shader was given.
    pub(crate) fn context(&self) -> Option<&AnimContext> {
        self.context.as_ref()
    }
}

impl App {
    /// Whether a clip is playing in the 3D workspace — the redraw loop keeps
    /// pacing frames while it is.
    pub(crate) fn animation_playing(&self) -> bool {
        self.ui.mode == review_ui::WorkspaceMode::ThreeD
            && self.ui.animation.playing
            && self.ui.animation.selected_clip.is_some()
    }

    /// Drop the evaluated pose and clock: the model is about to change. The
    /// context is rebuilt lazily by the next tick, from the model then loaded.
    pub(crate) fn reset_animation(&mut self) {
        self.animation = AnimationSubsystem::default();
    }

    /// Advance the clock while playing, then re-evaluate the pose when the
    /// selected clip or the time moved. Runs once per frame from `render`.
    pub(crate) fn tick_animation(&mut self) {
        let model = self.scene_model.clone();
        let anim = &mut self.animation;

        if !model.needs_deform() {
            // A static model draws its raw buffers; nothing to evaluate.
            if anim.active || anim.context.is_some() {
                *anim = AnimationSubsystem::default();
            }
            self.ui.animation.playing = false;
            return;
        }

        let rebuilt = anim.model_revision != self.scene_revision;
        if rebuilt {
            anim.context = Some(AnimContext::new(&model));
            anim.pose = Pose::new(&model);
            anim.model_revision = self.scene_revision;
            anim.evaluated = None;
            anim.last_tick = None;
        }

        // A clip index that no longer resolves (a stale selection) reads as the
        // rest pose.
        let state = &mut self.ui.animation;
        let clip = state
            .selected_clip
            .and_then(|index| model.animations.get(index));
        if clip.is_none() {
            state.selected_clip = None;
            state.playing = false;
        }

        // The clock only runs in the 3D workspace: elsewhere the pose is not
        // drawn, and resuming from a paused clock mustn't jump ahead.
        let playing = state.playing && self.ui.mode == review_ui::WorkspaceMode::ThreeD;
        if let (true, Some(clip)) = (playing, clip) {
            let now = Instant::now();
            let step = anim
                .last_tick
                .map_or(0.0, |last| now.duration_since(last).as_secs_f64())
                .min(MAX_STEP_SECONDS);
            anim.last_tick = Some(now);
            let advanced = state.time + step * state.speed.factor();
            if advanced > clip.time_end {
                if state.looping {
                    state.time = clip.wrap_time(advanced);
                } else {
                    state.time = clip.time_end;
                    state.playing = false;
                }
            } else {
                state.time = advanced;
            }
        } else {
            anim.last_tick = None;
        }

        // Framing and the bounding box describe the selected clip's whole motion —
        // once it has been measured. That happens on the import worker just after
        // the model is drawn (`loading.rs`), so a clip selected in the first
        // moments of a load frames on the whole model until its envelope lands.
        let clip_envelope = state
            .selected_clip
            .and_then(|index| self.ui.clip_bounds.get(index).copied().flatten());
        self.ui.bounds = clip_envelope.or(model.bounds);

        let key = (state.selected_clip, state.time.to_bits());
        if anim.evaluated != Some(key) || rebuilt {
            let Some(context) = anim.context.as_ref() else {
                return;
            };
            anim::evaluate_pose(&model, context, clip, state.time, &mut anim.pose);
            anim::build_palette(&model, context, &anim.pose, &mut anim.deform);
            anim.pose_revision = anim.pose_revision.wrapping_add(1);
            anim.evaluated = Some(key);
            anim.active = true;
        }
    }

    /// Space: play / pause the selected clip. Resuming restarts the clock so the
    /// pause itself is never counted as elapsed time.
    pub(crate) fn toggle_playback(&mut self) {
        let state = &mut self.ui.animation;
        let Some(clip) = state
            .selected_clip
            .and_then(|index| self.scene_model.animations.get(index))
        else {
            return;
        };
        state.toggle_playback(clip);
        self.animation.last_tick = None;
    }

    /// `,` / `.`: step the selected clip one frame back / forward at the file's
    /// frame rate, pausing playback.
    pub(crate) fn step_frame(&mut self, delta: i64) {
        let model = self.scene_model.clone();
        let state = &mut self.ui.animation;
        let Some(clip) = state
            .selected_clip
            .and_then(|index| model.animations.get(index))
        else {
            return;
        };
        state.time = clip.step_frame_time(state.time, delta, model.frame_rate_or_default());
        state.playing = false;
        self.animation.last_tick = None;
        self.redraw.requested = true;
    }
}
