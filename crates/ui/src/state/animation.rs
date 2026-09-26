//! Playback state for the transport card.
//!
//! This is view state and is never undone: the clock and the evaluated pose live
//! in `app`, and the pose math in `review_model::anim`.

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
    /// The selected clip (index into `ModelData::animations`), or `None` for the
    /// rest pose. Selecting a clip lands paused on its first frame.
    pub selected_clip: Option<usize>,
    pub playing: bool,
    /// Loop at the end (default) or stop on the last frame.
    pub looping: bool,
    pub speed: PlaybackSpeed,
    /// The current time on the clip's own timeline, in seconds.
    pub time: f64,
    /// Whether the loaded model carries any clip — gates the Animations tab. Set
    /// on load by [`crate::UiState::reset_animation_state`].
    pub has_clips: bool,
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
