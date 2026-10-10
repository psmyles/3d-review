//! The Outliner's own state: which tab, which view mode, and the cached tree.

use std::collections::HashSet;

use review_model::{ModelData, NodeKind};

use super::WorkspaceMode;

/// Which tab the Outliner shows. Each workspace offers its own subset (see
/// [`OutlinerTab::available`]), and a cheap click switches between them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerTab {
    /// The scene's nodes.
    #[default]
    Scene,
    /// The flat, deduplicated material list.
    Materials,
    /// The scene texture pool — what the Tex workspace views, and what the
    /// materials sample.
    Textures,
    /// The model's animation clips — offered only while the loaded file carries
    /// any, and only in the 3D workspace (UV and Tex have no pose, and Opt
    /// deliberately shows the bind pose).
    Animations,
    /// The audit's findings — the Aud workspace's own list.
    Issues,
}

impl OutlinerTab {
    /// Every tab `mode` can offer, in strip order, before the Animations tab's
    /// "only when the file has clips" rule. The one table that says which
    /// workspace shows what: UV lays out nodes and samples textures but has no
    /// use for a material list; Tex is an image viewer, so it lists only images;
    /// Opt edits geometry and leaves textures alone.
    fn for_mode(mode: WorkspaceMode) -> &'static [OutlinerTab] {
        use OutlinerTab::{Animations, Issues, Materials, Scene, Textures};
        match mode {
            WorkspaceMode::ThreeD => &[Scene, Materials, Textures, Animations],
            WorkspaceMode::Uv => &[Scene, Textures],
            WorkspaceMode::Texture => &[Textures],
            WorkspaceMode::Opt => &[Scene, Materials],
            WorkspaceMode::Aud => &[Issues, Scene],
        }
    }

    /// The tabs `mode` shows, in strip order. `has_clips` is whether the loaded
    /// model carries any animation, without which the Animations tab has nothing
    /// to list.
    pub(crate) fn available(
        mode: WorkspaceMode,
        has_clips: bool,
    ) -> impl Iterator<Item = OutlinerTab> {
        Self::for_mode(mode)
            .iter()
            .copied()
            .filter(move |tab| *tab != OutlinerTab::Animations || has_clips)
    }

    /// Whether `mode` ever shows this tab — the clip rule aside, which only
    /// matters to Animations.
    pub(crate) fn offered_in(self, mode: WorkspaceMode) -> bool {
        Self::for_mode(mode).contains(&self)
    }
}

/// The tab each workspace last showed, so leaving one workspace and coming back
/// lands on the same list rather than on whatever another workspace left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkspaceTabs {
    three_d: OutlinerTab,
    uv: OutlinerTab,
    texture: OutlinerTab,
    opt: OutlinerTab,
    aud: OutlinerTab,
}

impl Default for WorkspaceTabs {
    fn default() -> Self {
        Self {
            three_d: OutlinerTab::Scene,
            uv: OutlinerTab::Scene,
            texture: OutlinerTab::Textures,
            opt: OutlinerTab::Scene,
            aud: OutlinerTab::Issues,
        }
    }
}

impl WorkspaceTabs {
    /// The tab `mode` last showed. Not necessarily one it can show now — the
    /// Animations tab outlives a model with clips — so the Outliner resolves it
    /// against [`OutlinerTab::available`] before drawing.
    pub(crate) fn get(&self, mode: WorkspaceMode) -> OutlinerTab {
        match mode {
            WorkspaceMode::ThreeD => self.three_d,
            WorkspaceMode::Uv => self.uv,
            WorkspaceMode::Texture => self.texture,
            WorkspaceMode::Opt => self.opt,
            WorkspaceMode::Aud => self.aud,
        }
    }

    pub(crate) fn set(&mut self, mode: WorkspaceMode, tab: OutlinerTab) {
        let slot = match mode {
            WorkspaceMode::ThreeD => &mut self.three_d,
            WorkspaceMode::Uv => &mut self.uv,
            WorkspaceMode::Texture => &mut self.texture,
            WorkspaceMode::Opt => &mut self.opt,
            WorkspaceMode::Aud => &mut self.aud,
        };
        *slot = tab;
    }
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
    /// Which tab each workspace shows. Read through [`OutlinerState::tab`],
    /// which resolves it against what the workspace offers.
    pub(crate) tabs: WorkspaceTabs,
    /// Whether the Scene tab shows every node flat or the full scene tree.
    pub(crate) view: OutlinerViewMode,
    /// The Outliner header's search box. While non-empty it overrides
    /// [`OutlinerState::view`]: every tab collapses to a flat list of the rows
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
    /// The tab `mode` shows: the one it last showed when that is still on offer,
    /// else its first. `has_clips` is whether the model carries animation.
    pub(crate) fn tab(&self, mode: WorkspaceMode, has_clips: bool) -> OutlinerTab {
        let remembered = self.tabs.get(mode);
        let mut available = OutlinerTab::available(mode, has_clips);
        if OutlinerTab::available(mode, has_clips).any(|tab| tab == remembered) {
            remembered
        } else {
            // Every workspace offers at least one tab that needs no clips.
            available.next().unwrap_or_default()
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(mode: WorkspaceMode, has_clips: bool) -> Vec<OutlinerTab> {
        OutlinerTab::available(mode, has_clips).collect()
    }

    /// The per-workspace table, pinned: which lists each workspace shows is a
    /// product decision, not something a refactor should drift.
    #[test]
    fn each_workspace_offers_its_own_tabs() {
        use OutlinerTab::{Animations, Materials, Scene, Textures};
        assert_eq!(
            tabs(WorkspaceMode::ThreeD, true),
            [Scene, Materials, Textures, Animations]
        );
        assert_eq!(tabs(WorkspaceMode::Uv, true), [Scene, Textures]);
        assert_eq!(tabs(WorkspaceMode::Texture, true), [Textures]);
        assert_eq!(tabs(WorkspaceMode::Opt, true), [Scene, Materials]);
    }

    #[test]
    fn animations_need_clips() {
        assert!(!tabs(WorkspaceMode::ThreeD, false).contains(&OutlinerTab::Animations));
    }

    /// Every workspace has a tab to fall back on whatever the model carries.
    #[test]
    fn no_workspace_is_ever_left_without_a_tab() {
        for mode in [
            WorkspaceMode::ThreeD,
            WorkspaceMode::Uv,
            WorkspaceMode::Texture,
            WorkspaceMode::Opt,
        ] {
            assert!(!tabs(mode, false).is_empty(), "{mode:?}");
        }
    }

    /// Each workspace remembers its own tab, and a remembered tab the workspace
    /// can't show resolves to its first.
    #[test]
    fn tabs_are_remembered_per_workspace() {
        let mut outliner = OutlinerState::default();
        outliner
            .tabs
            .set(WorkspaceMode::ThreeD, OutlinerTab::Materials);
        assert_eq!(
            outliner.tab(WorkspaceMode::ThreeD, false),
            OutlinerTab::Materials
        );
        assert_eq!(outliner.tab(WorkspaceMode::Uv, false), OutlinerTab::Scene);
        assert_eq!(
            outliner.tab(WorkspaceMode::Texture, false),
            OutlinerTab::Textures
        );

        outliner
            .tabs
            .set(WorkspaceMode::ThreeD, OutlinerTab::Animations);
        assert_eq!(
            outliner.tab(WorkspaceMode::ThreeD, true),
            OutlinerTab::Animations
        );
        assert_eq!(
            outliner.tab(WorkspaceMode::ThreeD, false),
            OutlinerTab::Scene,
            "a model without clips falls back to the first tab"
        );
    }
}
