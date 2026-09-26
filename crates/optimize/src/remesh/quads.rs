//! Pairing the rebuilt triangles into quads.
//!
//! Everything before this stage produces triangles, and produces them well: even
//! in size, on the surface, wound the same way all over. This stage does not
//! move a single vertex. It decides, for each pair of triangles sharing an edge,
//! whether that edge is real - whether the two are two halves of one quad the
//! surface wanted, or two faces that genuinely meet at an angle - and rubs out
//! the ones that are not.
//!
//! That is the whole operation, and the reason it is worth having is what a quad
//! is *for*. A quad mesh is not better geometry; it is the same geometry, said in
//! the form every tool downstream of this one expects. A subdivision surface
//! needs quads. A deformation rig wants edge loops that run along a limb rather
//! than zig-zagging across it. An artist opening the export in Maya or Blender
//! wants to select a ring and slide it. None of that is available from a
//! triangle soup, however well shaped.
//!
//! ## The geometry does not change, and that is arranged rather than hoped
//!
//! A quad is a *planar* primitive and two triangles sharing an edge are
//! generally not planar, so everything that reads a quad has to choose a
//! diagonal to split it on, and the two choices are different surfaces.
//!
//! Ours chooses the one the pair already had. A merged quad carries its own
//! diagonal in its corner order, `super::layout::canonicalize` is restricted to
//! the rotations that keep it there, and [`super::fan`] splits on it - so the
//! triangles the viewport draws are the triangles that were there before the
//! merge, corner for corner. A quad rebuild is a triangle rebuild with a face
//! table over it: same vertices, same silhouette, same shading.
//!
//! That leaves what a *different* tool will do with the file, since it has a
//! rule of its own, and the answer is [`MERGE_WARP`]: a pair is merged only when
//! the four corners are nearly coplanar, measured as how far a corner stands out
//! of the plane of the other three against the quad's own size. Below that the
//! two splits describe the same surface to within a fraction of the faceting the
//! rebuild already has. A crease, a fold, the rim of a leaf - anything with a
//! real angle across it - fails by construction and stays two triangles, which
//! is right on every count: the shape is kept, the shading is kept, and the edge
//! stays where an artist can see it.
//!
//! **Not a dihedral angle**, which is the obvious way to write this and is wrong:
//! two long thin triangles meeting along their long edge can stand at a hundred
//! and sixty degrees across their *short* diagonal while every corner is a
//! hair out of plane. The angle is large and the surface is flat. Measured on a
//! plant, reading the angle instead of the distance passed quads bent 145
//! degrees.
//!
//! ## What makes one pairing better than another
//!
//! Each interior edge is one candidate, and no triangle can be in two quads, so
//! choosing is a matching problem on the dual graph. Two things score it:
//!
//! * **Rectangularity** - how close the four corners come to right angles, which
//!   is the standard measure ([`rectangularity`]) and is what stops a pair of
//!   slivers merging into a longer sliver.
//! * **Alignment** - whether the quad's axis runs along the [cross
//!   field](super::cross_field), which is what makes neighbouring quads agree on
//!   a direction instead of each being locally reasonable and collectively a
//!   patchwork.
//!
//! The second is why the field was built, and it is the one thing here a purely
//! local rule cannot do. A triangle mesh could not spend it - `cross_field`'s
//! module doc has the four measurements - because a triangle vertex wants six
//! neighbours and a cross offers four directions. A quad vertex wants exactly
//! four, so the mismatch that defeated it there is gone.
//!
//! ## Taking the best first is not the same as taking the most
//!
//! Greedy over the sorted candidates leaves a surprising number of triangles
//! behind: a pair taken early can be the only partner two other triangles had,
//! and each of those is then stranded for good. Measured on a plant, greedy
//! alone paired 70 % of the faces where nearly every interior edge was a legal
//! candidate.
//!
//! [`augment`] is the repair, and it is the shortest case of the standard one. A
//! stranded triangle beside a quad whose *other* half is beside a second
//! stranded triangle is three faces that can be rearranged into two quads: break
//! the quad, and re-pair each half with the triangle next to it. That is an
//! augmenting path of length three, and finding only those - never the longer
//! ones, which is where the real machinery starts - closes most of the gap. What
//! is left over after it is genuinely unpairable, which is the honest place for a
//! triangle to be.

use crate::cancel::{CancelToken, cancelled};

use super::RemeshOutput;
use super::cleanup::Quad;
use super::topology::Topology;

/// How far out of plane a quad's corner may stand, as a fraction of the quad's
/// mean edge length.
///
/// Two fifths, which on a square is about twenty-two degrees of fold. Stricter
/// than Blender's forty-degree default for the same job, and measured rather
/// than chosen: this setting alone decides how much of a curved object can be
/// quadrangulated at all, because a rebuilt surface is faceted everywhere it
/// curves. Across four fixtures at half density, the quad share runs
///
/// | warp | plant | pillar | stones | column |
/// |------|-------|--------|--------|--------|
/// | 0.15 |  65 % |   47 % |   76 % |   27 % |
/// | 0.20 |  71 % |   55 % |   81 % |   33 % |
/// | 0.30 |  77 % |   68 % |   85 % |   41 % |
/// | 0.40 |  81 % |   74 % |   87 % |   47 % |
/// | 0.60 |  85 % |   83 % |   88 % |   57 % |
///
/// The gain never stops outright, so there is no knee to point at - it is a
/// trade of quads against how much a quad means, and this is where the quads
/// are still flat enough that any tool's choice of diagonal lands in the same
/// place. Our own does not have to choose: see the module doc.
const MERGE_WARP: f64 = 0.40;

/// How much of a candidate's score is its agreement with the direction field,
/// the rest being its shape alone.
///
/// Half. The two want different things often enough to be worth balancing: shape
/// alone gives locally handsome quads facing every which way, and alignment
/// alone will merge a pair of slivers because they happen to point the right
/// way.
const ALIGNMENT_WEIGHT: f64 = 0.5;

/// What a candidate scores for alignment where the field has nothing to say.
///
/// The mean of what a direction picked at random would score, so a vertex the
/// field could not reach neither helps nor hurts the pair it is in.
const NO_OPINION: f64 = 0.5;

/// No quad.
const UNPAIRED: u32 = u32::MAX;

/// What the merge did.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Report {
    /// Faces that came out with four corners.
    pub(crate) quads: usize,
    /// Faces left as triangles, because no pairing was available or none was
    /// defensible.
    pub(crate) triangles: usize,
}

/// One pair of triangles that could become one quad.
struct Candidate {
    /// Higher is better. See [`score`].
    score: f64,
    /// The two faces, ascending.
    faces: [u32; 2],
    /// The merged quad, wound to match the pair it replaces.
    corners: [u32; 4],
}

impl Candidate {
    /// The face of this pair that is not `face`.
    fn other_than(&self, face: u32) -> u32 {
        if self.faces[0] == face {
            self.faces[1]
        } else {
            self.faces[0]
        }
    }
}

/// Merge `output`'s triangles into quads where the surface supports it.
///
/// `field` is one direction per output vertex, as
/// [`super::cross_field::CrossField`] gave it for the input vertex that vertex
/// came from; a zero entry means the field had nothing to say there. Passing an
/// empty slice is allowed and scores every candidate on shape alone.
pub(crate) fn merge(
    output: &mut RemeshOutput,
    field: &[[f32; 3]],
    cancel: Option<&CancelToken>,
) -> Report {
    let _z = crate::prof::zone!("Remesh Quads");
    let face_count = output.face_count();
    if face_count == 0 || output.positions.is_empty() {
        return Report::default();
    }
    debug_assert!(
        output
            .face_offsets
            .windows(2)
            .all(|window| window[1] - window[0] == 3),
        "the merge reads a pure triangle mesh"
    );
    let untouched = Report {
        quads: 0,
        triangles: face_count,
    };

    let topology = Topology::build(&output.corners, output.positions.len(), 1, cancel);
    if cancelled(cancel) {
        return untouched;
    }

    let mut candidates = gather(output, &topology, field);
    if cancelled(cancel) {
        return untouched;
    }
    // Best first, and on a tie the lower pair of faces - so the result is a
    // function of the mesh and not of the order the vertices were walked in.
    candidates.sort_by(|one, other| {
        other
            .score
            .total_cmp(&one.score)
            .then_with(|| one.faces.cmp(&other.faces))
    });

    let mut chosen = take_greedily(&candidates, face_count);
    augment(&candidates, face_count, &mut chosen);
    rewrite(output, &candidates, &chosen)
}

/// Every pair of triangles that could legally become a quad, scored.
fn gather(output: &RemeshOutput, topology: &Topology, field: &[[f32; 3]]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let mut edges = Vec::new();
    for vertex in 0..output.positions.len() as u32 {
        topology.edges_at(&output.corners, vertex, &mut edges);
        for edge in &edges {
            // Each undirected edge once, from its lower end; and a quad needs
            // exactly two faces to be made of.
            if edge.other <= vertex || edge.uses != 2 {
                continue;
            }
            // The winding is *read* from the two faces rather than assumed. The
            // same reading the flip pass uses, for the same reason: guessing it
            // turns a triangle over every time the guess is wrong, and an
            // inside-out face is a black patch. See `cleanup::Quad`.
            let Some(quad) = Quad::read(output, edge.faces[0], edge.faces[1], vertex, edge.other)
            else {
                continue;
            };
            let corners = quad.merged();
            let Some(score) = score(output, field, corners) else {
                continue;
            };
            candidates.push(Candidate {
                score,
                faces: edge.faces,
                corners,
            });
        }
    }
    candidates
}

/// Take candidates best first, skipping any whose faces are already spoken for.
///
/// Returns, per face, the candidate it belongs to or [`UNPAIRED`].
fn take_greedily(candidates: &[Candidate], face_count: usize) -> Vec<u32> {
    let mut chosen = vec![UNPAIRED; face_count];
    for (at, candidate) in candidates.iter().enumerate() {
        let [first, second] = candidate.faces.map(|face| face as usize);
        if chosen[first] != UNPAIRED || chosen[second] != UNPAIRED {
            continue;
        }
        let at = at as u32;
        chosen[first] = at;
        chosen[second] = at;
    }
    chosen
}

/// Re-pair around the quads that stranded a triangle. See the module doc.
fn augment(candidates: &[Candidate], face_count: usize, chosen: &mut [u32]) {
    // Which candidates touch a face, best first - the same order the greedy pass
    // took them in, so a reroute prefers the better quad here too.
    let mut counts = vec![0u32; face_count + 1];
    for candidate in candidates {
        for face in candidate.faces {
            counts[face as usize + 1] += 1;
        }
    }
    for at in 1..counts.len() {
        counts[at] += counts[at - 1];
    }
    let starts = counts;
    let mut fill = starts.clone();
    let mut touching = vec![0u32; candidates.len() * 2];
    for (at, candidate) in candidates.iter().enumerate() {
        for face in candidate.faces {
            touching[fill[face as usize] as usize] = at as u32;
            fill[face as usize] += 1;
        }
    }
    let touches = Touches { starts, touching };

    // Each round pairs at least one more face, so this cannot run away; in
    // practice it is two or three.
    loop {
        let mut gained = 0usize;
        for face in 0..face_count as u32 {
            if chosen[face as usize] != UNPAIRED {
                continue;
            }
            gained += usize::from(reroute(candidates, chosen, &touches, face));
        }
        if gained == 0 {
            break;
        }
    }
}

/// Which candidates touch each face, as a CSR - best first, since it is built
/// from the sorted candidate list.
struct Touches {
    starts: Vec<u32>,
    touching: Vec<u32>,
}

impl Touches {
    fn at(&self, face: u32) -> &[u32] {
        let face = face as usize;
        &self.touching[self.starts[face] as usize..self.starts[face + 1] as usize]
    }
}

/// Find `face` a partner, breaking one existing quad to do it if that is what it
/// takes. Whether it found one.
fn reroute(candidates: &[Candidate], chosen: &mut [u32], touches: &Touches, face: u32) -> bool {
    for &at in touches.at(face) {
        let neighbour = candidates[at as usize].other_than(face);
        let held = chosen[neighbour as usize];
        if held == UNPAIRED {
            // A partner freed up by an earlier reroute. Nothing to break.
            chosen[face as usize] = at;
            chosen[neighbour as usize] = at;
            return true;
        }

        // The other half of the quad this neighbour is in. Breaking that quad
        // only pays if that half can find a stranded triangle of its own.
        let far = candidates[held as usize].other_than(neighbour);
        for &other in touches.at(far) {
            let stranded = candidates[other as usize].other_than(far);
            if stranded == face || chosen[stranded as usize] != UNPAIRED {
                continue;
            }
            chosen[face as usize] = at;
            chosen[neighbour as usize] = at;
            chosen[far as usize] = other;
            chosen[stranded as usize] = other;
            return true;
        }
    }
    false
}

/// Write the matching back as the mesh.
fn rewrite(output: &mut RemeshOutput, candidates: &[Candidate], chosen: &[u32]) -> Report {
    let face_count = output.face_count();
    let mut report = Report::default();
    let mut rebuilt = RemeshOutput {
        positions: std::mem::take(&mut output.positions),
        face_offsets: Vec::with_capacity(face_count + 1),
        corners: Vec::with_capacity(output.corners.len()),
    };
    rebuilt.face_offsets.push(0);
    // In face order, each quad standing where the lower of its two triangles
    // stood.
    for (face, &held) in chosen.iter().enumerate() {
        match held {
            UNPAIRED => {
                rebuilt.corners.extend_from_slice(output.face(face));
                report.triangles += 1;
            }
            at if candidates[at as usize].faces[0] as usize == face => {
                rebuilt
                    .corners
                    .extend_from_slice(&candidates[at as usize].corners);
                report.quads += 1;
            }
            // The other half of a quad already written.
            _ => continue,
        }
        rebuilt
            .face_offsets
            .push(rebuilt.corners.len().try_into().unwrap_or(u32::MAX));
    }

    *output = rebuilt;
    report
}

/// How good a quad this pair would make, or `None` if it would not make one.
///
/// The corners arrive in order around the quad, with the shared edge as the
/// `0..2` diagonal - so the two triangles being weighed are `0,1,2` and `0,2,3`,
/// which is the pair that exists now.
fn score(output: &RemeshOutput, field: &[[f32; 3]], corners: [u32; 4]) -> Option<f64> {
    let points = corners.map(|corner| point_of(output, corner));

    // The two halves as they stand. Either being degenerate means there is no
    // quad here to speak of.
    let first = cross(sub(points[1], points[0]), sub(points[2], points[0]));
    let second = cross(sub(points[2], points[0]), sub(points[3], points[0]));
    let (one, other) = (unit(first)?, unit(second)?);
    let normal = unit(add(one, other))?;

    if warp(&points, first, second)? > MERGE_WARP {
        return None;
    }
    if !is_convex(&points, normal) {
        return None;
    }
    let shape = rectangularity(&points);
    if shape <= 0.0 {
        return None;
    }

    let alignment = alignment(&points, corners, field, normal);
    Some(shape * (1.0 - ALIGNMENT_WEIGHT + ALIGNMENT_WEIGHT * alignment))
}

/// How far the quad's corners stand out of each other's plane, as a fraction of
/// its mean edge length. Zero for a flat quad.
///
/// `first` and `second` are the two halves' unnormalized normals, whose lengths
/// are twice their areas - so the tetrahedron's volume over each gives the
/// distance from the opposite corner to that half's plane directly, and the
/// larger of the two distances is the one that matters.
fn warp(points: &[[f64; 3]; 4], first: [f64; 3], second: [f64; 3]) -> Option<f64> {
    let mut perimeter = 0.0;
    for corner in 0..4 {
        let edge = sub(points[(corner + 1) % 4], points[corner]);
        perimeter += dot(edge, edge).sqrt();
    }
    let size = perimeter / 4.0;
    if size <= 0.0 {
        return None;
    }

    // Six times the volume of the tetrahedron the four corners make, which is
    // zero exactly when they are coplanar.
    let volume = dot(
        cross(sub(points[1], points[0]), sub(points[2], points[0])),
        sub(points[3], points[0]),
    )
    .abs();
    let smaller = dot(first, first).sqrt().min(dot(second, second).sqrt());
    if smaller <= 0.0 {
        return None;
    }
    Some(volume / smaller / size)
}

/// Whether the four corners turn the same way all the way round.
///
/// A quad with a reflex corner is one whose two diagonals do not both lie inside
/// it, so the two ways of splitting it are not merely different surfaces but
/// different *outlines*: the far one cuts across open air. The pair stays two
/// triangles.
fn is_convex(points: &[[f64; 3]; 4], normal: [f64; 3]) -> bool {
    (0..4).all(|corner| {
        let previous = points[(corner + 3) % 4];
        let here = points[corner];
        let next = points[(corner + 1) % 4];
        dot(cross(sub(here, previous), sub(next, here)), normal) > 0.0
    })
}

/// How close the quad's worst corner comes to a right angle, 1 for a rectangle
/// and 0 for a corner folded flat.
///
/// The worst corner rather than the average, because a quad is only as good as
/// its worst: three right angles and one of five degrees is a sliver with a
/// respectable mean.
fn rectangularity(points: &[[f64; 3]; 4]) -> f64 {
    let mut worst: f64 = 0.0;
    for corner in 0..4 {
        let previous = points[(corner + 3) % 4];
        let here = points[corner];
        let next = points[(corner + 1) % 4];
        let (Some(back), Some(forward)) = (unit(sub(previous, here)), unit(sub(next, here))) else {
            return 0.0;
        };
        let angle = dot(back, forward).clamp(-1.0, 1.0).acos();
        worst = worst.max((angle - std::f64::consts::FRAC_PI_2).abs());
    }
    (1.0 - worst / std::f64::consts::FRAC_PI_2).max(0.0)
}

/// How well the quad's axis lines up with the direction field, 1 along an arm of
/// the cross and 0 at forty-five degrees to every arm.
///
/// The axis is the average of the two opposite edges that run `0 -> 1` and
/// `3 -> 2`. Only one axis is measured because the other is at right angles to
/// it, and a cross cannot tell the two apart - lining up one lines up both.
fn alignment(
    points: &[[f64; 3]; 4],
    corners: [u32; 4],
    field: &[[f32; 3]],
    normal: [f64; 3],
) -> f64 {
    let Some(axis) = unit(add(sub(points[1], points[0]), sub(points[2], points[3]))) else {
        return NO_OPINION;
    };

    let mut total = 0.0;
    let mut seen = 0u32;
    for corner in corners {
        let Some(direction) = field.get(corner as usize).copied() else {
            continue;
        };
        let direction = [
            f64::from(direction[0]),
            f64::from(direction[1]),
            f64::from(direction[2]),
        ];
        // The cross's frame in the quad's own plane. A field direction is in the
        // *input* vertex's tangent plane, which is near enough this one to
        // project onto but not to assume it already lies in.
        let Some(along) = unit(sub(direction, scale(normal, dot(direction, normal)))) else {
            continue;
        };
        let across = cross(normal, along);

        // The angle from the axis to the cross, four times over: a cross repeats
        // every quarter turn, so `cos 4t` is 1 on any arm of it and -1 at
        // forty-five degrees to all four, with nothing in between to
        // distinguish an arm from its opposite.
        let (u, v) = (dot(axis, along), dot(axis, across));
        let square = u * u + v * v;
        if square <= 0.0 {
            continue;
        }
        let double = (u * u - v * v) / square;
        let twist = 2.0 * u * v / square;
        total += ((double * double - twist * twist) + 1.0) * 0.5;
        seen += 1;
    }

    if seen == 0 {
        NO_OPINION
    } else {
        total / f64::from(seen)
    }
}

fn point_of(output: &RemeshOutput, vertex: u32) -> [f64; 3] {
    let at = output.positions[vertex as usize];
    [f64::from(at.x), f64::from(at.y), f64::from(at.z)]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f64; 3], by: f64) -> [f64; 3] {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(vector, vector).sqrt();
    (length > 0.0).then(|| scale(vector, 1.0 / length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// A flat grid of `side` x `side` quads, each split into two triangles - the
    /// mesh a perfect merge turns back into exactly `side * side` quads.
    fn grid(side: u32) -> RemeshOutput {
        let span = side + 1;
        let mut positions = Vec::new();
        for row in 0..span {
            for column in 0..span {
                positions.push(Vec3::new(column as f32, row as f32, 0.0));
            }
        }
        let mut corners = Vec::new();
        for row in 0..side {
            for column in 0..side {
                let at = row * span + column;
                corners.extend_from_slice(&[at, at + 1, at + span]);
                corners.extend_from_slice(&[at + 1, at + span + 1, at + span]);
            }
        }
        RemeshOutput {
            positions,
            face_offsets: (0..=(corners.len() as u32 / 3))
                .map(|face| face * 3)
                .collect(),
            corners,
        }
    }

    fn degrees(faces: &RemeshOutput) -> Vec<usize> {
        (0..faces.face_count())
            .map(|face| faces.face(face).len())
            .collect()
    }

    #[test]
    fn a_split_grid_comes_back_as_quads() {
        let mut output = grid(6);
        let before = output.face_count();

        let report = merge(&mut output, &[], None);

        assert_eq!(report.quads, before / 2, "every pair merges");
        assert_eq!(report.triangles, 0);
        assert!(degrees(&output).iter().all(|&degree| degree == 4));
        assert!(output.validate().is_ok());
    }

    #[test]
    fn a_quad_is_wound_the_way_its_triangles_were() {
        let mut output = grid(3);

        merge(&mut output, &[], None);

        // Every face faces the same way the flat grid did, which is what says
        // the corner order came out of the mesh rather than an assumption.
        for face in 0..output.face_count() {
            let corners = output.face(face);
            let at = |corner: usize| point_of(&output, corners[corner]);
            let normal = cross(sub(at(1), at(0)), sub(at(2), at(0)));
            assert!(normal[2] > 0.0, "face {face} is inside out: {normal:?}");
        }
    }

    #[test]
    fn a_folded_pair_stays_two_triangles() {
        // Two triangles sharing an edge, bent ninety degrees across it: a
        // crease, and no quad describes it.
        let mut output = RemeshOutput {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            face_offsets: vec![0, 3, 6],
            corners: vec![0, 1, 2, 1, 0, 3],
        };

        let report = merge(&mut output, &[], None);

        assert_eq!(report.quads, 0);
        assert_eq!(report.triangles, 2);
    }

    #[test]
    fn a_pair_that_would_make_a_dart_stays_two_triangles() {
        // The fourth corner sits inside the triangle of the other three, so the
        // "quad" has a reflex corner and one of its diagonals runs outside it.
        let mut output = RemeshOutput {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(1.0, 0.25, 0.0),
            ],
            face_offsets: vec![0, 3, 6],
            corners: vec![0, 1, 2, 1, 0, 3],
        };

        let report = merge(&mut output, &[], None);

        assert_eq!(report.quads, 0, "a dart is not a quad");
    }

    #[test]
    fn a_merged_quad_keeps_the_diagonal_it_was_built_on() {
        // A flat rhombus cut across its *long* diagonal. The merge takes it -
        // there is nothing wrong with it - and what matters is that the quad it
        // writes still names that diagonal first, because that is what the fan
        // will split it back on.
        let mut output = RemeshOutput {
            positions: vec![
                Vec3::new(-2.0, 0.0, 0.0),
                Vec3::new(0.0, -0.5, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(0.0, 0.5, 0.0),
            ],
            face_offsets: vec![0, 3, 6],
            // Sharing the long 0..2 diagonal.
            corners: vec![0, 1, 2, 2, 3, 0],
        };

        let report = merge(&mut output, &[], None);

        assert_eq!(report.quads, 1);
        let corners = output.face(0);
        assert!(
            (corners[0] == 0 && corners[2] == 2) || (corners[0] == 2 && corners[2] == 0),
            "the long diagonal is still the 0..2 one: {corners:?}"
        );
    }

    #[test]
    fn a_warped_quad_is_refused_and_a_bent_thin_one_is_not() {
        // Both pairs stand at a steep angle across their short diagonal. The
        // first is a square folded for real; the second is two long slivers
        // whose corners are all but coplanar, and which a dihedral test would
        // refuse for no reason. Only the first is a genuine fold.
        let folded = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.8],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let thin = [
            [0.0, 0.0, 0.0],
            [4.0, 0.02, 0.001],
            [8.0, 0.0, 0.0],
            [4.0, -0.02, -0.001],
        ];

        for (points, refused) in [(folded, true), (thin, false)] {
            let first = cross(sub(points[1], points[0]), sub(points[2], points[0]));
            let second = cross(sub(points[2], points[0]), sub(points[3], points[0]));
            let measured = warp(&points, first, second).expect("both have area");
            assert_eq!(
                measured > MERGE_WARP,
                refused,
                "{points:?} measured {measured:.3}"
            );
        }
    }

    #[test]
    fn splitting_the_quads_back_gives_the_triangles_that_were_there() {
        // The claim the whole design rests on: a merge is a face table over the
        // mesh, not an edit of it. Split every quad on the diagonal its corner
        // order names and the original triangles come back, every one of them.
        let mut output = grid(5);
        let before = triangle_set(&output);

        merge(&mut output, &[], None);

        let mut after: Vec<[u32; 3]> = Vec::new();
        for face in 0..output.face_count() {
            let corners = output.face(face);
            match corners.len() {
                3 => after.push(sorted([corners[0], corners[1], corners[2]])),
                4 => {
                    after.push(sorted([corners[0], corners[1], corners[2]]));
                    after.push(sorted([corners[0], corners[2], corners[3]]));
                }
                other => panic!("a face with {other} corners"),
            }
        }
        after.sort_unstable();
        assert_eq!(after, before);
    }

    /// Every face as a sorted corner triple, sorted - a set to compare against.
    fn triangle_set(output: &RemeshOutput) -> Vec<[u32; 3]> {
        let mut all: Vec<[u32; 3]> = (0..output.face_count())
            .map(|face| {
                let corners = output.face(face);
                sorted([corners[0], corners[1], corners[2]])
            })
            .collect();
        all.sort_unstable();
        all
    }

    fn sorted(mut corners: [u32; 3]) -> [u32; 3] {
        corners.sort_unstable();
        corners
    }

    #[test]
    fn a_stranded_triangle_is_re_paired_around_the_quad_beside_it() {
        // Three quads' worth of grid is six triangles, and greedy can leave two
        // of them stranded either side of one quad. The augmenting pass is what
        // turns three faces back into two quads; without it this comes back with
        // triangles in it.
        let mut output = grid(4);

        let report = merge(&mut output, &[], None);

        assert_eq!(report.triangles, 0, "nothing is left stranded on a grid");
    }

    #[test]
    fn a_mesh_with_no_interior_edge_is_left_alone() {
        let mut output = RemeshOutput {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            face_offsets: vec![0, 3],
            corners: vec![0, 1, 2],
        };

        let report = merge(&mut output, &[], None);

        assert_eq!(report.triangles, 1);
        assert_eq!(report.quads, 0);
        assert_eq!(output.corners, vec![0, 1, 2]);
    }

    #[test]
    fn the_merge_is_the_same_however_the_faces_are_numbered() {
        // The same grid with its faces listed back to front: the order the
        // candidates are taken in must come from the scores and the geometry,
        // not from the face table.
        let forward = grid(5);
        let mut reversed = RemeshOutput {
            positions: forward.positions.clone(),
            face_offsets: forward.face_offsets.clone(),
            corners: Vec::new(),
        };
        for face in (0..forward.face_count()).rev() {
            reversed.corners.extend_from_slice(forward.face(face));
        }

        let mut one = forward;
        let mut other = reversed;
        let first = merge(&mut one, &[], None);
        let second = merge(&mut other, &[], None);

        assert_eq!(first.quads, second.quads);
        assert_eq!(first.triangles, second.triangles);
    }
}
