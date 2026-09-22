//! Turning the regions into the output mesh.
//!
//! Each region becomes one vertex, and the way it gets there is by **collapsing
//! the edges inside it** — never by building a new surface. That is the whole
//! difference between this and the field extraction it replaces, and it is what
//! fixes the failure that prompted the rewrite.
//!
//! A collapse can only ever *remove* an edge from a mesh that was already valid.
//! It cannot open a hole, it cannot drop a border, and it cannot invent a face
//! where the input had none. A leaf one quad wide survives because the output
//! *is* the input with edges taken out of it — which is exactly why `Reduce`
//! always handled those and the old Remesh tore them.
//!
//! ## Constrained, which is the part that makes it a retopology
//!
//! An ordinary simplifier collapses the cheapest edge anywhere, and the mesh
//! drifts wherever the error metric leads. Here a collapse is **rejected when
//! its two endpoints are in different regions**, so the partition decides what
//! merges with what and the error metric only decides the order and the final
//! position. The regions are even and follow the size field
//! ([`super::partition`]), so the output is too.
//!
//! ## Three rules, each one a way the output could stop being a surface
//!
//! * **The link condition.** Collapsing an edge whose endpoints share a
//!   neighbour that is not part of a triangle with them folds the surface onto
//!   itself. The standard test: the common neighbours of the two endpoints must
//!   be exactly the third corners of the faces along the edge.
//! * **No normal flips.** A collapse that turns a face inside out leaves a
//!   visible spike. Every face that survives the collapse is checked — and so
//!   is the final placement below, which is a move like any other and was for a
//!   while the one that was not checked.
//! * **Features survive.** A border, crease or branching vertex is never
//!   collapsed *into* an ordinary one, and two separate feature chains are never
//!   merged. That is what holds an outline where it was.
//!
//! A region that cannot fully collapse keeps more than one vertex. That is the
//! manifold guarantee working: the output has a few more vertices than asked
//! for rather than a defect.
//!
//! ## Why the survivor does not move until the end
//!
//! Within a region the survivor keeps its own position, and only the last vertex
//! standing moves to the quadric's optimum. That makes every edge length in the
//! region *fixed*, so the order edges are collapsed in can be decided once, up
//! front, by sorting — instead of a priority queue that has to be re-scored
//! after every collapse.
//!
//! It also makes that last move the largest and the least constrained in the
//! whole rebuild, because by then every collapse has already pulled the
//! neighbourhood in tight around the vertex. See [`place_survivor`].

use glam::Vec3;

use crate::cancel::{CancelToken, cancelled};

use super::RemeshOutput;
use super::features::Features;
use super::partition::Partition;
use super::quadric::Quadric;
use super::seeds::Seeds;
use super::surface::Surface;

/// A face that has been collapsed away.
const DEAD: u32 = u32::MAX;

/// Stands in for "no vertex is dying" in [`would_flip`], which is how a plain
/// move is expressed as the collapse it is not: no face can contain it, so
/// nothing is skipped and nothing is renamed.
const NO_VERTEX: u32 = u32::MAX;

/// How far from its region's centre the quadric's answer may be, as a multiple
/// of the region's own extent. A quadric on a nearly flat patch can solve to a
/// point far off the surface; past this the seed's own position is the better
/// answer.
const OPTIMUM_REACH: f64 = 2.0;

/// Fractions of the way to the quadric's answer a region's last vertex is
/// offered, longest first, until one does not fold its fan. See
/// [`place_survivor`].
const OPTIMUM_BACKOFF: [f64; 4] = [1.0, 0.5, 0.25, 0.125];

/// The mesh mid-collapse: what is still alive, and what each face now says.
pub(crate) struct Collapsed {
    /// Face corners, rewritten as vertices merge. A dead face reads [`DEAD`].
    pub(crate) corners: Vec<u32>,
    /// Per input vertex: still part of the mesh.
    pub(crate) alive: Vec<bool>,
    /// Per input vertex: where it is now. Only a region's last survivor moves.
    pub(crate) positions: Vec<[f64; 3]>,
    /// Regions that did not come down to a single vertex.
    pub(crate) stubborn: usize,
}

impl Collapsed {
    fn face(&self, face: u32) -> [u32; 3] {
        let base = face as usize * 3;
        [
            self.corners[base],
            self.corners[base + 1],
            self.corners[base + 2],
        ]
    }

    fn is_dead(&self, face: u32) -> bool {
        self.corners[face as usize * 3] == DEAD
    }

    fn kill(&mut self, face: u32) {
        let base = face as usize * 3;
        self.corners[base..base + 3].fill(DEAD);
    }

    /// The vertices sharing a face with `vertex`, given its incidence.
    fn neighbours(&self, vertex: u32, incident: &[u32], out: &mut Vec<u32>) {
        out.clear();
        for &face in incident {
            if self.is_dead(face) {
                continue;
            }
            for corner in self.face(face) {
                if corner != vertex {
                    out.push(corner);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// Collapse every region down, as far as it will go.
pub(crate) fn run(
    surface: Surface<'_>,
    features: &Features,
    seeds: &Seeds,
    partition: &Partition,
    quadrics: &[Quadric],
    cancel: Option<&CancelToken>,
) -> Collapsed {
    let _z = crate::prof::zone!("Remesh Collapse");
    let count = surface.vertex_count();
    let mut state = Collapsed {
        corners: surface.indices.to_vec(),
        alive: (0..count)
            .map(|vertex| surface.topology.is_referenced(vertex as u32))
            .collect(),
        positions: (0..count as u32).map(|v| surface.position(v)).collect(),
        stubborn: 0,
    };

    // The incidence starts as the input's and grows as vertices absorb their
    // neighbours'. Only the vertices being collapsed need a mutable one: a
    // vertex outside the region keeps its faces, they just say something
    // different afterwards.
    let mut incidence: Vec<Vec<u32>> = (0..count as u32)
        .map(|vertex| surface.topology.faces_of(vertex).to_vec())
        .collect();

    for region in 0..partition.regions as u32 {
        if cancelled(cancel) {
            break;
        }
        collapse_region(
            surface,
            features,
            seeds,
            partition,
            quadrics,
            &mut state,
            &mut incidence,
            region,
        );
    }
    state
}

/// Collapse one region as far as the three rules allow.
#[allow(
    clippy::too_many_arguments,
    reason = "one region against the whole state"
)]
fn collapse_region(
    surface: Surface<'_>,
    features: &Features,
    seeds: &Seeds,
    partition: &Partition,
    quadrics: &[Quadric],
    state: &mut Collapsed,
    incidence: &mut [Vec<u32>],
    region: u32,
) {
    let members = partition.members_of(region);
    if members.len() < 2 {
        return;
    }

    // Collapsing *creates* adjacencies. A region shaped like a hub with spokes
    // has edges from the hub to each spoke and none between the spokes — so
    // once the hub is merged away, the spokes are neighbours that no list built
    // beforehand knows about, and the region stops half collapsed. (Measured on
    // a sphere: 446 faces where 200 were asked for.)
    //
    // So the edges are re-read from the live mesh each round, and a round that
    // collapses nothing is the end. Each round removes at least one vertex, so
    // this is bounded by the size of the region.
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut edges: Vec<(f64, u32, u32)> = Vec::new();
    for _ in 0..members.len() {
        live_edges(state, incidence, partition, members, region, &mut edges);
        if edges.is_empty() {
            break;
        }
        let mut collapsed = false;
        for &(_, a, b) in &edges {
            if !state.alive[a as usize] || !state.alive[b as usize] {
                continue;
            }
            let along = faces_along_edge(state, incidence, a, b);
            let Some((dying, survivor)) = survivor_of(surface, features, seeds, a, b, along) else {
                continue;
            };
            if !link_condition(state, incidence, dying, survivor, &mut left, &mut right) {
                continue;
            }
            let Some(at) = placement(state, incidence, quadrics, features, dying, survivor) else {
                continue;
            };
            state.positions[survivor as usize] = at;
            apply(state, incidence, dying, survivor);
            collapsed = true;
        }
        if !collapsed {
            break;
        }
    }

    // Where a region came down to one vertex, that vertex takes the position
    // the region's own surface is best fitted by. A region that did not is
    // counted: the output keeps its extra vertices, which is a slightly denser
    // mesh rather than a broken one.
    let living: Vec<u32> = members
        .iter()
        .copied()
        .filter(|&vertex| state.alive[vertex as usize])
        .collect();
    if living.len() == 1 {
        place_survivor(
            surface, features, partition, quadrics, state, incidence, region, living[0],
        );
    } else if living.len() > 1 {
        state.stubborn += 1;
    }
}

/// The edges inside a region as the mesh stands *now*, shortest first.
///
/// Read from the live faces rather than from the input's index, because a
/// collapse rewrites what is adjacent to what. Ties break on the vertex numbers
/// so two runs agree.
fn live_edges(
    state: &Collapsed,
    incidence: &[Vec<u32>],
    partition: &Partition,
    members: &[u32],
    region: u32,
    out: &mut Vec<(f64, u32, u32)>,
) {
    out.clear();
    for &vertex in members {
        if !state.alive[vertex as usize] {
            continue;
        }
        for &face in &incidence[vertex as usize] {
            if state.is_dead(face) {
                continue;
            }
            for other in state.face(face) {
                if other > vertex
                    && state.alive[other as usize]
                    && partition.label[other as usize] == region
                {
                    out.push((distance_between(state, vertex, other), vertex, other));
                }
            }
        }
    }
    out.sort_unstable_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((a.1, a.2).cmp(&(b.1, b.2)))
    });
    out.dedup_by(|a, b| (a.1, a.2) == (b.1, b.2));
}

fn distance_between(state: &Collapsed, a: u32, b: u32) -> f64 {
    let (a, b) = (state.positions[a as usize], state.positions[b as usize]);
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (x * x + y * y + z * z).sqrt()
}

/// Which of the two endpoints dies, or `None` when the edge may not collapse at
/// all.
fn survivor_of(
    surface: Surface<'_>,
    features: &Features,
    seeds: &Seeds,
    a: u32,
    b: u32,
    faces_along: usize,
) -> Option<(u32, u32)> {
    let feature_a = features.on_feature[a as usize];
    let feature_b = features.on_feature[b as usize];

    // Two vertices of an outline may only merge where the outline actually runs
    // between them - across an edge with one face. Two points a long way apart
    // on the *same* loop share a chain and pass every other test here, and
    // collapsing them pinches the loop into a figure of eight: the vertex left
    // behind carries four border edges instead of two. Measured on a stylized
    // plant's bark, which is an open sheet curled into a tube so its outline
    // passes close to itself.
    if surface.topology.boundary[a as usize]
        && surface.topology.boundary[b as usize]
        && faces_along != 1
    {
        return None;
    }

    if feature_a && feature_b {
        // Both on a feature: legal along one chain, never between two. Merging
        // separate chains is what would round off a corner or join two holes.
        //
        // Decided from which chain each vertex belongs to, and deliberately not
        // by looking the edge up in the input's topology: a collapse *creates*
        // adjacencies, so by the time two chain vertices become neighbours the
        // edge between them need never have existed in the input. Asking the
        // input refused those collapses and left the result a fifth denser than
        // it was asked for (measured on a nine-stone pedestal: 3586 faces for a
        // budget of 3000).
        let chain = features.polyline_of[a as usize];
        if chain == u32::MAX || chain != features.polyline_of[b as usize] {
            return None;
        }
        // A junction outranks a plain feature vertex; it is a corner.
        return match (features.junction[a as usize], features.junction[b as usize]) {
            (true, true) => None,
            (true, false) => Some((b, a)),
            (false, true) => Some((a, b)),
            (false, false) => Some((b, a)),
        };
    }
    // A feature never collapses into an ordinary vertex, or the outline moves.
    if feature_a {
        return Some((b, a));
    }
    if feature_b {
        return Some((a, b));
    }
    // Otherwise the seed is the natural survivor, so the region's last vertex
    // is the one it grew from.
    let seed = seeds.vertices.binary_search(&a).is_ok();
    if seed { Some((b, a)) } else { Some((a, b)) }
}

/// How many live faces use the edge between `a` and `b`. One is an outline, two
/// is ordinary surface, and anything else is a branch.
fn faces_along_edge(state: &Collapsed, incidence: &[Vec<u32>], a: u32, b: u32) -> usize {
    incidence[a as usize]
        .iter()
        .filter(|&&face| {
            !state.is_dead(face) && {
                let corners = state.face(face);
                corners.contains(&a) && corners.contains(&b)
            }
        })
        .count()
}

/// Whether collapsing `dying` into `survivor` keeps the mesh a surface.
///
/// The link condition: the two endpoints' common neighbours must be exactly the
/// third corners of the faces along the edge. A common neighbour that is *not*
/// one of those means the collapse would fold two separate parts of the surface
/// onto each other.
fn link_condition(
    state: &Collapsed,
    incidence: &[Vec<u32>],
    dying: u32,
    survivor: u32,
    left: &mut Vec<u32>,
    right: &mut Vec<u32>,
) -> bool {
    state.neighbours(dying, &incidence[dying as usize], left);
    state.neighbours(survivor, &incidence[survivor as usize], right);

    let mut shared: Vec<u32> = left
        .iter()
        .copied()
        .filter(|vertex| right.binary_search(vertex).is_ok())
        .collect();
    shared.sort_unstable();
    shared.dedup();

    let mut thirds: Vec<u32> = Vec::new();
    for &face in &incidence[dying as usize] {
        if state.is_dead(face) {
            continue;
        }
        let corners = state.face(face);
        if corners.contains(&survivor) {
            for corner in corners {
                if corner != dying && corner != survivor {
                    thirds.push(corner);
                }
            }
        }
    }
    thirds.sort_unstable();
    thirds.dedup();

    shared == thirds
}

/// Where to put the survivor, or `None` when nowhere works.
///
/// Tried in order of how much error each costs, and the first that turns no
/// face inside out wins. Letting the survivor *move* is what makes a region
/// collapse at all: pinned to its own position, the last merges in a region
/// drag a vertex across the whole of it and flip every sliver on the way.
/// Measured on a nine-stone pedestal, that refused 1489 collapses against 102
/// for every other reason put together, and left the result a fifth denser than
/// it was asked for.
///
/// A feature vertex is the exception and does not move: it is on an outline,
/// and the outline is where it is.
fn placement(
    state: &Collapsed,
    incidence: &[Vec<u32>],
    quadrics: &[Quadric],
    features: &Features,
    dying: u32,
    survivor: u32,
) -> Option<[f64; 3]> {
    let here = state.positions[survivor as usize];
    if features.on_feature[survivor as usize] {
        return (!flips_a_normal(state, incidence, dying, survivor, here)).then_some(here);
    }

    let there = state.positions[dying as usize];
    let mut sum = quadrics[dying as usize];
    sum.add(&quadrics[survivor as usize]);

    let mut candidates = vec![
        here,
        [
            (here[0] + there[0]) * 0.5,
            (here[1] + there[1]) * 0.5,
            (here[2] + there[2]) * 0.5,
        ],
        there,
    ];
    // The quadric's own answer, when it has one and it is not somewhere absurd.
    if let Some(best) = sum.optimal_point() {
        let (x, y, z) = (best[0] - here[0], best[1] - here[1], best[2] - here[2]);
        let reach = (there[0] - here[0]).powi(2)
            + (there[1] - here[1]).powi(2)
            + (there[2] - here[2]).powi(2);
        if x * x + y * y + z * z <= reach * OPTIMUM_REACH * OPTIMUM_REACH {
            candidates.push(best);
        }
    }

    // Cheapest first, ties on the order above so two runs agree.
    let mut ranked: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(at, point)| (at, sum.error_at(*point)))
        .collect();
    ranked.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked
        .into_iter()
        .map(|(at, _)| candidates[at])
        .find(|&point| !flips_a_normal(state, incidence, dying, survivor, point))
}

/// Whether any face that survives the collapse would turn inside out, with the
/// survivor at `at`.
///
/// Both vertices' faces are checked, not just the dying one's: the survivor is
/// moving too, so the faces around it are just as able to fold over.
fn flips_a_normal(
    state: &Collapsed,
    incidence: &[Vec<u32>],
    dying: u32,
    survivor: u32,
    at: [f64; 3],
) -> bool {
    let faces = incidence[dying as usize]
        .iter()
        .chain(&incidence[survivor as usize])
        .copied();
    would_flip(state, faces, dying, survivor, at)
}

/// Whether moving `vertex` — on its own, with nothing collapsing — to `at`
/// would turn any of its faces inside out.
///
/// [`flips_a_normal`] with no dying vertex, which is what a region's final
/// placement is: the merges are all done and one vertex is being put where its
/// region's surface is best fitted. Without this the placement was the one
/// move in the whole rebuild that was never checked, and it is the move most
/// able to fold a fan, because it happens after every collapse has already
/// pulled the neighbourhood in around it.
fn move_flips_a_normal(
    state: &Collapsed,
    incidence: &[Vec<u32>],
    vertex: u32,
    at: [f64; 3],
) -> bool {
    would_flip(
        state,
        incidence[vertex as usize].iter().copied(),
        NO_VERTEX,
        vertex,
        at,
    )
}

/// The body of both: every live face in `faces`, with `dying` renamed to
/// `survivor` and the survivor at `at`.
fn would_flip(
    state: &Collapsed,
    faces: impl Iterator<Item = u32>,
    dying: u32,
    survivor: u32,
    at: [f64; 3],
) -> bool {
    for face in faces {
        if state.is_dead(face) {
            continue;
        }
        let corners = state.face(face);
        // A face along the edge disappears, so it cannot flip.
        if corners.contains(&dying) && corners.contains(&survivor) {
            continue;
        }
        let before = normal_of(state, corners);
        let after = normal_with(
            state,
            corners.map(|corner| if corner == dying { survivor } else { corner }),
            survivor,
            at,
        );
        match (before, after) {
            // The face had area and now has none, or has turned over.
            (Some(_), None) => return true,
            (Some(before), Some(after)) => {
                let dot = before[0] * after[0] + before[1] * after[1] + before[2] * after[2];
                if dot <= 0.0 {
                    return true;
                }
            }
            // It had no area to begin with; moving it cannot make things worse.
            (None, _) => {}
        }
    }
    false
}

fn normal_of(state: &Collapsed, corners: [u32; 3]) -> Option<[f64; 3]> {
    normal_with(state, corners, u32::MAX, [0.0; 3])
}

/// [`normal_of`], with `moved` taken to be at `at` instead of where it is.
fn normal_with(state: &Collapsed, corners: [u32; 3], moved: u32, at: [f64; 3]) -> Option<[f64; 3]> {
    let position = |corner: u32| {
        if corner == moved {
            at
        } else {
            state.positions[corner as usize]
        }
    };
    let (a, b, c) = (
        position(corners[0]),
        position(corners[1]),
        position(corners[2]),
    );
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    (length > 0.0).then(|| [cross[0] / length, cross[1] / length, cross[2] / length])
}

/// Perform the collapse: the faces along the edge die, the rest rename their
/// corner, and the survivor takes over the dying vertex's faces.
fn apply(state: &mut Collapsed, incidence: &mut [Vec<u32>], dying: u32, survivor: u32) {
    let faces = std::mem::take(&mut incidence[dying as usize]);
    for &face in &faces {
        if state.is_dead(face) {
            continue;
        }
        let corners = state.face(face);
        if corners.contains(&survivor) {
            state.kill(face);
            continue;
        }
        let base = face as usize * 3;
        for slot in 0..3 {
            if state.corners[base + slot] == dying {
                state.corners[base + slot] = survivor;
            }
        }
        incidence[survivor as usize].push(face);
    }
    state.alive[dying as usize] = false;
}

/// Move a region's last vertex to where its own surface is best fitted.
///
/// Two things bound the move, and they are not the same thing. The **reach**
/// limit says how far: a quadric on a nearly flat patch is barely determined
/// and can solve to a point off in space. The **flip** guard says which way: a
/// move well inside the reach limit can still fold the fan back over its
/// neighbours, because the collapses have just pulled that fan in tight and the
/// optimum is under no obligation to sit inside it.
///
/// Without the second, this was the only move in the rebuild that nothing
/// checked, and it was the one most able to do damage. Measured on a nine-stone
/// pedestal it put 3.3 % of the triangles through each other at ratio 0.25 and
/// 5.4 % at 0.07, which reads on screen as a torn silhouette — and it inflated
/// the surface area past 100 %, so the area measure the fidelity suite leans on
/// scored the fold as better than a perfect rebuild.
#[allow(
    clippy::too_many_arguments,
    reason = "one region's last vertex against the whole state"
)]
fn place_survivor(
    surface: Surface<'_>,
    features: &Features,
    partition: &Partition,
    quadrics: &[Quadric],
    state: &mut Collapsed,
    incidence: &[Vec<u32>],
    region: u32,
    survivor: u32,
) {
    // A feature vertex stays exactly where it is: the whole reason it survived
    // is that the outline runs through it.
    if features.on_feature[survivor as usize] {
        return;
    }
    let members = partition.members_of(region);
    let mut sum = Quadric::default();
    for &vertex in members {
        sum.add(&quadrics[vertex as usize]);
    }
    let Some(point) = sum.optimal_point() else {
        return;
    };

    // How far the region reaches, so an answer from a nearly flat patch — where
    // the solve is barely determined — cannot fling the vertex off the surface.
    let here = state.positions[survivor as usize];
    let mut reach = 0.0f64;
    for &vertex in members {
        reach = reach.max(surface.distance(survivor, vertex));
    }
    let reach = if reach > 0.0 {
        reach
    } else {
        surface.sizes[survivor as usize] as f64
    };
    let (x, y, z) = (point[0] - here[0], point[1] - here[1], point[2] - here[2]);
    if (x * x + y * y + z * z).sqrt() > reach * OPTIMUM_REACH {
        return;
    }
    // And the same guard every other placement in this module passes. The reach
    // limit above bounds how far the vertex goes; it says nothing about which
    // *way*, and a fan that the collapses have just pulled tight around this
    // vertex folds over at a fraction of the region's own extent.
    //
    // Refused outright the move is simply lost, and with it the area the
    // optimum was keeping. So the same move is offered shorter first: the
    // direction is the quadric's answer either way, and part of the way there
    // is better than none. Stepping down by halves rather than searching for
    // the exact limit, because the last fraction of the move is worth less than
    // the error term suggests and a bisection would cost a fan walk per step.
    for &fraction in &OPTIMUM_BACKOFF {
        let at = [
            here[0] + x * fraction,
            here[1] + y * fraction,
            here[2] + z * fraction,
        ];
        if !move_flips_a_normal(state, incidence, survivor, at) {
            state.positions[survivor as usize] = at;
            return;
        }
    }
}

/// The collapsed mesh as a polygon soup the rest of the operation understands.
///
/// Vertices are renumbered in input order, so the output's numbering is a
/// function of the mesh rather than of the order the regions happened to be
/// collapsed in.
///
/// Two things come back with it, and both are things only this stage knows.
///
/// [`Carried::pinned`] says a vertex sat on a border, a crease or a fold in the
/// source: it is *where the shape is*, and the tidying that follows must leave
/// it alone. The output's own topology cannot answer that — a rim is an
/// ordinary interior edge once the shell is closed.
///
/// [`Carried::source`] says which input vertex each output vertex *is*. Every
/// output vertex is one — a collapse merges vertices, it never invents one —
/// and this is the last moment that is known, because the renumbering below is
/// what throws the input's numbering away. The [quad merge](super::quads) reads
/// it to find the direction field, which is over the input.
pub(crate) fn into_output(state: &Collapsed, features: &Features) -> Carried {
    let mut slot_of = vec![u32::MAX; state.alive.len()];
    let mut pinned: Vec<bool> = Vec::new();
    let mut source: Vec<u32> = Vec::new();
    let mut output = RemeshOutput {
        positions: Vec::new(),
        face_offsets: vec![0],
        corners: Vec::new(),
    };
    for face in 0..(state.corners.len() / 3) as u32 {
        if state.is_dead(face) {
            continue;
        }
        let corners = state.face(face);
        // A face that lost a corner to a collapse it was not part of, or that
        // has come to name one vertex twice, is not a triangle any more.
        if corners.iter().any(|&corner| !state.alive[corner as usize])
            || corners[0] == corners[1]
            || corners[1] == corners[2]
            || corners[0] == corners[2]
        {
            continue;
        }
        for corner in corners {
            if slot_of[corner as usize] == u32::MAX {
                slot_of[corner as usize] = output.positions.len() as u32;
                let at = state.positions[corner as usize];
                output
                    .positions
                    .push(Vec3::new(at[0] as f32, at[1] as f32, at[2] as f32));
                pinned.push(
                    features
                        .on_feature
                        .get(corner as usize)
                        .copied()
                        .unwrap_or(false),
                );
                source.push(corner);
            }
            output.corners.push(slot_of[corner as usize]);
        }
        output
            .face_offsets
            .push(output.corners.len().try_into().unwrap_or(u32::MAX));
    }
    Carried {
        output,
        pinned,
        source,
    }
}

/// A collapsed mesh, and what only the collapse knows about it. See
/// [`into_output`].
pub(crate) struct Carried {
    pub(crate) output: RemeshOutput,
    /// Per output vertex: it sat on a feature of the source.
    pub(crate) pinned: Vec<bool>,
    /// Per output vertex: the input vertex it is.
    pub(crate) source: Vec<u32>,
}

/// One quadric per vertex, from the faces around it plus a plane along every
/// border edge.
pub(crate) fn build_quadrics(surface: Surface<'_>, threads: usize) -> Vec<Quadric> {
    let _z = crate::prof::zone!("Remesh Quadrics");
    let mut quadrics = vec![Quadric::default(); surface.vertex_count()];
    crate::parallel::sweep(&mut quadrics, threads, |base, chunk| {
        let mut edges = Vec::new();
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let mut sum = Quadric::default();
            for &face in surface.topology.faces_of(vertex) {
                let Some(normal) = surface.face_normal(face) else {
                    continue;
                };
                let corners = surface.face(face);
                sum.add(&Quadric::from_plane(
                    normal,
                    surface.position(corners[0]),
                    surface.face_area(face),
                ));
            }

            // A border edge has surface on one side only, so nothing above stops
            // the vertex sliding along it. The plane through the edge and
            // perpendicular to its face is what does.
            surface
                .topology
                .edges_at(surface.indices, vertex, &mut edges);
            for edge in &edges {
                if !edge.is_boundary() {
                    continue;
                }
                let Some(face_normal) = surface.face_normal(edge.faces[0]) else {
                    continue;
                };
                let here = surface.position(vertex);
                let there = surface.position(edge.other);
                let along = [there[0] - here[0], there[1] - here[1], there[2] - here[2]];
                let length =
                    (along[0] * along[0] + along[1] * along[1] + along[2] * along[2]).sqrt();
                if length <= 0.0 {
                    continue;
                }
                let normal = [
                    along[1] * face_normal[2] - along[2] * face_normal[1],
                    along[2] * face_normal[0] - along[0] * face_normal[2],
                    along[0] * face_normal[1] - along[1] * face_normal[0],
                ];
                let scale =
                    (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                if scale <= 0.0 {
                    continue;
                }
                sum.add(&Quadric::from_plane(
                    [normal[0] / scale, normal[1] / scale, normal[2] / scale],
                    here,
                    length * length,
                ));
            }
            *slot = sum;
        }
    });
    quadrics
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::remesh::topology::Topology;
    use crate::remesh::{features, partition, seeds, size_field};

    /// A flat strip of `steps` quads: an open surface one quad wide, which is
    /// what a leaf or a sheet of cloth is and what the old field extraction
    /// shredded.
    fn strip(steps: u32) -> (Vec<f32>, Vec<u32>) {
        let mut positions = Vec::new();
        for step in 0..=steps {
            let x = step as f32 * 0.1;
            positions.extend_from_slice(&[x, 0.0, 0.0]);
            positions.extend_from_slice(&[x, 0.1, 0.0]);
        }
        let mut indices = Vec::new();
        for step in 0..steps {
            let at = step * 2;
            indices.extend_from_slice(&[at, at + 2, at + 1]);
            indices.extend_from_slice(&[at + 1, at + 2, at + 3]);
        }
        (positions, indices)
    }

    fn grid(side: u32) -> (Vec<f32>, Vec<u32>) {
        let step = 1.0 / (side - 1) as f32;
        let mut positions = Vec::new();
        for row in 0..side {
            for column in 0..side {
                positions.extend_from_slice(&[column as f32 * step, row as f32 * step, 0.0]);
            }
        }
        let mut indices = Vec::new();
        for row in 0..side - 1 {
            for column in 0..side - 1 {
                let at = row * side + column;
                indices.extend_from_slice(&[at, at + 1, at + side]);
                indices.extend_from_slice(&[at + 1, at + side + 1, at + side]);
            }
        }
        (positions, indices)
    }

    /// A torus, so "the shape survived" means something stronger than "it is
    /// still a disc".
    fn torus(around: u32, through: u32) -> (Vec<f32>, Vec<u32>) {
        let (big, small) = (1.0f32, 0.35f32);
        let mut positions = Vec::new();
        for major in 0..around {
            let a = major as f32 / around as f32 * std::f32::consts::TAU;
            for minor in 0..through {
                let b = minor as f32 / through as f32 * std::f32::consts::TAU;
                let radius = big + small * b.cos();
                positions.extend_from_slice(&[radius * a.cos(), radius * a.sin(), small * b.sin()]);
            }
        }
        let mut indices = Vec::new();
        for major in 0..around {
            for minor in 0..through {
                let next_major = (major + 1) % around;
                let next_minor = (minor + 1) % through;
                let a = major * through + minor;
                let b = major * through + next_minor;
                let c = next_major * through + minor;
                let d = next_major * through + next_minor;
                indices.extend_from_slice(&[a, c, b]);
                indices.extend_from_slice(&[b, c, d]);
            }
        }
        (positions, indices)
    }

    struct Run {
        output: RemeshOutput,
        stubborn: usize,
    }

    fn rebuild(positions: &[f32], indices: &[u32], faces: u32, size: f32) -> Run {
        let topology = Topology::build(indices, positions.len() / 3, 1, None);
        let sizes = vec![size; topology.vertex_count];
        let areas = size_field::dual_areas(positions, indices, &topology, 1);
        let surface = Surface {
            positions,
            indices,
            topology: &topology,
            sizes: &sizes,
            areas: &areas,
        };
        let features = features::build(surface, true, None, None);
        let seeds = seeds::place(surface, &features, faces, None);
        let partition = partition::build(surface, &features, &seeds, 1, None);
        let quadrics = build_quadrics(surface, 1);
        let state = run(surface, &features, &seeds, &partition, &quadrics, None);
        Run {
            stubborn: state.stubborn,
            output: into_output(&state, &features).output,
        }
    }

    /// Every undirected edge of the result, with how many faces use it.
    fn edge_uses(output: &RemeshOutput) -> HashMap<(u32, u32), u32> {
        let mut uses = HashMap::new();
        for face in 0..output.face_count() {
            let corners = output.face(face);
            for corner in 0..corners.len() {
                let (a, b) = (corners[corner], corners[(corner + 1) % corners.len()]);
                let key = if a < b { (a, b) } else { (b, a) };
                *uses.entry(key).or_insert(0u32) += 1;
            }
        }
        uses
    }

    /// The headline property: a collapse cannot invent a third face on an edge,
    /// whatever it is handed.
    #[test]
    fn the_result_is_always_a_surface() {
        for (label, (positions, indices), faces) in [
            ("strip", strip(40), 30),
            ("grid", grid(20), 150),
            ("torus", torus(24, 12), 200),
        ] {
            let run = rebuild(&positions, &indices, faces, 0.15);

            assert!(run.output.validate().is_ok(), "{label}: malformed soup");
            for (edge, uses) in edge_uses(&run.output) {
                assert!(
                    uses <= 2,
                    "{label}: edge {edge:?} is used by {uses} faces, so the output branches"
                );
            }
        }
    }

    /// The failure that prompted the rewrite. An open strip one quad wide has
    /// to come back closed along its border, with no holes punched in it.
    #[test]
    fn an_open_strip_keeps_its_border_and_gains_no_holes() {
        let (positions, indices) = strip(40);

        let run = rebuild(&positions, &indices, 24, 0.25);

        let uses = edge_uses(&run.output);
        let border: Vec<(u32, u32)> = uses
            .iter()
            .filter(|&(_, &count)| count == 1)
            .map(|(&edge, _)| edge)
            .collect();
        assert!(!border.is_empty(), "a strip has a border");

        // Its border is one closed loop: every border vertex has exactly two
        // border edges. A tear or a hole shows up here as a vertex with more,
        // and a dropped border as one with fewer.
        let mut degree: HashMap<u32, u32> = HashMap::new();
        for (a, b) in border {
            *degree.entry(a).or_insert(0) += 1;
            *degree.entry(b).or_insert(0) += 1;
        }
        for (vertex, count) in degree {
            assert_eq!(
                count, 2,
                "border vertex {vertex} has {count} border edges, so the outline is torn"
            );
        }
    }

    /// A closed surface stays closed: no border edges appear from nowhere.
    #[test]
    fn a_closed_surface_comes_back_closed() {
        let (positions, indices) = torus(28, 14);

        let run = rebuild(&positions, &indices, 300, 0.2);

        let open = edge_uses(&run.output)
            .values()
            .filter(|&&uses| uses == 1)
            .count();
        assert_eq!(open, 0, "a torus has no border to acquire one");
    }

    /// Shape is preserved, not just validity: a torus is still a torus.
    #[test]
    fn a_torus_keeps_its_euler_characteristic() {
        let (positions, indices) = torus(32, 16);

        let run = rebuild(&positions, &indices, 400, 0.18);

        let vertices = run.output.positions.len() as i64;
        let edges = edge_uses(&run.output).len() as i64;
        let faces = run.output.face_count() as i64;
        assert_eq!(
            vertices - edges + faces,
            0,
            "a torus has Euler characteristic zero"
        );
    }

    /// A region that cannot fully collapse keeps its vertices rather than
    /// forcing through a collapse that would break the surface.
    #[test]
    fn a_stubborn_region_leaves_extra_vertices_rather_than_a_defect() {
        let (positions, indices) = strip(40);

        // A budget far below what a one-quad-wide strip can give up: every
        // vertex is on the border, so most regions cannot come down to one.
        let run = rebuild(&positions, &indices, 6, 0.6);

        assert!(run.output.validate().is_ok());
        for (edge, uses) in edge_uses(&run.output) {
            assert!(uses <= 2, "edge {edge:?} used {uses} times");
        }
    }

    #[test]
    fn the_collapse_is_the_same_on_every_run() {
        let (positions, indices) = grid(18);

        let first = rebuild(&positions, &indices, 120, 0.12);
        let second = rebuild(&positions, &indices, 120, 0.12);

        assert_eq!(first.output.positions, second.output.positions);
        assert_eq!(first.output.corners, second.output.corners);
        assert_eq!(first.stubborn, second.stubborn);
    }

    /// Asking for fewer faces actually produces fewer.
    #[test]
    fn the_output_follows_the_budget() {
        let (positions, indices) = grid(30);

        let coarse = rebuild(&positions, &indices, 80, 0.3);
        let fine = rebuild(&positions, &indices, 600, 0.08);

        assert!(
            coarse.output.face_count() < fine.output.face_count(),
            "80 faces asked should give fewer than 600: {} against {}",
            coarse.output.face_count(),
            fine.output.face_count()
        );
    }

    /// A hexagonal fan around one movable centre, wound the same way all over.
    fn fan() -> (Collapsed, Vec<Vec<u32>>) {
        let mut positions = vec![[0.0, 0.0, 0.0]];
        for step in 0..6 {
            let angle = step as f64 / 6.0 * std::f64::consts::TAU;
            positions.push([angle.cos(), angle.sin(), 0.0]);
        }
        let mut corners = Vec::new();
        for step in 0..6u32 {
            corners.extend_from_slice(&[0, 1 + step, 1 + (step + 1) % 6]);
        }
        let mut incidence = vec![Vec::new(); positions.len()];
        for face in 0..6u32 {
            for slot in 0..3 {
                incidence[corners[face as usize * 3 + slot] as usize].push(face);
            }
        }
        let state = Collapsed {
            corners,
            alive: vec![true; positions.len()],
            positions,
            stubborn: 0,
        };
        (state, incidence)
    }

    /// The guard [`place_survivor`] gained: a plain move, with nothing
    /// collapsing, still has to leave every face the right way round.
    #[test]
    fn moving_a_vertex_out_of_its_own_fan_reads_as_a_flip() {
        let (state, incidence) = fan();

        // Off the surface but still over the fan: every face keeps its normal.
        assert!(
            !move_flips_a_normal(&state, &incidence, 0, [0.1, 0.0, 0.4]),
            "a move within the fan is not a flip"
        );
        // Dragged out past the ring: the faces on the far side turn over.
        assert!(
            move_flips_a_normal(&state, &incidence, 0, [3.0, 0.0, 0.0]),
            "a move past the ring turns the far faces inside out"
        );
    }
}
