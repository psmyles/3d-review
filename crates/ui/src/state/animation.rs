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

    pub fn label(self) -> &'static str {
        match self {
            PlaybackSpeed::Quarter => "0.25x",
            PlaybackSpeed::Half => "0.5x",
            PlaybackSpeed::Normal => "1x",
            PlaybackSpeed::Double => "2x",
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

/// Animation playback state. Owned by [`UiState`] and edited in place by the
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
    /// on load by [`UiState::reset_animation_state`].
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
