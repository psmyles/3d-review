//! The D2 gate stamp: `--gate-out <file>` (`docs/ARCHITECTURE.md`, Platform decisions D2).
//!
//! The port is only allowed to land on Windows if it costs nothing measurable
//! against `main` on the same box, same fixture, release build. Measuring that by
//! hand is not repeatable — a launch is dominated by whatever else the machine is
//! doing — so the viewer measures itself: with `--gate-out <file>` it loads the
//! fixture, plays a scripted orbit, writes one JSON stamp and exits. The harness
//! (`scripts/gate.ps1`) interleaves launches of the two builds and takes medians.
//!
//! What is measured, and why here rather than in the script:
//!
//!  * **Startup** is the wall clock at the first present *with the model on
//!    screen*. Import runs on a worker thread, so the first present is an empty
//!    viewport and stamping it would time the window, not the viewer. The
//!    process-creation end of the interval is the script's job (`GetProcessTimes`
//!    via `Process.StartTime`) — `app` is `forbid(unsafe_code)` and has no business
//!    calling it. Both ends read the same system clock, so the subtraction is valid.
//!  * **Frame time** is the interval between presents with **vsync off**, which is
//!    the only way the number reflects GPU + CPU work rather than the refresh rate.
//!    CPU time (the frame's work up to the present call) is recorded alongside it,
//!    because a regression in one and not the other says different things.
//!  * **Memory** is *not* measured here. The stamp is written and the process then
//!    sits idle for [`HOLD`] with the model resident, which is precisely the state
//!    D2 wants sampled; the script reads private bytes and GPU dedicated memory
//!    from outside in that window. Doing it from inside would mean either `unsafe`
//!    in `app` or a new dependency in the shipped binary, and would put a DXGI
//!    query in the backend leaf that the Metal side would then owe a twin for — all
//!    to learn what Windows already reports about any process.
//!
//! The measured conditions are the viewer's own defaults (GTAO on, 2× MSAA) rather
//! than overrides, so the gate measures what a user gets; the stamp records them so
//! a comparison across two stamps can be checked rather than assumed. The one thing
//! forced is the animation: D2 asks for a skinned clip playing, so gate mode selects
//! the fixture's first clip and starts it when the model lands.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use glam::Vec2;

use crate::App;

/// Frames drawn but not measured, after the model is up. The first frames after a
/// model lands build every derived buffer the view needs and warm the shader/PSO
/// caches; including them would measure the load, not the steady state.
const WARMUP_FRAMES: u32 = 120;
/// Frames measured. D2 says 300.
const MEASURED_FRAMES: u32 = 300;
/// How long the process sits idle, model resident, after writing the stamp, so the
/// harness can sample memory. Generous on purpose: the sample is a poll from
/// another process and must not race the exit.
const HOLD: Duration = Duration::from_secs(4);
/// The scripted orbit's per-frame delta, in the same units the pointer drag feeds
/// [`review_render::OrbitCamera::orbit`] (0.01 rad per unit). Small enough that the
/// model stays framed for the whole run, large enough that every frame redraws a
/// genuinely different view — a static camera would measure a cache, not a render.
const ORBIT_STEP: Vec2 = Vec2::new(0.35, 0.0);
/// The window gate mode opens at, ignoring any saved placement: the two builds must
/// render the same number of pixels for their frame times to be comparable. In
/// *physical* pixels, so the pixel count is independent of the display scale.
///
/// It is a **request, not a guarantee**: the viewer's minimum window is 960 × 640
/// logical points, and above a 1.66× display scale that minimum is the larger of the
/// two, so the window comes up bigger than this. Measured on a 2× Retina Mac, where
/// a run reports 1920 × 1280 rather than the 1600 × 960 asked for — the clamp is
/// real, not hypothetical, and an earlier version of this comment claimed 150% was
/// the worst case it had to survive.
///
/// That costs the comparison nothing, because the clamp is a property of the display
/// and not of the build: two runs on one box clamp identically. What makes it safe
/// rather than merely lucky is that the stamp records the size the swapchain actually
/// came up at, and both harnesses refuse to compare two runs that disagree — so a
/// figure from a 2× laptop is never silently held against one from a 1× monitor.
pub(crate) const WINDOW_SIZE: (u32, u32) = (1600, 960);

/// One frame's timings, in milliseconds.
struct Sample {
    /// Present-to-present: GPU + CPU, the throughput number.
    total_ms: f64,
    /// The frame's CPU work, from the top of `render` to the present call.
    cpu_ms: f64,
}

/// Where a gate run has got to. The states are strictly ordered — a run never goes
/// back — which is what keeps the stamp honest: the frame samples cannot include a
/// frame from before the model was up, and the hold cannot start before the stamp
/// is written.
enum Phase {
    /// Launched; the import worker has not come back yet.
    AwaitingModel,
    /// Model up, drawing frames that are deliberately thrown away.
    Warmup { left: u32 },
    /// Collecting.
    Measuring,
    /// Stamp written; idling so the harness can sample memory. Exits at the deadline.
    Holding { until: Instant },
}

pub(crate) struct Gate {
    out: PathBuf,
    phase: Phase,
    /// The wall clock at the first present with the model on screen — the far end
    /// of the startup interval whose near end is process creation.
    first_present: Option<SystemTime>,
    /// The previous present, for the present-to-present interval.
    last_present: Option<Instant>,
    samples: Vec<Sample>,
    /// Set by `render` at the top of each frame, read after the present.
    frame_start: Option<Instant>,
    /// Recorded when the model lands, for the stamp.
    fixture: Option<PathBuf>,
    triangles: usize,
    vertices: usize,
    clips: usize,
    clip_playing: bool,
}

impl Gate {
    pub(crate) fn new(out: PathBuf) -> Self {
        Self {
            out,
            phase: Phase::AwaitingModel,
            first_present: None,
            last_present: None,
            samples: Vec::with_capacity(MEASURED_FRAMES as usize),
            frame_start: None,
            fixture: None,
            triangles: 0,
            vertices: 0,
            clips: 0,
            clip_playing: false,
        }
    }

    /// Whether this frame should present without waiting for vblank. True for the
    /// whole run: a vsync-paced frame time measures the monitor.
    fn vsync_off(&self) -> bool {
        !matches!(self.phase, Phase::Holding { .. })
    }

    /// Whether the run wants another frame immediately (it is still collecting).
    fn wants_redraw(&self) -> bool {
        !matches!(self.phase, Phase::Holding { .. })
    }
}

impl App {
    /// Whether a gate run is active — the flag the frame loop reads for its
    /// timing hooks. Cheap enough to call per frame.
    pub(crate) fn gate_active(&self) -> bool {
        self.gate.is_some()
    }

    /// The vsync flag this frame presents with: normally on, off for a gate run.
    pub(crate) fn gate_vsync(&self) -> bool {
        self.gate.as_ref().is_none_or(|gate| !gate.vsync_off())
    }

    /// Mark the top of a frame, and step the scripted orbit. Called from `render`
    /// before any work, so the CPU figure covers the whole frame.
    pub(crate) fn gate_frame_begin(&mut self) {
        let Some(gate) = self.gate.as_mut() else {
            return;
        };
        gate.frame_start = Some(Instant::now());
        let orbiting = matches!(gate.phase, Phase::Warmup { .. } | Phase::Measuring);
        if orbiting && let Some(renderer) = self.renderer.as_mut() {
            renderer.camera.orbit(ORBIT_STEP);
        }
    }

    /// Fold one presented frame into the run, and advance the phase. Writes the
    /// stamp on the last measured frame.
    pub(crate) fn gate_after_present(&mut self, before_present: Option<Instant>) {
        // `gate_finish` needs all of `self`, so the collection borrow is closed
        // before it runs rather than the run being finished inside the match.
        let mut collected = false;
        let Some(gate) = self.gate.as_mut() else {
            return;
        };
        let now = Instant::now();
        let previous = gate.last_present.replace(now);

        match gate.phase {
            // The model is not up yet, so this present is the empty viewport.
            // Deliberately not stamped (see the module docs).
            Phase::AwaitingModel => {}
            Phase::Warmup { left } => {
                if gate.first_present.is_none() {
                    gate.first_present = Some(SystemTime::now());
                }
                gate.phase = if left <= 1 {
                    Phase::Measuring
                } else {
                    Phase::Warmup { left: left - 1 }
                };
            }
            Phase::Measuring => {
                // A frame with no predecessor (the first measured one) has no
                // interval to report, so it seeds `last_present` and nothing else.
                if let (Some(previous), Some(start), Some(present)) =
                    (previous, gate.frame_start, before_present)
                {
                    gate.samples.push(Sample {
                        total_ms: now.duration_since(previous).as_secs_f64() * 1000.0,
                        cpu_ms: present.duration_since(start).as_secs_f64() * 1000.0,
                    });
                }
                collected = gate.samples.len() >= MEASURED_FRAMES as usize;
            }
            Phase::Holding { .. } => {}
        }

        if collected {
            self.gate_finish();
        }
    }

    /// The model landed: record what it is, start its first clip (D2 asks for a
    /// skinned clip playing), and begin the warm-up.
    pub(crate) fn gate_model_ready(&mut self, path: &Path) {
        if self.gate.is_none() {
            return;
        }
        let model = self.scene_model.clone();
        let clips = model.animations.len();
        let playing = clips > 0;
        if playing {
            self.ui.animation.selected_clip = Some(0);
            self.ui.animation.playing = true;
            self.ui.animation.looping = true;
        }
        let Some(gate) = self.gate.as_mut() else {
            return;
        };
        gate.fixture = Some(path.to_path_buf());
        gate.triangles = model.indices.len() / 3;
        gate.vertices = model.vertices.len();
        gate.clips = clips;
        gate.clip_playing = playing;
        gate.phase = Phase::Warmup {
            left: WARMUP_FRAMES,
        };
        log::info!("gate: model up, warming");
    }

    /// Whether the run is still drawing (the frame loop keeps redrawing) and
    /// whether it is over (the event loop exits). Read from `about_to_wait`.
    pub(crate) fn gate_wants_redraw(&self) -> bool {
        self.gate.as_ref().is_some_and(Gate::wants_redraw)
    }

    pub(crate) fn gate_should_exit(&self) -> bool {
        matches!(
            self.gate.as_ref().map(|gate| &gate.phase),
            Some(Phase::Holding { until }) if Instant::now() >= *until
        )
    }

    /// Write the stamp and enter the hold. A write failure is reported and the run
    /// still ends — the harness reports the missing stamp, which is a clearer
    /// failure than a viewer left running.
    fn gate_finish(&mut self) {
        let size = self.gpu.as_ref().map_or((0, 0), review_render::Gpu::size);
        let msaa = self.ui.anti_aliasing.effective_sample_count();
        let gtao = self.ui.gtao.enabled;
        let Some(gate) = self.gate.as_mut() else {
            return;
        };
        let stamp = gate.stamp(size, msaa, gtao);
        match std::fs::write(&gate.out, stamp) {
            Ok(()) => log::info!("gate: stamp written"),
            Err(err) => log::error!("gate: stamp write failed: {err}"),
        }
        gate.phase = Phase::Holding {
            until: Instant::now() + HOLD,
        };
    }
}

impl Gate {
    /// The stamp, as JSON written by hand — `app` has no serializer and this is
    /// eleven fields, none of which need escaping beyond the two paths.
    fn stamp(&self, size: (u32, u32), msaa: u32, gtao: bool) -> String {
        let total: Vec<f64> = self.samples.iter().map(|s| s.total_ms).collect();
        let cpu: Vec<f64> = self.samples.iter().map(|s| s.cpu_ms).collect();
        let first_present_ns = self
            .first_present
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());

        let mut out = String::with_capacity(1024);
        out.push_str("{\n");
        let _ = writeln!(out, "  \"schema\": 1,");
        let _ = writeln!(
            out,
            "  \"exe\": {},",
            json_string(
                &std::env::current_exe()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            )
        );
        let _ = writeln!(out, "  \"pid\": {},", std::process::id());
        let _ = writeln!(
            out,
            "  \"fixture\": {},",
            json_string(
                &self
                    .fixture
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            )
        );
        let _ = writeln!(out, "  \"first_present_unix_ns\": {first_present_ns},");
        let _ = writeln!(out, "  \"frames\": {},", self.samples.len());
        let _ = writeln!(out, "  \"frame_ms\": {},", stats_json(&total));
        let _ = writeln!(out, "  \"cpu_ms\": {},", stats_json(&cpu));
        let _ = writeln!(
            out,
            "  \"settings\": {{ \"vsync\": false, \"msaa\": {msaa}, \"gtao\": {gtao}, \
             \"clip_playing\": {}, \"width\": {}, \"height\": {} }},",
            self.clip_playing, size.0, size.1
        );
        let _ = writeln!(
            out,
            "  \"model\": {{ \"triangles\": {}, \"vertices\": {}, \"clips\": {} }}",
            self.triangles, self.vertices, self.clips
        );
        out.push_str("}\n");
        out
    }
}

/// Median / p95 / mean / min / max of one series, as a JSON object. Sorting a copy
/// rather than the samples keeps the series in frame order for anything later.
fn stats_json(values: &[f64]) -> String {
    if values.is_empty() {
        return "{ \"median\": 0.0, \"p95\": 0.0, \"mean\": 0.0, \"min\": 0.0, \"max\": 0.0 }"
            .to_owned();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = percentile(&sorted, 0.5);
    let p95 = percentile(&sorted, 0.95);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    format!(
        "{{ \"median\": {median:.4}, \"p95\": {p95:.4}, \"mean\": {mean:.4}, \
         \"min\": {min:.4}, \"max\": {max:.4} }}"
    )
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// A JSON string literal. Windows paths carry backslashes, which is exactly what
/// JSON escapes, so this is not optional.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// How often the loop wakes during a gate run's idle hold, to notice the deadline.
pub(crate) const GATE_POLL: Duration = Duration::from_millis(100);
#[cfg(test)]
mod tests {
    use super::{json_string, percentile, stats_json};

    #[test]
    fn percentiles_are_nearest_rank() {
        let sorted = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&sorted, 0.5), 3.0);
        assert_eq!(percentile(&sorted, 0.95), 5.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    /// The stamp is parsed by the harness, so a backslash in a Windows path must
    /// come out escaped rather than as an invalid JSON escape sequence.
    #[test]
    fn paths_are_escaped_for_json() {
        assert_eq!(
            json_string(r"D:\Dev\3d-review-rs\a.fbx"),
            r#""D:\\Dev\\3d-review-rs\\a.fbx""#
        );
    }

    #[test]
    fn empty_series_still_produces_a_parseable_object() {
        assert!(stats_json(&[]).contains("\"median\": 0.0"));
    }
}
