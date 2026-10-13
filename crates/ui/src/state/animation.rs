//! Playback state for the transport card, and the clips it picks from.
//!
//! This is view state and is never undone: the clock and the evaluated pose live
//! in `app`, and the pose math in `review_model::anim`. A clip is addressed by a
//! [`ClipId`] — which file, then which of its clips — since the model's own file
//! and every animation file attached to it each contribute their own.

use std::sync::Arc;

use review_model::{AttachedAnimation, Bounds, ClipId, Clips, ModelData};

use crate::UiState;

/// The transport's playback-rate choices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaybackSpeed {
    Quarter,
    Half,
    #[default]
    Normal,
    Double,
}

impl PlaybackSpeed {
    /// Every speed, in dropdown order.
    pub const ALL: [PlaybackSpeed; 4] = [
        PlaybackSpeed::Quarter,
        PlaybackSpeed::Half,
        PlaybackSpeed::Normal,
        PlaybackSpeed::Double,
    ];

    pub fn label(self) -> review_localization::Key {
        match self {
            PlaybackSpeed::Quarter => crate::keys::ui_transport::SPEED_QUARTER,
            PlaybackSpeed::Half => crate::keys::ui_transport::SPEED_HALF,
            PlaybackSpeed::Normal => crate::keys::ui_transport::SPEED_NORMAL,
            PlaybackSpeed::Double => crate::keys::ui_transport::SPEED_DOUBLE,
        }
    }

    /// The multiplier on wall-clock time.
    pub fn factor(self) -> f64 {
        match self {
            PlaybackSpeed::Quarter => 0.25,
            PlaybackSpeed::Half => 0.5,
            PlaybackSpeed::Normal => 1.0,
            PlaybackSpeed::Double => 2.0,
        }
    }
}

/// Animation playback state. Owned by [`crate::UiState`] and edited in place by the
/// Outliner's Animations tab and the transport (the convention selection and the
/// Opt stack follow); `app` owns the clock and advances [`Self::time`] while
/// [`Self::playing`], evaluates the pose, and paces the redraw loop (invariant
/// 6). View state: deliberately outside the undo history.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationUiState {
    /// The selected clip, or `None` for the rest pose. Selecting a clip lands
    /// paused on its first frame.
    pub selected_clip: Option<ClipId>,
    pub playing: bool,
    /// Loop at the end (default) or stop on the last frame.
    pub looping: bool,
    pub speed: PlaybackSpeed,
    /// The current time on the clip's own timeline, in seconds.
    pub time: f64,
    /// Whether the model or any file attached to it carries a clip — gates the
    /// Animations tab. Kept by [`crate::UiState::reset_animation_state`] and
    /// [`crate::UiState::attach_animation`].
    pub has_clips: bool,
    /// Whether the selected clip's animation curves are drawn — the path each
    /// bone or animated object takes, in a color of its own that its bone
    /// shares. The transport's toggle; off to begin with, and kept across clip
    /// changes so stepping through clips compares their paths. `app` samples the
    /// paths while it is on and frees them when it goes off (invariant 3).
    pub show_curves: bool,
}

impl Default for AnimationUiState {
    fn default() -> Self {
        Self {
            selected_clip: None,
            playing: false,
            looping: true,
            speed: PlaybackSpeed::default(),
            time: 0.0,
            has_clips: false,
            show_curves: false,
        }
    }
}

impl AnimationUiState {
    /// Play or pause `clip`, the selected one. Playing again from the end of a
    /// non-looping clip starts it over, so the button never presses to no effect.
    /// The transport's button and `Space` both come here, so the two cannot
    /// disagree about what resuming at the end means.
    pub fn toggle_playback(&mut self, clip: &review_model::AnimationClip) {
        if !self.playing && !self.looping && self.time >= clip.time_end {
            self.time = clip.time_begin;
        }
        self.playing = !self.playing;
    }
}

impl UiState {
    /// Every clip on offer: `model`'s own, then each attached file's.
    pub fn clips<'a>(&'a self, model: &'a ModelData) -> Clips<'a> {
        Clips::new(model, &self.attached_animations)
    }

    /// The motion envelope of clip `id`, once it has been measured.
    pub fn clip_envelope(&self, id: ClipId) -> Option<Bounds> {
        self.clip_bounds
            .get(id.file)?
            .get(id.clip)
            .copied()
            .flatten()
    }

    /// Re-point the animation state at `model`, the one about to be shown, with
    /// nothing attached to it yet: drop the attached files, their envelopes, the
    /// clip selection and playback (all of them index the old session's clips),
    /// and re-derive the capability flag that gates the Animations tab. A stale
    /// Animations tab needs no snapping back here: the Outliner resolves its tab
    /// against what the model offers every frame ([`super::OutlinerState::tab`]).
    pub fn reset_animation_state(&mut self, model: &ModelData) {
        self.attached_animations.clear();
        self.clip_bounds = vec![vec![None; model.animations.len()]];
        self.animation = AnimationUiState {
            has_clips: !model.animations.is_empty(),
            ..AnimationUiState::default()
        };
    }

    /// Add an animation file to the session; its clips are file
    /// `attached_animations.len()` once it is in. Nothing already selected moves,
    /// since files are only ever appended.
    pub fn attach_animation(&mut self, file: Arc<AttachedAnimation>) {
        self.animation.has_clips |= !file.clips.is_empty();
        self.clip_bounds.push(vec![None; file.clips.len()]);
        self.attached_animations.push(file);
    }

    /// File `file`'s clip envelopes, as measured. A list that no longer fits the
    /// file (a session changed under the measurement) is ignored.
    pub fn set_clip_bounds(&mut self, file: usize, bounds: Vec<Option<Bounds>>) {
        if let Some(slot) = self.clip_bounds.get_mut(file)
            && slot.len() == bounds.len()
        {
            *slot = bounds;
        }
    }

    /// Select clip `clip` (or none), landing paused on its first frame; the app's
    /// clock picks the change up on the next frame.
    pub fn select_clip(&mut self, model: &ModelData, clip: Option<ClipId>) {
        let found = clip.and_then(|id| self.clips(model).get(id).map(|clip| (id, clip.time_begin)));
        self.animation.selected_clip = found.map(|(id, _)| id);
        self.animation.playing = false;
        self.animation.time = found.map_or(0.0, |(_, begin)| begin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> review_model::AnimationClip {
        review_model::AnimationClip {
            time_begin: 1.0,
            time_end: 3.0,
            ..Default::default()
        }
    }

    #[test]
    fn playing_again_from_the_end_of_a_clip_that_does_not_loop_starts_over() {
        let mut state = AnimationUiState {
            looping: false,
            time: 3.0,
            ..Default::default()
        };
        state.toggle_playback(&clip());
        assert!(state.playing);
        assert_eq!(state.time, 1.0);
    }

    #[test]
    fn a_looping_clip_or_a_pause_keeps_its_time() {
        let mut looping = AnimationUiState {
            time: 3.0,
            ..Default::default()
        };
        looping.toggle_playback(&clip());
        assert_eq!(looping.time, 3.0, "a looping clip wraps by itself");

        let mut pausing = AnimationUiState {
            looping: false,
            playing: true,
            time: 3.0,
            ..Default::default()
        };
        pausing.toggle_playback(&clip());
        assert!(!pausing.playing);
        assert_eq!(pausing.time, 3.0, "pausing never moves the playhead");
    }
}
