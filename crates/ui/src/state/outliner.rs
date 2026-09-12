//! The Outliner's own state: which tab, which view mode, and the cached tree.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind};

/// Which tab the Outliner shows: the scene's nodes or the flat deduplicated
/// material list. A cheap click switches between them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerTab {
    #[default]
    Scene,
    Materials,
    /// The model's animation clips — offered only while the loaded file carries
    /// any, and never in the Opt workspace (which shows the bind pose).
    Animations,
}

/// How the Outliner's Scene tab presents the model: every node in one flat list,
/// or the full node hierarchy as a collapsible tree. Both honor the type filter;
/// the tree additionally indents and draws parent guides. Toggled by the header
/// button, and overridden while a search is active (matches always list flat).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerViewMode {
    /// Every node, in model order, with no hierarchy.
    Flat,
    /// The full scene graph, indented and collapsible.
    #[default]
    SceneTree,
}

/// Everything the Outliner side panel owns: which tab and view mode it shows,
/// its search box and type filter, the per-model scene-tree cache it walks, and
/// its keyboard-navigation bookkeeping.
///
/// These group by lifecycle, not by widget: every one of them is scoped to the
/// panel's own session over the loaded model — most are keyed on that model's
/// node indices, and [`UiState::reset_skeletal_state`] drops them together when
/// a new model arrives. Nothing outside `ui` reads any of it, so the fields stay
/// crate-private (`app` never touches the Outliner's view state).
#[derive(Debug, Clone, Default)]
pub struct OutlinerState {
    /// Which Outliner tab is shown (scene nodes vs material list).
    pub(crate) tab: OutlinerTab,
    /// Whether the Scene tab shows every node flat or the full scene tree.
    pub(crate) view: OutlinerViewMode,
    /// The Outliner header's search box. While non-empty it overrides
    /// [`OutlinerState::view`]: both tabs collapse to a flat list of the rows
    /// whose name matches, so a hit is never buried inside a collapsed branch.
    pub(crate) search: String,
    /// Scene-tree nodes the user has *collapsed*. Stored inverted (rather than as
    /// an expanded set) so the default — an empty set — is a fully expanded tree,
    /// with no per-model initialization pass. Cleared on model load.
    pub(crate) collapsed: HashSet<usize>,
    /// Node kinds the scene tree's type filter is currently *hiding*. Empty (the
    /// default) shows everything. A hidden kind's rows still render — greyed and
    /// unselectable — when a visible node lives beneath them, so the hierarchy
    /// never breaks into disconnected fragments.
    pub(crate) hidden_kinds: HashSet<NodeKind>,
    /// Child adjacency for the scene tree (`children[parent]` lists the child
    /// node indices, in model order), built once per model rather than per frame.
    /// Empty means "not built yet"; a model load invalidates it.
    pub(crate) children: Vec<Vec<usize>>,
    /// Root node indices for the scene tree — nodes with no parent, in model
    /// order. Built alongside [`OutlinerState::children`].
    pub(crate) roots: Vec<usize>,
    /// Whether the Outliner owns the arrow keys. Set by clicking a row or by a
    /// handled arrow press, cleared by a pointer press outside the panel, so
    /// navigation survives the pointer wandering back to the viewport without the
    /// Outliner ever swallowing arrows meant for somewhere else.
    pub(crate) nav_focus: bool,
    /// One-frame request from keyboard navigation: the next Outliner draw scrolls
    /// the selected row into view. Cleared by the draw that honors it.
    pub(crate) scroll_to_selection: bool,
}

impl OutlinerState {
    /// Drop the cached scene-tree adjacency so the next Outliner frame rebuilds
    /// it. Called from [`UiState::reset_skeletal_state`], which `app` runs on
    /// model load — where the cached node indices stop meaning anything.
    pub(crate) fn invalidate_tree(&mut self) {
        self.children.clear();
        self.roots.clear();
    }

    /// Build the scene-tree adjacency if it isn't current for `model`. The scan is
    /// O(nodes) and rigs run to hundreds of nodes, so it must not happen per frame
    /// — the cache is rebuilt only after [`OutlinerState::invalidate_tree`].
    ///
    /// A node whose `parent` doesn't resolve (out of range, or itself) is treated as
    /// a root rather than dropped, so a malformed hierarchy still lists every node.
    pub(crate) fn ensure_tree(&mut self, model: &ModelData) {
        if self.children.len() == model.nodes.len() && !model.nodes.is_empty() {
            return;
        }
        self.children = vec![Vec::new(); model.nodes.len()];
        self.roots.clear();
        for (index, node) in model.nodes.iter().enumerate() {
            match node.parent {
                Some(parent) if parent < model.nodes.len() && parent != index => {
                    self.children[parent].push(index);
                }
                _ => self.roots.push(index),
            }
        }
    }
}
