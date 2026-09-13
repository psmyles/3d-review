//! Measured values held across frames: the scoped stats, the bounds caches that
//! avoid re-measuring a selection every frame, and what the device can do.

use review_model::{Bounds, MeshGroupStats, ScopeStats};
use review_render::{MsaaSamples, Selection};

use super::*;

/// The two *measured* scopes the stats overlay reports beside the file's own
/// figures: the current Outliner selection, and whatever it has left visible.
///
/// Each is summed off the mesh by [`ModelData::scope_stats`] (invariant 5),
/// never apportioned from the whole-model counts — which the card takes straight
/// from [`UiState::stats`], since those are what the source file said. Together
/// the three columns name the same slices of the scene as [`BoundsScope`], so
/// the stats card and the bounding box answer "which geometry" the same way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScopedStats {
    /// The selection's own geometry — a node's subtree, or a material slot's
    /// triangles. `None` when nothing is selected, so the column reads as empty
    /// rather than as a measured zero.
    pub(crate) selected: Option<ScopeStats>,
    /// The meshes the Outliner has left visible. Equal to `all` while nothing is
    /// hidden, which is the honest answer rather than a reason to blank it.
    pub(crate) visible: ScopeStats,
}

/// The measurements the chrome derives from the loaded model that are too
/// expensive to redo every frame, each stored beside the *input* it was computed
/// for so a change to that input — and nothing else — rebuilds it.
///
/// What binds them is one lifecycle, not one panel: every entry is a fact about
/// the currently-loaded [`ModelData`], keyed on node indices or a [`Selection`]
/// that mean nothing once a different model is on screen. [`BoundsCaches::reset`]
/// drops the whole set at once, so that invalidation is stated here instead of
/// riding on `app` happening to clear the inputs each key is compared against.
#[derive(Debug, Clone, Default)]
pub struct BoundsCaches {
    /// Cached `model.visible_bounds(hidden)` for the dimension-label overlay's
    /// "visible only" box. That scan is O(triangles); it must not run per-frame,
    /// so it's rebuilt only when [`BoundsCaches::visible_bounds_key`] (the hidden
    /// set it was computed for) no longer matches the live hidden set. Unused —
    /// the overlay falls back to [`UiState::bounds`] — when nothing is hidden or
    /// the whole-model box is shown.
    pub(crate) visible_bounds: Option<Bounds>,
    /// The sorted hidden-node set [`BoundsCaches::visible_bounds`] was built for;
    /// a mismatch with the live hidden set invalidates the cache.
    pub(crate) visible_bounds_key: Vec<u32>,
    /// Cached `selection_bounds(selection)` for the dimension-label overlay's
    /// "only selection" box (same O(triangles) caching as the visible-only box,
    /// keyed by the selection it was computed for).
    pub(crate) selection_bounds: Option<Bounds>,
    /// The selection [`BoundsCaches::selection_bounds`] was built for — the
    /// primary *and* the whole selected set, since adding a part to a
    /// multi-selection grows the box without moving the primary. A mismatch with
    /// the live selection invalidates the cache.
    pub(crate) selection_bounds_key: (Selection, Vec<u32>),
    /// How many logical source vertices the current bone selection influences,
    /// shown by the Inspector. The scan is O(influences) — 168k on a game
    /// character — so it must not run per frame; it is recomputed only when
    /// [`BoundsCaches::bone_influence_key`] no longer matches the live selection.
    pub(crate) bone_influence: usize,
    /// The sorted bone set [`BoundsCaches::bone_influence`] was measured for; a
    /// mismatch with the live selection invalidates it.
    pub(crate) bone_influence_key: Vec<u32>,
    /// The model's per-(node, material) measured table — the input every column
    /// of the stats overlay sums. Building it is an O(corners) hash walk, so it
    /// happens once, the first frame the overlay asks for it.
    pub(crate) mesh_groups: Option<Vec<MeshGroupStats>>,
    /// The stats card's three scoped columns, and the (selection, hidden set)
    /// they were summed for. `None` means "not measured yet" — which
    /// [`Selection::None`] plus an empty hidden set cannot express on its own,
    /// since that is also a perfectly ordinary live state.
    pub(crate) scoped_stats: Option<ScopedStats>,
    pub(crate) scoped_stats_key: (Selection, Vec<u32>, Vec<u32>),
    /// The Opt workspace's *processed* level measured under the active bounding-box
    /// scope, for the split's right-hand dimension labels. Its own slot rather than
    /// a share of the two above: both meshes are measured in the same frame, so one
    /// slot would thrash between them every frame.
    pub(crate) processed_bounds: Option<Bounds>,
    /// What [`BoundsCaches::processed_bounds`] was measured for. The revision leads
    /// because a reprocess replaces the mesh outright; the scope and its inputs
    /// follow, exactly as for the source's two scoped caches. `None` until measured
    /// — which no key value can express on its own, since a level legitimately
    /// measures to `None` when the scope selects no geometry.
    pub(crate) processed_bounds_key: Option<ProcessedBoundsKey>,
}

/// What a processed level's bounds were measured for: the level's revision, the
/// scope, and the two inputs a scope can read — the hidden set and the selected
/// node set.
pub(crate) type ProcessedBoundsKey = (u64, BoundsScope, Selection, Vec<u32>, Vec<u32>);

impl BoundsCaches {
    /// Drop every cached measurement, so the next frame that needs one rebuilds
    /// it against the model now on screen.
    ///
    /// `app` calls this on model load. Each key is a node-index list or a
    /// [`Selection`] naming the *outgoing* model, so an incoming model that
    /// happens to reproduce one — the same node hidden again, the same node
    /// selected again — would otherwise be served the previous model's box.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What the running build and the active adapter can do — the facts `app`
/// establishes once during startup and never revises.
///
/// The lifecycle is what groups them: each is written at most once (in `App`'s
/// construction or its device bootstrap) and is read-only for the rest of the
/// session, so nothing in `ui` ever needs a `&mut` to one. That is why the
/// capability seams live here rather than beside the settings they gate — the
/// settings change every frame, these never do.
#[derive(Debug, Clone)]
pub struct Capabilities {
    /// MSAA levels the active adapter actually supports, set by `app` from the
    /// device's `supported_msaa_counts()`, which the backend leaf answers.
    /// The Anti Aliasing menu disables any level not in this list (invariant 4).
    /// Empty until the adapter is known (the panel then falls back to offering only
    /// the current level).
    pub msaa_levels: Vec<MsaaSamples>,
    /// Whether the active adapter can build the IBL maps. Always true on the desktop
    /// target (the device hard-requires `TEXTURE_COMPRESSION_BC` for the BC6H IBL
    /// cubes, and 11_0+ guarantees the float formats), so the Environment panel never
    /// disables the IBL toggle in practice; kept as a field for the capability seam.
    pub(crate) ibl: bool,
    /// Whether the active adapter can run GTAO. Always true on the desktop target
    /// (the G-buffer + horizon passes need only float render targets + samplers
    /// guaranteed at feature level 11_0+), so the status-bar AO button is never
    /// disabled in practice; kept as a field for the capability seam.
    pub(crate) gtao: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            msaa_levels: Vec::new(),
            // Assume supported until the adapter is queried; `app` corrects these
            // once the device is known.
            ibl: true,
            gtao: true,
        }
    }
}
