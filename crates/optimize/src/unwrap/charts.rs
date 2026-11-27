//! The charts each island is flattened in: as few and as large as flatten
//! within the stretch limit, their seams placed where they show least.
//!
//! An island the fixed seams bound is tried whole first. Where that fails, its
//! seams are *planned* ([`super::cuts`]): opened if it is closed or has a
//! handle, then relieved with branches from where it stretches most, and split
//! in two only if branching cannot bring it within the limit. The seams cost
//! least along creases, in valleys and crevices, and along the existing UVs'
//! own seams ([`super::cost`]), and follow edge loops rather than stepping
//! across them ([`super::paths`]).
//!
//! Each chart is flattened by LSCM and, for minimum stretch, refined by
//! as-rigid-as-possible passes; a chart that is kept gets the full count and
//! comes out in model units ([`super::flatten`]).
//!
//! **Limits.** A chart is within the limit when its area-weighted mean stretch
//! is at most [`Settings::max_stretch`], and all but the worst
//! [`super::flatten::TAIL_SHARE`] of its area stretch (σ₁/σ₂) at most
//! [`TAIL_FACTOR`] times as far past 1 ([`Settings::tail_limit`]).
//!
//! Islands are independent, so they are planned side by side, a share of the
//! object's threads each; every result is a function of its island alone and
//! the charts come back in order of first triangle, so nothing depends on the
//! scheduling.

use std::collections::BTreeMap;

use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use glam::{Vec2, Vec3};

use super::cuts::{Outcome, Planner, split_to_size};
use super::flatten::Flat;
use crate::cancel::CancelToken;
use crate::stack::Flattening;

/// A round of branches is kept when it brings an island at least this share
/// nearer the limit.
const MIN_GAIN: f64 = 0.005;

/// What a fold counts for against stretch: a tenth of the area folded is as
/// bad as the mean at twice the limit.
const FOLD_WEIGHT: f64 = 10.0;

/// How much further past 1 a single triangle's σ₁/σ₂ may go than the mean: at
/// the default limit of 1.2, 2:1. Measured on the monkey at 1.2 when it was chosen: ten (the
/// bound before) let 39 triangles past 2:1 for 13 islands, five 16 for 15,
/// three none for 25 — five buys most of the evenness for few seams.
const TAIL_FACTOR: f64 = 5.0;

/// The surface the charts are cut from.
pub(super) struct CutMesh<'a> {
    /// Per cut vertex, its position.
    pub positions: &'a [Vec3],
    /// Per triangle, its three cut vertices.
    pub triangles: &'a [[u32; 3]],
    /// Per triangle, its island.
    pub island: &'a [u32],
    /// Per triangle, its face: the triangles of one polygon share one, and
    /// no seam runs between them.
    pub face: &'a [u32],
    /// Per triangle, a bit per edge the reference UVs are cut along.
    pub preferred: &'a [u8],
}

/// A chart: its triangles, and per triangle its corners' UVs, in model units.
pub(super) struct Chart {
    pub triangles: Vec<u32>,
    pub uvs: Vec<[Vec2; 3]>,
}

/// What the charts are measured and flattened by.
#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub flattening: Flattening,
    pub max_stretch: f32,
}

impl Settings {
    /// The most a triangle's own σ₁/σ₂ may be (all but a sliver of the area):
    /// [`TAIL_FACTOR`] times as far past 1 as the limit.
    pub(super) fn tail_limit(&self) -> f64 {
        1.0 + TAIL_FACTOR * (f64::from(self.max_stretch) - 1.0)
    }

    /// Whether a flattening is one-to-one and within the limit.
    pub(super) fn accepts(&self, flat: &Flat) -> bool {
        flat.valid && flat.mean <= f64::from(self.max_stretch) && flat.tail <= self.tail_limit()
    }

    /// How many times the limit a flattening stretches, whichever of the two
    /// is further past: 1 at the limit.
    pub(super) fn excess(&self, flat: &Flat) -> f64 {
        (flat.mean / f64::from(self.max_stretch)).max(flat.tail / self.tail_limit())
    }

    /// Whether `new` is nearer acceptable than `old`: one-to-one where `old`
    /// was not, or at least [`MIN_GAIN`] less bad ([`Self::badness`]).
    pub(super) fn better(&self, new: &Flat, old: &Flat) -> bool {
        match (new.valid, old.valid) {
            (true, false) => true,
            (false, true) => false,
            _ => self.badness(new) < self.badness(old) * (1.0 - MIN_GAIN),
        }
    }

    /// How bad a flattening is, as a sum that falls with every kind of
    /// progress: the mean and the tail each against its limit, and the folded
    /// share of the area heavily — so a round that halves the mean counts even
    /// while a fold elsewhere holds [`Self::excess`] at its ceiling.
    fn badness(&self, flat: &Flat) -> f64 {
        flat.mean / f64::from(self.max_stretch)
            + flat.tail / self.tail_limit()
            + FOLD_WEIGHT * flat.folded
    }
}

/// The charts of `mesh`, planned on up to `threads` threads, or `None` once
/// `cancel` says the run is no longer wanted.
pub(super) fn charts(
    mesh: &CutMesh<'_>,
    settings: Settings,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> Option<Vec<Chart>> {
    let mut by_island: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for triangle in 0..mesh.triangles.len() as u32 {
        by_island
            .entry(mesh.island[triangle as usize])
            .or_default()
            .push(triangle);
    }
    let mut jobs: Vec<Vec<u32>> = Vec::new();
    for triangles in by_island.into_values() {
        split_to_size(mesh, triangles, &mut jobs, cancel)?;
    }

    let occluders = OnceLock::new();
    let planner = Planner {
        mesh,
        settings,
        occluders: &occluders,
        cancel,
    };
    let mut done = run(jobs, threads, |island| planner.island(island))?;
    // In order of first triangle, so the layout's order is the mesh's.
    done.sort_by_key(|chart| chart.triangles[0]);
    Some(done)
}

/// The islands still to plan, and how many are being planned.
struct Work {
    pending: Vec<Vec<u32>>,
    busy: usize,
    charts: Vec<Chart>,
    cancelled: bool,
}

/// Plan every island on up to `threads` threads, the pieces a split makes
/// going back on the one queue — so the pieces of one large island spread
/// over every thread rather than queueing behind each other on the one that
/// split it. `None` once `plan` says the run is no longer wanted. The charts
/// come back in no particular order; each is a function of its island alone.
fn run(
    jobs: Vec<Vec<u32>>,
    threads: usize,
    plan: impl Fn(&[u32]) -> Option<Outcome> + Sync,
) -> Option<Vec<Chart>> {
    let mut pending = jobs;
    // Largest last, so it is taken first.
    pending.sort_by(|a, b| a.len().cmp(&b.len()).then(b[0].cmp(&a[0])));
    let work = Mutex::new(Work {
        pending,
        busy: 0,
        charts: Vec::new(),
        cancelled: false,
    });
    let changed = Condvar::new();
    let worker = || {
        loop {
            let island = {
                let mut state = work.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if state.cancelled {
                        break None;
                    }
                    if let Some(island) = state.pending.pop() {
                        state.busy += 1;
                        break Some(island);
                    }
                    if state.busy == 0 {
                        break None;
                    }
                    state = changed.wait(state).unwrap_or_else(PoisonError::into_inner);
                }
            };
            let Some(island) = island else {
                changed.notify_all();
                return;
            };
            let outcome = plan(&island);
            let mut state = work.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy -= 1;
            match outcome {
                Some(Outcome::Charts(found)) => state.charts.extend(found),
                Some(Outcome::Split(pieces)) => state.pending.extend(pieces),
                None => state.cancelled = true,
            }
            changed.notify_all();
        }
    };
    let workers = threads.max(1);
    if workers == 1 {
        worker();
    } else {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(worker);
            }
        });
    }
    let state = work.into_inner().unwrap_or_else(PoisonError::into_inner);
    (!state.cancelled).then_some(state.charts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube's eight corners and twelve triangles, wound outwards.
    fn cube() -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let positions = (0..8)
            .map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32))
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let triangles = quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .collect();
        (positions, triangles)
    }

    fn run_charts(
        positions: &[Vec3],
        triangles: &[[u32; 3]],
        quads: bool,
        max_stretch: f32,
    ) -> Vec<Chart> {
        let island = vec![0; triangles.len()];
        let face: Vec<u32> = (0..triangles.len() as u32)
            .map(|triangle| if quads { triangle / 2 } else { triangle })
            .collect();
        let preferred = vec![0u8; triangles.len()];
        let mesh = CutMesh {
            positions,
            triangles,
            island: &island,
            face: &face,
            preferred: &preferred,
        };
        let settings = Settings {
            flattening: Flattening::MinimumStretch,
            max_stretch,
        };
        charts(&mesh, settings, 1, None).expect("not cancelled")
    }

    /// Every triangle in one chart, unfolded without stretch.
    fn assert_unfolded(charts: &[Chart], count: usize) {
        let mut seen = vec![0; count];
        for chart in charts {
            for (&triangle, uv) in chart.triangles.iter().zip(&chart.uvs) {
                seen[triangle as usize] += 1;
                let [a, b, c] = *uv;
                let area = (b - a).perp_dot(c - a) * 0.5;
                assert!(
                    (area - 0.5).abs() < 1.0e-3,
                    "triangle {triangle}: UV area {area}, 3D area 0.5"
                );
            }
        }
        assert!(seen.iter().all(|&count| count == 1), "{seen:?}");
    }

    /// A closed cube is opened along its edges and unfolded flat, its faces
    /// unstretched — not shattered: a cube unfolds into a cross or a few
    /// strips, never twelve triangles.
    #[test]
    fn a_closed_cube_is_cut_open_along_its_edges() {
        let (positions, triangles) = cube();
        let charts = run_charts(&positions, &triangles, true, 1.1);
        assert_unfolded(&charts, triangles.len());
        assert!(charts.len() <= 3, "{} charts", charts.len());
    }

    /// A quad's two triangles are never parted, however the seams fall.
    #[test]
    fn no_seam_runs_inside_a_quad() {
        let (positions, triangles) = cube();
        for max_stretch in [1.0, 1.1, 1.5] {
            let charts = run_charts(&positions, &triangles, true, max_stretch);
            let mut chart_of = vec![usize::MAX; triangles.len()];
            for (at, chart) in charts.iter().enumerate() {
                for &triangle in &chart.triangles {
                    chart_of[triangle as usize] = at;
                }
            }
            for pair in chart_of.chunks(2) {
                assert_eq!(pair[0], pair[1], "a quad was cut at {max_stretch}");
            }
        }
    }

    /// An open tube is a disc with a hole, which lies flat only as a ring,
    /// stretched; bridged — cut along its length — it unrolls into one strip
    /// with next to no stretch, which is what an artist would do.
    #[test]
    fn a_tube_unrolls_into_one_strip() {
        let (segments, rings) = (16u32, 6u32);
        let mut positions = Vec::new();
        for ring in 0..=rings {
            for segment in 0..segments {
                let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
                positions.push(Vec3::new(angle.cos(), angle.sin(), ring as f32 * 0.4));
            }
        }
        let at = |ring: u32, segment: u32| ring * segments + segment % segments;
        let mut triangles = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let (a, b) = (at(ring, segment), at(ring, segment + 1));
                let (c, d) = (at(ring + 1, segment + 1), at(ring + 1, segment));
                triangles.push([a, b, c]);
                triangles.push([a, c, d]);
            }
        }
        let charts = run_charts(&positions, &triangles, true, 1.1);
        assert_eq!(charts.len(), 1, "one strip");
        let mut seen = vec![0; triangles.len()];
        for chart in &charts {
            for (&triangle, uv) in chart.triangles.iter().zip(&chart.uvs) {
                seen[triangle as usize] += 1;
                let [p, q, r] = triangles[triangle as usize].map(|v| positions[v as usize]);
                let stretch = super::super::layout::stretch([p, q, r], *uv).expect("an area");
                assert!(stretch < 1.05, "triangle {triangle} stretches {stretch}");
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    /// Islands and the pieces splits make go on one shared queue, taken in
    /// whatever order the threads get to them; the charts must not show it.
    /// A closed sphere is opened, branched and split, so every path runs.
    #[test]
    fn the_charts_do_not_depend_on_the_thread_count() {
        let (rings, segments) = (12u32, 24u32);
        let mut positions = vec![Vec3::new(0.0, 0.0, 1.0)];
        for ring in 1..rings {
            let polar = ring as f32 / rings as f32 * std::f32::consts::PI;
            for segment in 0..segments {
                let azimuth = segment as f32 / segments as f32 * std::f32::consts::TAU;
                positions.push(Vec3::new(
                    polar.sin() * azimuth.cos(),
                    polar.sin() * azimuth.sin(),
                    polar.cos(),
                ));
            }
        }
        positions.push(Vec3::new(0.0, 0.0, -1.0));
        let south = positions.len() as u32 - 1;
        let at = |ring: u32, segment: u32| 1 + (ring - 1) * segments + segment % segments;
        let mut triangles = Vec::new();
        for segment in 0..segments {
            triangles.push([0, at(1, segment), at(1, segment + 1)]);
            triangles.push([south, at(rings - 1, segment + 1), at(rings - 1, segment)]);
        }
        for ring in 1..rings - 1 {
            for segment in 0..segments {
                let (a, b) = (at(ring, segment), at(ring, segment + 1));
                let (c, d) = (at(ring + 1, segment + 1), at(ring + 1, segment));
                triangles.push([a, d, c]);
                triangles.push([a, c, b]);
            }
        }
        let island = vec![0; triangles.len()];
        let face: Vec<u32> = (0..triangles.len() as u32).collect();
        let preferred = vec![0u8; triangles.len()];
        let mesh = CutMesh {
            positions: &positions,
            triangles: &triangles,
            island: &island,
            face: &face,
            preferred: &preferred,
        };
        let settings = Settings {
            flattening: Flattening::MinimumStretch,
            max_stretch: 1.05,
        };
        let bits = |threads: usize| -> Vec<(Vec<u32>, Vec<[u32; 6]>)> {
            charts(&mesh, settings, threads, None)
                .expect("not cancelled")
                .into_iter()
                .map(|chart| {
                    let uvs = chart
                        .uvs
                        .iter()
                        .map(|corners| {
                            let [a, b, c] = corners.map(|uv| [uv.x.to_bits(), uv.y.to_bits()]);
                            [a[0], a[1], b[0], b[1], c[0], c[1]]
                        })
                        .collect();
                    (chart.triangles, uvs)
                })
                .collect()
        };
        let one = bits(1);
        assert!(one.len() > 1, "the sphere was cut into charts");
        assert_eq!(one, bits(8));
    }

    /// A band all the way round (the cube's four sides) is a disc with a hole
    /// at each end only once it is cut: it comes out unstretched.
    #[test]
    fn a_band_is_opened_and_unrolled() {
        let (positions, triangles) = cube();
        let sides: Vec<[u32; 3]> = triangles[4..].to_vec();
        let charts = run_charts(&positions, &sides, true, 1.1);
        assert_unfolded(&charts, sides.len());
    }
}
