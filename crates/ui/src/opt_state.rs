//! UI state for the Opt workspace: the operation stack the user edits, the
//! comparison-view settings, and the last processed result's measured figures.
//!
//! Ownership follows the existing convention for document state the UI edits
//! directly (selection, solo, the hidden-mesh set): `UiState` holds it, the
//! chrome mutates it in place, and `app` reads it back each frame — bumping
//! [`OptUiState::stack_revision`] is what tells `app` to reprocess and what the
//! undo snapshot compares. Only the actions `app` must *perform* — the file
//! dialogs behind export and presets — travel as [`OptIntent`]s.

use std::sync::Arc;

use review_model::ModelStats;
use review_optimize::{AnalysisMetrics, MeshCounts, OptStack};

/// How the Opt viewport compares the two meshes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OptLayout {
    /// Side by side: source left, processed right.
    #[default]
    Split,
    /// One view, both meshes in the same space — the processed mesh shaded and
    /// the source drawn as a ghost over it.
    Overlay,
}

/// How the source mesh is drawn in [`OptLayout::Overlay`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GhostStyle {
    /// A translucent tinted surface: reads the silhouette difference at a glance.
    #[default]
    Xray,
    /// Only the source's edges: keeps the processed surface fully visible.
    Wireframe,
}

impl GhostStyle {
    pub const ALL: [GhostStyle; 2] = [GhostStyle::Xray, GhostStyle::Wireframe];

    pub fn label(self) -> review_localization::Key {
        match self {
            GhostStyle::Xray => crate::keys::ui_enums::GHOST_XRAY,
            GhostStyle::Wireframe => crate::keys::ui_enums::GHOST_WIREFRAME,
        }
    }
}

impl From<GhostStyle> for review_render::GhostStyle {
    fn from(value: GhostStyle) -> Self {
        match value {
            GhostStyle::Xray => review_render::GhostStyle::Xray,
            GhostStyle::Wireframe => review_render::GhostStyle::Wireframe,
        }
    }
}

/// Which mesh the single-view comparison currently shows solid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComparisonSide {
    Source,
    #[default]
    Processed,
}

impl ComparisonSide {
    pub fn swapped(self) -> Self {
        match self {
            ComparisonSide::Source => ComparisonSide::Processed,
            ComparisonSide::Processed => ComparisonSide::Source,
        }
    }

    pub fn label(self) -> review_localization::Key {
        match self {
            ComparisonSide::Source => crate::keys::ui_enums::SIDE_SOURCE,
            ComparisonSide::Processed => crate::keys::ui_enums::SIDE_PROCESSED,
        }
    }
}

/// What the stack panel currently has selected, and therefore what the Inspector
/// shows parameters for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackItem {
    /// An operation, addressed by its stable [`review_optimize::OpInstance::id`].
    Op(u64),
    /// The pinned "Export settings" row at the bottom of the stack.
    ExportSettings,
}

/// A snapshot of one processed level's measured figures, pushed by `app` after a
/// run completes. Plain values only — the UI never holds mesh data (invariant 2).
#[derive(Debug, Clone, PartialEq)]
pub struct OptLevelView {
    pub stats: ModelStats,
    pub metrics: AnalysisMetrics,
}

/// What the UI knows about the last completed processing run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptResultView {
    /// One entry per output level, level 0 first.
    pub levels: Vec<OptLevelView>,
    /// The source mesh's own buffer counts, measured by the same run. The
    /// processed card's deltas are against these rather than against the Model
    /// Stats panel's figures, which are the file's DCC counts — import splits
    /// every face corner, so those two never described the same mesh and their
    /// difference read as the optimizer inventing vertices.
    ///
    /// Measured *after* the run's lossless index pass, so it equals the stats
    /// panel's "GPU Verts" — the two figures the UI presents as the same thing
    /// are computed to be the same thing.
    pub source: MeshCounts,
    /// The source mesh's own cache / overdraw / fetch figures. Shown on their own
    /// before anything is enabled — a run with an empty stack measures the mesh
    /// and produces no levels — and as the baseline each level's change is quoted
    /// against once there are.
    pub source_metrics: AnalysisMetrics,
    /// Wall-clock milliseconds the run took.
    pub elapsed_ms: f32,
    /// Non-fatal problems the run reported, already de-duplicated.
    pub warnings: Vec<String>,
}

/// Everything the Opt workspace keeps in the UI.
#[derive(Debug, Clone)]
pub struct OptUiState {
    /// The operation stack. Behind an `Arc` so `app` can hand it to a worker
    /// thread and the undo snapshot can hold it without cloning the value.
    pub stack: Arc<OptStack>,
    /// Bumped on every mutation of `stack`. `app` watches this to schedule a
    /// reprocess, and the undo snapshot compares it instead of the stack itself.
    pub stack_revision: u64,
    /// The stack row whose parameters the Inspector shows.
    pub selected: Option<StackItem>,
    /// Which LOD level the viewport shows. Clamped by `app` against the actual
    /// result, so a stale selection from a longer chain can't point past the end.
    pub active_lod: usize,
    /// Whether the user has picked a level themselves.
    ///
    /// Until they do, a run that produces a chain shows its *first simplified*
    /// level. Level 0 is never simplified, so leaving the viewport there after
    /// adding a LOD operation shows a mesh identical to the source and reads as
    /// the operation having done nothing at all.
    pub lod_pinned: bool,
    pub layout: OptLayout,
    /// Whether the two views in [`OptLayout::Split`] share one camera.
    pub camera_sync: bool,
    pub ghost_style: GhostStyle,
    /// Which mesh reads as the "solid" one — the A/B swap.
    pub side: ComparisonSide,
    /// The last completed run, or `None` before the first one finishes.
    pub result: Option<OptResultView>,
    /// A run is in flight. Drives the toolbar's busy affordance; the viewport
    /// keeps showing the previous result rather than blanking.
    pub processing: bool,
}

impl Default for OptUiState {
    fn default() -> Self {
        Self {
            stack: Arc::new(OptStack::default()),
            stack_revision: 0,
            selected: None,
            active_lod: 0,
            lod_pinned: false,
            layout: OptLayout::default(),
            camera_sync: true,
            ghost_style: GhostStyle::default(),
            side: ComparisonSide::default(),
            result: None,
            processing: false,
        }
    }
}

impl OptUiState {
    /// Mutate the stack through `edit` and bump the revision.
    ///
    /// Every stack edit goes through here so no caller can forget the bump — a
    /// missed one leaves the viewport showing a result for a stack that no longer
    /// exists, with nothing to indicate why.
    pub fn edit_stack(&mut self, edit: impl FnOnce(&mut OptStack)) {
        self.edit_stack_with(edit);
    }

    /// [`Self::edit_stack`] for an edit that returns something — the id of a
    /// newly pushed operation, say, so the caller can select the row it created.
    pub fn edit_stack_with<T>(&mut self, edit: impl FnOnce(&mut OptStack) -> T) -> T {
        let value = edit(Arc::make_mut(&mut self.stack));
        self.stack_revision = self.stack_revision.saturating_add(1);
        value
    }

    /// Replace the stack wholesale (a loaded preset, or an undo/redo restore).
    pub fn set_stack(&mut self, stack: Arc<OptStack>) {
        self.stack = stack;
        self.stack_revision = self.stack_revision.saturating_add(1);
        // The replacement stack's operation ids are not the old ones, so a
        // selection into it would address nothing (or, worse, the wrong row).
        self.selected = None;
    }

    /// The measured figures for the level the viewport is showing.
    pub fn active_level(&self) -> Option<&OptLevelView> {
        self.result
            .as_ref()
            .and_then(|result| result.levels.get(self.active_lod))
    }

    /// How many levels the last run produced. `0` before the first result.
    pub fn level_count(&self) -> usize {
        self.result.as_ref().map_or(0, |result| result.levels.len())
    }

    /// Whether there is a processed mesh to compare against. Until the first run
    /// completes the workspace shows the source mesh alone, so the comparison
    /// controls have nothing to act on.
    pub fn has_result(&self) -> bool {
        self.level_count() > 0
    }
}

/// An Opt action `app` must carry out. Only the operations that reach outside
/// the app — file dialogs and disk writes — are intents; everything else the UI
/// owns is a plain [`OptUiState`] field that `app` reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptIntent {
    /// Write the current LOD chain to disk using the stack's export settings.
    Export,
    /// Save the stack as a JSON preset.
    SavePreset,
    /// Load a stack from a JSON preset, replacing the current one.
    LoadPreset,
}
