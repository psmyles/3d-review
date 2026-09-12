//! Baked animation: the tracks a clip carries, and the time range it spans.
//!
//! Frame counts derive from the stack range times the frame rate, never from key
//! counts — `ufbx_bake_anim` can place keys past the stack range, and the
//! evaluator holds the end values outside them. A clip's *motion envelope* is
//! not here: it is measured after the model is drawn and lives in the UI.

use glam::{Quat, Vec3};

use crate::*;

/// One keyframe of a baked animation channel: linearly (or, for rotations,
/// spherically) interpolated to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key<T> {
    /// Absolute time in seconds on the clip's own timeline.
    pub time: f64,
    pub value: T,
}

/// The baked transform animation of one node within a clip. A channel with no
/// keys is not animated by the clip and keeps the node's [`SceneNode::rest_local`]
/// value; a channel with keys holds its first/last value outside the key range.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeTrack {
    /// Indexes [`ModelData::nodes`].
    pub node: u32,
    pub translation: Vec<Key<Vec3>>,
    pub rotation: Vec<Key<Quat>>,
    pub scale: Vec<Key<Vec3>>,
}

/// The baked weight animation of one blend-shape channel within a clip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MorphTrack {
    /// Indexes [`MorphData::channels`].
    pub channel: u32,
    /// Channel weight in `0..=1` (the file's percent ÷ 100).
    pub keys: Vec<Key<f32>>,
}

/// One animation clip — an FBX animation stack, baked to keyframes at import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    /// The stack's playback range, in seconds. Frame numbering and the transport
    /// derive from this range and the file's frame rate, never from key counts:
    /// baked keys may extend past it (stepped keys, exporter padding) and are
    /// simply held there.
    pub time_begin: f64,
    pub time_end: f64,
    pub tracks: Vec<NodeTrack>,
    pub morph_tracks: Vec<MorphTrack>,
}

impl AnimationClip {
    pub fn duration(&self) -> f64 {
        (self.time_end - self.time_begin).max(0.0)
    }

    /// The number of frames the transport steps through at `fps`: the range
    /// rounded to whole frames, plus the frame at `time_begin` itself.
    pub fn frame_count(&self, fps: f64) -> usize {
        let fps = frame_rate_or_default(fps);
        (self.duration() * fps).round().max(0.0) as usize + 1
    }

    /// The time of frame `frame` (0-based) at `fps`, clamped into the clip.
    pub fn frame_time(&self, frame: usize, fps: f64) -> f64 {
        let fps = frame_rate_or_default(fps);
        self.clamp_time(self.time_begin + frame as f64 / fps)
    }

    /// The frame `time` falls on at `fps` (nearest, 0-based, within the clip).
    pub fn frame_at(&self, time: f64, fps: f64) -> usize {
        let fps = frame_rate_or_default(fps);
        let frame = ((time - self.time_begin) * fps).round().max(0.0) as usize;
        frame.min(self.frame_count(fps) - 1)
    }

    /// The time `delta` whole frames from the frame `time` falls on, clamped
    /// into the clip — one step of the transport's frame buttons / `,` `.` keys.
    pub fn step_frame_time(&self, time: f64, delta: i64, fps: f64) -> f64 {
        let frame = self.frame_at(time, fps) as i64 + delta;
        let last = self.frame_count(fps) as i64 - 1;
        self.frame_time(frame.clamp(0, last) as usize, fps)
    }

    /// `time` clamped into the clip's range.
    pub fn clamp_time(&self, time: f64) -> f64 {
        time.clamp(self.time_begin, self.time_end.max(self.time_begin))
    }

    /// `time` wrapped into the clip's range (looping playback). A zero-length
    /// clip always reads as its start.
    pub fn wrap_time(&self, time: f64) -> f64 {
        let duration = self.duration();
        if duration <= 0.0 {
            return self.time_begin;
        }
        self.time_begin + (time - self.time_begin).rem_euclid(duration)
    }

    /// The import-funnel guard: every track names a real node / channel, and
    /// every key is finite with non-decreasing times.
    pub fn validate(&self, node_count: usize, channel_count: usize) -> Result<(), String> {
        if !self.time_begin.is_finite()
            || !self.time_end.is_finite()
            || self.time_end < self.time_begin
        {
            return Err(format!(
                "clip '{}' has an invalid time range {}..{}",
                self.name, self.time_begin, self.time_end
            ));
        }
        fn check_times<T>(keys: &[Key<T>], what: &str, clip: &str) -> Result<(), String> {
            if keys.iter().any(|key| !key.time.is_finite()) {
                return Err(format!("clip '{clip}' has a non-finite {what} key time"));
            }
            if keys.windows(2).any(|pair| pair[1].time < pair[0].time) {
                return Err(format!("clip '{clip}' has {what} keys out of order"));
            }
            Ok(())
        }
        for track in &self.tracks {
            if track.node as usize >= node_count {
                return Err(format!(
                    "clip '{}' animates node {} of {node_count}",
                    self.name, track.node
                ));
            }
            check_times(&track.translation, "translation", &self.name)?;
            check_times(&track.rotation, "rotation", &self.name)?;
            check_times(&track.scale, "scale", &self.name)?;
            if track.translation.iter().any(|key| !key.value.is_finite())
                || track.scale.iter().any(|key| !key.value.is_finite())
                || track.rotation.iter().any(|key| !key.value.is_finite())
            {
                return Err(format!(
                    "clip '{}' has a non-finite transform key",
                    self.name
                ));
            }
        }
        for track in &self.morph_tracks {
            if track.channel as usize >= channel_count {
                return Err(format!(
                    "clip '{}' animates morph channel {} of {channel_count}",
                    self.name, track.channel
                ));
            }
            check_times(&track.keys, "morph", &self.name)?;
            if track.keys.iter().any(|key| !key.value.is_finite()) {
                return Err(format!("clip '{}' has a non-finite morph key", self.name));
            }
        }
        Ok(())
    }
}

/// The frame rate assumed when a file declares none.
pub const DEFAULT_FRAME_RATE: f64 = 30.0;

/// `fps` when it is a usable rate, else [`DEFAULT_FRAME_RATE`].
pub fn frame_rate_or_default(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        DEFAULT_FRAME_RATE
    }
}

/// The file's own frame rate, or the fallback.
impl ModelData {
    /// The file's frame rate, or [`DEFAULT_FRAME_RATE`] when it declared none.
    pub fn frame_rate_or_default(&self) -> f64 {
        frame_rate_or_default(self.frame_rate)
    }
}
