//! What a click does to the selection — shared by the Outliner's rows and the
//! viewport's pick, so the two can never drift into different rules.
//!
//! Both sets ([`UiState::selected_nodes`] and [`UiState::selected_bones`]) are
//! click-ordered lists whose last entry is the primary, mirrored into
//! [`UiState::selection`]. Only one of them is ever populated: selecting a bone
//! clears the mesh set and vice versa, because the skeleton highlight and the
//! skin-weight heat map read the bone set and must not outlive it.

use review_render::Selection;

use super::{HoverTarget, OutlinerTab, UiState, WorkspaceMode};
use crate::opt_state::StackItem;

/// Which set a click is editing. The two behave identically; they differ only in
/// which list they write and which one they clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionKind {
    /// A mesh or group node.
    Node,
    /// A bone node.
    Bone,
}

/// How a click combines with what is already selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectMode {
    /// Add the target if absent, remove it if present (the primary modifier).
    pub(crate) toggle: bool,
    /// Add the target, leaving an already-selected one selected (Shift).
    pub(crate) add: bool,
}

impl SelectMode {
    /// A mode from the two decisions directly, for a caller that reads its own
    /// modifiers — `app` does, because the primary modifier is `Cmd` on macOS
    /// and it already owns that distinction for every other shortcut.
    pub fn new(toggle: bool, add: bool) -> Self {
        Self { toggle, add }
    }

    /// The modifier state as a mode. Toggle wins when both are held, so a user
    /// rolling their fingers across `Ctrl`+`Shift` gets the one that can still
    /// *remove* something rather than the one that cannot.
    pub fn from_modifiers(modifiers: egui::Modifiers) -> Self {
        Self {
            toggle: modifiers.command || modifiers.ctrl,
            add: modifiers.shift,
        }
    }

    /// Whether any modifier is held — i.e. whether this click is editing an
    /// existing selection rather than replacing it.
    pub fn is_editing(self) -> bool {
        self.toggle || self.add
    }
}

impl UiState {
    /// The list a `kind` of selection lives in. Handed out so the Outliner's
    /// Shift-range can write a whole run at once; every other edit goes through
    /// the methods below.
    pub(crate) fn selection_set_for(&mut self, kind: SelectionKind) -> &mut Vec<usize> {
        match kind {
            SelectionKind::Node => &mut self.selected_nodes,
            SelectionKind::Bone => &mut self.selected_bones,
        }
    }

    /// Empty the set a `kind` of selection does *not* live in. A click never
    /// leaves both populated.
    fn clear_other_set(&mut self, kind: SelectionKind) {
        match kind {
            SelectionKind::Node => self.selected_bones.clear(),
            SelectionKind::Bone => self.selected_nodes.clear(),
        }
    }

    /// Point [`UiState::selection`] at the set's last member, or clear it when
    /// the set has emptied — rather than leaving a stale primary behind.
    fn follow_primary(&mut self, kind: SelectionKind) {
        self.selection = match self.selection_set_for(kind).last() {
            Some(&last) => Selection::Node(last),
            None => Selection::None,
        };
    }

    /// Hand the Inspector over to the scene: whatever the Opt stack pane or the
    /// Textures tab had selected gives it up.
    ///
    /// The Inspector shows one thing, and several surfaces can fill it — the Opt
    /// stack pane's rows, the Textures tab's, and the scene's. They are therefore
    /// exclusive in both directions: every scene selection comes through here,
    /// and [`UiState::select_stack_item`] / [`UiState::select_texture`] clear or
    /// outrank the scene selection the same way. Without it a selected operation
    /// simply outranked the Outliner and kept the panel however much the user
    /// clicked around the tree.
    fn release_stack_selection(&mut self) {
        self.opt.selected = None;
        self.texture_view.inspected = false;
        self.comments.inspected = false;
    }

    /// Make pooled texture `index` the current one and show it in the Inspector.
    ///
    /// Unlike a material, a texture is not a scene selection: nothing in the
    /// viewport highlights it, so the node or material the user had selected
    /// stays selected (and lit) underneath, and gets the Inspector back the
    /// moment anything in the scene is clicked.
    pub(crate) fn select_texture(&mut self, index: usize) {
        self.texture_view.selected = index;
        self.texture_view.inspected = true;
        self.comments.inspected = false;
    }

    /// Select review-comment thread `index` and show it in the Inspector. With
    /// `select_host`, the object it is stored on becomes the scene selection too
    /// (a click in the Comments tab, which goes to the comment); a click on a pin
    /// leaves the scene selection alone, since the user is already looking at it.
    pub(crate) fn select_comment(&mut self, index: usize, select_host: bool) {
        if select_host
            && let Some(node) = self
                .comments
                .threads
                .get(index)
                .and_then(|entry| entry.node)
        {
            // Through the normal path, which hands the Inspector over to the
            // scene — so the comment takes it back below.
            self.select_only(node, SelectionKind::Node);
            self.outliner.scroll_to_selection = true;
        }
        self.comments.selected = Some(index);
        self.comments.inspected = true;
        self.texture_view.inspected = false;
    }

    /// Post the comment being written and show it in the Inspector.
    pub(crate) fn post_comment(&mut self) {
        if let Some(index) = self.comments.post_draft() {
            self.select_comment(index, false);
        }
    }

    /// The clip and frame on screen, as a one-frame range — what a new comment
    /// offers to be about.
    pub fn current_frames(
        &self,
        model: &review_model::ModelData,
    ) -> Option<review_annotate::thread::FrameRange> {
        let clip = model.animations.get(self.animation.selected_clip?)?;
        let frame = clip.frame_at(self.animation.time, model.frame_rate_or_default()) as u32;
        Some(review_annotate::thread::FrameRange {
            clip: clip.name.clone(),
            start: frame,
            end: frame,
        })
    }

    /// Start a comment from the chrome — about an object picked in the Outliner,
    /// or about the whole file — opening its composer at `screen`.
    pub(crate) fn start_comment(
        &mut self,
        model: &review_model::ModelData,
        anchor: crate::state::DraftAnchor,
        screen: egui::Pos2,
    ) {
        let Some(view) = self.comments.view_now else {
            return;
        };
        let frames = self.current_frames(model);
        self.comments.begin_draft(anchor, frames, view, screen);
    }

    /// Whether the Inspector is showing the selected review comment: after a
    /// click in the Comments tab or on a pin, until the next scene selection, and
    /// only in a workspace that lists comments.
    pub(crate) fn comment_inspected(&self) -> bool {
        self.comments.inspected
            && self
                .comments
                .selected
                .is_some_and(|index| index < self.comments.threads.len())
            && OutlinerTab::Comments.offered_in(self.mode)
    }

    /// Whether the Inspector is showing the current texture rather than the
    /// scene selection: always in the Tex workspace, which has nothing else to
    /// show, and elsewhere only after a Textures-tab click and only in a
    /// workspace that offers that tab — Opt never does, so a texture picked in 3D
    /// can't hide the operation settings there.
    pub(crate) fn texture_inspected(&self) -> bool {
        self.mode == WorkspaceMode::Texture
            || (self.texture_view.inspected && OutlinerTab::Textures.offered_in(self.mode))
    }

    /// Select a stack row (an operation or the export settings), taking the
    /// Inspector from whatever the Outliner had selected.
    pub(crate) fn select_stack_item(&mut self, item: StackItem) {
        self.clear_selection();
        self.opt.selected = Some(item);
    }

    /// Select `material` (or [`Selection::None`]) as the whole selection. A
    /// material covers triangles across any number of nodes, so neither node set
    /// describes it; both go, along with the range anchor.
    pub(crate) fn select_material(&mut self, material: Selection) {
        self.release_stack_selection();
        self.selection = material;
        self.selected_nodes.clear();
        self.selected_bones.clear();
        self.row_anchor = None;
    }

    /// Select `node` alone, as the whole set and the range anchor.
    pub(crate) fn select_only(&mut self, node: usize, kind: SelectionKind) {
        self.release_stack_selection();
        self.clear_other_set(kind);
        let set = self.selection_set_for(kind);
        set.clear();
        set.push(node);
        self.selection = Selection::Node(node);
        self.row_anchor = Some(node);
    }

    /// Apply a modified click on `node` to the set `kind` names.
    ///
    /// * toggle — flip its membership; the primary follows the set's new last
    ///   member, and empties to [`Selection::None`].
    /// * add — put it at the end of the set (moving it there if it was already
    ///   in), making it the primary.
    ///
    /// Returns `false` when `mode` asked for neither, so the caller can apply
    /// whatever it means by a plain click (which differs: the Outliner toggles a
    /// lone selection off, the viewport replaces it).
    pub(crate) fn apply_select_mode(
        &mut self,
        node: usize,
        kind: SelectionKind,
        mode: SelectMode,
    ) -> bool {
        if !mode.is_editing() {
            return false;
        }
        self.release_stack_selection();
        self.clear_other_set(kind);
        let set = self.selection_set_for(kind);
        let at = set.iter().position(|&member| member == node);
        match (mode.toggle, at) {
            (true, Some(at)) => {
                set.remove(at);
            }
            (true, None) => set.push(node),
            (false, Some(at)) => {
                // Re-adding a member makes it the primary rather than
                // duplicating it, so the Inspector follows the last thing the
                // user actually pointed at.
                set.remove(at);
                set.push(node);
            }
            (false, None) => set.push(node),
        }
        self.row_anchor = Some(node);
        self.follow_primary(kind);
        true
    }
}

/// Apply a viewport pick to the selection: `target` is what the ray hit, or
/// `None` for empty space.
///
/// The same rules the Outliner's rows follow, with one difference the two
/// surfaces genuinely need: there is no row order in a 3D view to sweep along,
/// so Shift here *adds* the one thing clicked rather than taking a range.
///
/// Clicking empty space clears — but only on a plain click. With a modifier
/// held the user is part-way through building a selection, and a miss must not
/// throw it away: a near-miss is the most likely click there is, and it would be
/// the most expensive one to punish.
pub fn apply_pick(state: &mut UiState, target: Option<HoverTarget>, mode: SelectMode) {
    let Some(target) = target else {
        if !mode.is_editing() {
            state.clear_selection();
        }
        return;
    };
    let (node, kind) = match target {
        HoverTarget::Node(node) => (node, SelectionKind::Node),
        HoverTarget::Bone(bone) => (bone, SelectionKind::Bone),
    };
    if !state.apply_select_mode(node, kind, mode) {
        // A plain click replaces, and does *not* toggle the way an Outliner row
        // does: clicking a thing in the viewport says "this one", and a model
        // where a second click deselected would make a double-click on a part
        // land on nothing.
        state.select_only(node, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(toggle: bool, add: bool) -> SelectMode {
        SelectMode { toggle, add }
    }

    #[test]
    fn toggle_adds_then_removes_and_the_primary_follows() {
        let mut state = UiState::default();
        state.select_only(4, SelectionKind::Node);
        assert!(state.apply_select_mode(7, SelectionKind::Node, mode(true, false)));
        assert_eq!(state.selected_nodes, vec![4, 7]);
        assert_eq!(state.selection, Selection::Node(7));

        state.apply_select_mode(7, SelectionKind::Node, mode(true, false));
        assert_eq!(state.selected_nodes, vec![4]);
        assert_eq!(state.selection, Selection::Node(4));

        // Emptying the set clears the primary rather than leaving a stale node.
        state.apply_select_mode(4, SelectionKind::Node, mode(true, false));
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.selection, Selection::None);
    }

    #[test]
    fn add_never_removes_and_re_adding_only_moves_the_primary() {
        let mut state = UiState::default();
        state.select_only(4, SelectionKind::Node);
        state.apply_select_mode(7, SelectionKind::Node, mode(false, true));
        state.apply_select_mode(4, SelectionKind::Node, mode(false, true));
        assert_eq!(
            state.selected_nodes,
            vec![7, 4],
            "no duplicate, moved to last"
        );
        assert_eq!(state.selection, Selection::Node(4));
    }

    #[test]
    fn toggle_wins_when_both_modifiers_are_held() {
        let mut state = UiState::default();
        state.select_only(4, SelectionKind::Node);
        state.apply_select_mode(4, SelectionKind::Node, mode(true, true));
        assert!(state.selected_nodes.is_empty(), "toggle removed it");
    }

    #[test]
    fn a_plain_click_is_left_to_the_caller() {
        let mut state = UiState::default();
        assert!(!state.apply_select_mode(4, SelectionKind::Node, mode(false, false)));
        assert!(state.selected_nodes.is_empty());
    }

    #[test]
    fn the_two_sets_never_hold_each_other() {
        let mut state = UiState::default();
        state.select_only(4, SelectionKind::Node);
        state.apply_select_mode(5, SelectionKind::Node, mode(true, false));
        assert_eq!(state.selected_nodes.len(), 2);

        state.select_only(9, SelectionKind::Bone);
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.selected_bones, vec![9]);

        state.apply_select_mode(3, SelectionKind::Node, mode(true, false));
        assert!(state.selected_bones.is_empty());
        assert_eq!(state.selected_nodes, vec![3]);
    }

    /// The Inspector shows one thing, so the Outliner and the Opt stack pane
    /// cannot both hold a selection — in either direction.
    #[test]
    fn the_scene_and_the_stack_never_hold_a_selection_at_once() {
        let mut state = UiState::default();

        state.select_stack_item(StackItem::Op(7));
        assert_eq!(state.opt.selected, Some(StackItem::Op(7)));

        state.select_only(4, SelectionKind::Node);
        assert_eq!(state.opt.selected, None, "a node click takes the Inspector");

        state.select_stack_item(StackItem::ExportSettings);
        assert!(!state.has_selection(), "the stack row takes it back");

        state.apply_select_mode(4, SelectionKind::Bone, mode(true, false));
        assert_eq!(state.opt.selected, None, "so does a modified click");

        state.select_stack_item(StackItem::Op(7));
        state.select_material(Selection::Material(2));
        assert_eq!(state.opt.selected, None, "and a material click");
        assert_eq!(state.selection, Selection::Material(2));
    }

    #[test]
    fn clear_selection_drops_everything() {
        let mut state = UiState::default();
        state.select_only(4, SelectionKind::Bone);
        assert!(state.has_selection());
        state.clear_selection();
        assert!(!state.has_selection());
        assert_eq!(state.selection, Selection::None);
        assert!(state.selected_bones.is_empty());
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.row_anchor, None);
    }

    #[test]
    fn a_viewport_click_replaces_and_does_not_toggle_off() {
        let mut state = UiState::default();
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(false, false));
        assert_eq!(state.selected_nodes, vec![4]);
        // Unlike an Outliner row, clicking it again keeps it selected.
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(false, false));
        assert_eq!(state.selected_nodes, vec![4]);
        assert_eq!(state.selection, Selection::Node(4));
    }

    #[test]
    fn a_viewport_click_takes_the_two_modifiers() {
        let mut state = UiState::default();
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(false, false));
        apply_pick(&mut state, Some(HoverTarget::Node(7)), mode(false, true));
        assert_eq!(state.selected_nodes, vec![4, 7], "shift adds");
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(true, false));
        assert_eq!(state.selected_nodes, vec![7], "ctrl removes");
    }

    #[test]
    fn clicking_empty_space_clears_only_without_a_modifier() {
        let mut state = UiState::default();
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(false, false));
        apply_pick(&mut state, Some(HoverTarget::Node(7)), mode(true, false));

        // A near miss while building a selection must not throw it away.
        apply_pick(&mut state, None, mode(true, false));
        apply_pick(&mut state, None, mode(false, true));
        assert_eq!(state.selected_nodes, vec![4, 7]);

        apply_pick(&mut state, None, mode(false, false));
        assert!(!state.has_selection());
    }

    #[test]
    fn picking_a_bone_clears_the_part_selection() {
        let mut state = UiState::default();
        apply_pick(&mut state, Some(HoverTarget::Node(4)), mode(false, false));
        apply_pick(&mut state, Some(HoverTarget::Bone(9)), mode(false, false));
        assert!(state.selected_nodes.is_empty());
        assert_eq!(state.selected_bones, vec![9]);
        assert_eq!(state.selection, Selection::Node(9));
    }

    #[test]
    fn a_lone_primary_still_reports_as_a_set() {
        // An undo restore or an Opt click sets the scalar with no set behind it.
        let mut state = UiState {
            selection: Selection::Node(6),
            ..UiState::default()
        };
        assert_eq!(state.selected_node_set(), vec![6]);
        assert!(state.has_selection());
        // A material selection covers no nodes.
        state.selection = Selection::Material(2);
        assert!(state.selected_node_set().is_empty());
    }
}
