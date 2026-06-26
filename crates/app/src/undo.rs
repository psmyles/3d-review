//! The unified undo/redo system (a snapshot / memento stack + an `impl App`
//! block).
//!
//! Every *document edit* the user makes — Outliner selection, mesh hide/unhide,
//! material parameters, texture slot bindings, and the scene texture pool — is
//! captured as a single [`EditSnapshot`]. The [`UndoStack`] keeps a baseline plus
//! an undo/redo history of those snapshots; `Ctrl+Z` / `Ctrl+Y` (or
//! `Ctrl+Shift+Z`) restore one. *View* state (camera, grid/shading/AA/bloom/AO/
//! tonemap/environment, UV/Tex viewports) is deliberately **not** captured.
//!
//! Why snapshots, not a command pattern: `app` is the single coordinator that
//! owns/touches all of this state (invariant 2), and the heavy parts already have
//! cheap monotonic change-detection (the renderer's `material_revision`, and an
//! app-side `texture_revision`). A snapshot design is what makes the system
//! **extensible** — a new undoable field is one line in [`EditSnapshot`], one line
//! in [`App::capture_edit_state`], and one line in [`App::restore_edit_state`];
//! no per-action inverse-command code. The material table and texture pool are the
//! only large parts and are **structurally shared** across snapshots via `Arc`,
//! re-cloned only when their revision says they actually changed, so a
//! selection-only edit costs almost nothing.
//!
//! Recording is driven once per frame by [`App::observe_edit_state`] (top of
//! `render`, the same one-frame-behind cadence the selection flash uses): it
//! builds the live snapshot and lets the stack diff it against the baseline. A
//! change to a cheap field (selection/solo/hidden) or a revision bump (materials /
//! pool) is caught automatically — that is the extensibility win. The lone special
//! case is a continuous drag (a material slider / color-picker), coalesced into a
//! single undo step via the `dragging` flag so a drag isn't 30 entries.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use review_render::{DecodedImage, MaterialState, Selection};

use crate::App;

/// Cap on each of the undo / redo stacks, so a long editing session can't grow
/// them unbounded. Older entries fall off the bottom once the cap is reached.
const UNDO_LIMIT: usize = 128;

/// The app-side texture pool captured in a snapshot: the ordered pool membership
/// plus the decoded-image cache. Images are shared by `Arc`, so cloning this for a
/// snapshot bumps refcounts rather than copying pixels.
#[derive(Debug, Clone, Default)]
pub(crate) struct TexturePool {
    pub pool: Vec<PathBuf>,
    pub cache: HashMap<PathBuf, Arc<DecodedImage>>,
}

/// One captured point of all undoable document state. Cheap fields are stored by
/// value; the heavy material table + texture pool are stored behind `Arc` and
/// shared across snapshots that didn't change them (gated by the revision tags).
#[derive(Debug, Clone)]
pub(crate) struct EditSnapshot {
    selection: Selection,
    solo: bool,
    hidden_meshes: HashSet<usize>,
    /// The renderer's editable material table (scalar params + texture slot
    /// bindings). `material_revision` is the change tag — equal revisions mean
    /// equal tables, so the `Arc` is shared instead of re-cloned.
    materials: Arc<Vec<MaterialState>>,
    material_revision: u64,
    /// The scene texture pool + decode cache, tagged by `texture_revision`.
    textures: Arc<TexturePool>,
    texture_revision: u64,
}

impl EditSnapshot {
    /// The empty baseline used before any model is loaded.
    fn empty() -> Self {
        Self {
            selection: Selection::None,
            solo: false,
            hidden_meshes: HashSet::new(),
            materials: Arc::new(Vec::new()),
            material_revision: 0,
            textures: Arc::new(TexturePool::default()),
            texture_revision: 0,
        }
    }

    /// Whether two snapshots represent different document state. The cheap fields
    /// compare by value; the heavy parts compare by their monotonic revision tag
    /// (any mutation bumps it), so this never deep-compares the material table or
    /// the texture cache.
    fn differs(&self, other: &EditSnapshot) -> bool {
        self.selection != other.selection
            || self.solo != other.solo
            || self.hidden_meshes != other.hidden_meshes
            || self.material_revision != other.material_revision
            || self.texture_revision != other.texture_revision
    }
}

/// The undo/redo history: a committed `baseline` plus two stacks of snapshots, and
/// an open-drag transaction used to coalesce a continuous edit into one step.
#[derive(Debug)]
pub(crate) struct UndoStack {
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    /// The last committed state. The next observation diffs against this.
    baseline: EditSnapshot,
    /// The pre-drag baseline while a continuous drag is in progress: captured once
    /// at drag start, committed as a single entry when the drag ends.
    drag_open: Option<EditSnapshot>,
}

impl UndoStack {
    pub(crate) fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            baseline: EditSnapshot::empty(),
            drag_open: None,
        }
    }

    /// Drop all history and adopt a fresh baseline (a new model loaded / reset):
    /// the stored snapshots reference the previous model's indices / materials.
    fn reset(&mut self, baseline: EditSnapshot) {
        self.undo.clear();
        self.redo.clear();
        self.drag_open = None;
        self.baseline = baseline;
    }

    fn baseline(&self) -> &EditSnapshot {
        &self.baseline
    }

    fn set_baseline(&mut self, baseline: EditSnapshot) {
        self.baseline = baseline;
    }

    /// Fold this frame's live snapshot into the history. `dragging` is true while a
    /// material slider / color-picker is being actively dragged, which collapses
    /// the whole drag into one undo step.
    fn observe(&mut self, live: EditSnapshot, dragging: bool) {
        if dragging {
            // Open the transaction once (capturing the pre-drag baseline) and hold
            // off recording; the live edits still apply, we just don't push a step
            // per intermediate value. Keep the baseline current so the post-drag
            // diff is pre-drag vs final.
            if self.drag_open.is_none() {
                self.drag_open = Some(self.baseline.clone());
            }
            self.baseline = live;
            return;
        }
        if let Some(pre) = self.drag_open.take() {
            // The drag just ended: commit it as a single step if it changed
            // anything net.
            if pre.differs(&live) {
                self.push_undo_entry(pre);
                self.redo.clear();
            }
            self.baseline = live;
            return;
        }
        // A discrete edit: record the prior state and clear the redo branch.
        if self.baseline.differs(&live) {
            let pre = self.baseline.clone();
            self.push_undo_entry(pre);
            self.redo.clear();
            self.baseline = live;
        }
    }

    /// Pop the most recent undo step, banking `current` for redo. Returns the
    /// snapshot the caller should restore, or `None` when nothing can be undone.
    fn pop_undo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        let target = self.undo.pop()?;
        self.push_redo_entry(current);
        Some(target)
    }

    /// Pop the most recent redo step, banking `current` back onto undo.
    fn pop_redo(&mut self, current: EditSnapshot) -> Option<EditSnapshot> {
        let target = self.redo.pop()?;
        self.push_undo_entry(current);
        Some(target)
    }

    fn push_undo_entry(&mut self, snapshot: EditSnapshot) {
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }

    fn push_redo_entry(&mut self, snapshot: EditSnapshot) {
        if self.redo.len() >= UNDO_LIMIT {
            self.redo.remove(0);
        }
        self.redo.push(snapshot);
    }
}

impl App {
    /// Build the live snapshot of all undoable state, reusing the baseline's `Arc`s
    /// for the heavy parts whose revision tag hasn't moved (the cheap path: a
    /// selection edit shares the same material / texture `Arc`s as before).
    fn capture_edit_state(&self) -> EditSnapshot {
        let prev = self.undo.baseline();

        let material_revision = self
            .renderer
            .as_ref()
            .map_or(0, |renderer| renderer.material_revision());
        let materials = if prev.material_revision == material_revision {
            Arc::clone(&prev.materials)
        } else {
            let states = self
                .renderer
                .as_ref()
                .map_or_else(Vec::new, |renderer| renderer.material_states().to_vec());
            Arc::new(states)
        };

        let textures = if prev.texture_revision == self.texture_revision {
            Arc::clone(&prev.textures)
        } else {
            Arc::new(TexturePool {
                pool: self.texture_pool.clone(),
                cache: self.texture_cache.clone(),
            })
        };

        EditSnapshot {
            selection: self.ui.selection,
            solo: self.ui.solo,
            hidden_meshes: self.ui.hidden_meshes.clone(),
            materials,
            material_revision,
            textures,
            texture_revision: self.texture_revision,
        }
    }

    /// Restore all undoable state from a snapshot (the undo/redo target). Replaces
    /// the renderer's whole material table (re-uploaded on the next frame when its
    /// revision bumps), the texture pool + decode cache, and the UI's selection /
    /// solo / hidden set, then re-registers disk watches and refreshes the UI
    /// mirrors. The decoded images live in the snapshot by `Arc`, so a texture
    /// removed from the pool since is rebound correctly.
    fn restore_edit_state(&mut self, snapshot: &EditSnapshot) {
        self.ui.selection = snapshot.selection;
        self.ui.solo = snapshot.solo;
        self.ui.hidden_meshes = snapshot.hidden_meshes.clone();
        // Any restored selection should frame the part first on the next `F`.
        self.frame_showing_selection = false;

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.restore_materials((*snapshot.materials).clone());
        }

        self.texture_pool = snapshot.textures.pool.clone();
        self.texture_cache = snapshot.textures.cache.clone();
        // The pool/cache changed wholesale; bump so the next capture rebuilds the
        // textures `Arc` for the baseline rather than reusing a stale one.
        self.texture_revision = self.texture_revision.wrapping_add(1);
        // Re-watch each restored path's directory (idempotent per directory; stale
        // extra watches on no-longer-pooled dirs are harmless — a reload event for
        // an unbound path is a no-op).
        let pooled: Vec<PathBuf> = self.texture_pool.clone();
        for path in &pooled {
            self.watch_texture(path);
        }

        self.refresh_materials();
        self.refresh_texture_pool();
        self.redraw_requested = true;
    }

    /// Drop all undo/redo history and rebaseline to the current state. Called after
    /// a model load / reset has cleared selection / materials / the texture pool,
    /// so the stored snapshots (old model's indices) can't be restored.
    pub(crate) fn reset_undo_history(&mut self) {
        self.frame_showing_selection = false;
        self.drag_in_progress = false;
        let baseline = self.capture_edit_state();
        self.undo.reset(baseline);
    }

    /// Record any edit committed since the last frame into the undo history. Run
    /// once per frame at the top of `render`, before this frame's egui pass: an
    /// Outliner click / material edit lands during the egui run, so it's observed
    /// (and recorded) on the following frame.
    pub(crate) fn observe_edit_state(&mut self) {
        let live = self.capture_edit_state();
        // Reset the `F`-frame toggle whenever the selection itself changes, so a
        // fresh selection frames the part first (was piggybacked on the old
        // selection-only recorder).
        if live.selection != self.undo.baseline().selection {
            self.frame_showing_selection = false;
        }
        let dragging = self.drag_in_progress;
        self.undo.observe(live, dragging);
    }

    /// Commit any edit not yet folded into the history before an undo/redo. The
    /// per-frame observer runs at the top of `render`, so an edit (or a just-ended
    /// drag) made since the last frame isn't recorded yet; flushing here makes
    /// `Ctrl+Z`/`Ctrl+Y` correct regardless of render timing. The user released the
    /// pointer to press the chord, so any open drag has genuinely ended.
    fn flush_pending_edit(&mut self) {
        self.drag_in_progress = false;
        self.observe_edit_state();
    }

    /// Undo the most recent edit (`Ctrl+Z`).
    pub(crate) fn undo(&mut self) {
        self.flush_pending_edit();
        let current = self.capture_edit_state();
        let Some(target) = self.undo.pop_undo(current) else {
            return;
        };
        self.restore_edit_state(&target);
        // Restoring bumped the material / texture revisions; resync the baseline to
        // the real post-restore state so the next observation doesn't re-record it.
        let baseline = self.capture_edit_state();
        self.undo.set_baseline(baseline);
    }

    /// Redo the most recently undone edit (`Ctrl+Y` / `Ctrl+Shift+Z`).
    pub(crate) fn redo(&mut self) {
        self.flush_pending_edit();
        let current = self.capture_edit_state();
        let Some(target) = self.undo.pop_redo(current) else {
            return;
        };
        self.restore_edit_state(&target);
        let baseline = self.capture_edit_state();
        self.undo.set_baseline(baseline);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a snapshot carrying just the fields the stack diffs on, so the pure
    /// stack logic can be exercised without a renderer / GPU.
    fn snap(selection: Selection, material_revision: u64, texture_revision: u64) -> EditSnapshot {
        EditSnapshot {
            selection,
            solo: false,
            hidden_meshes: HashSet::new(),
            materials: Arc::new(Vec::new()),
            material_revision,
            textures: Arc::new(TexturePool::default()),
            texture_revision,
        }
    }

    #[test]
    fn discrete_edits_each_record_one_step() {
        let mut stack = UndoStack::new();
        stack.reset(snap(Selection::None, 0, 0));

        // Two distinct selection changes → two undo entries.
        stack.observe(snap(Selection::Node(1), 0, 0), false);
        stack.observe(snap(Selection::Node(2), 0, 0), false);
        assert_eq!(stack.undo.len(), 2);
        assert!(stack.redo.is_empty());

        // An unchanged observation records nothing.
        stack.observe(snap(Selection::Node(2), 0, 0), false);
        assert_eq!(stack.undo.len(), 2);
    }

    #[test]
    fn undo_then_redo_round_trips_and_new_edit_clears_redo() {
        let mut stack = UndoStack::new();
        stack.reset(snap(Selection::None, 0, 0));
        stack.observe(snap(Selection::Node(1), 0, 0), false);
        stack.observe(snap(Selection::Material(3), 0, 0), false);

        // Undo restores the prior target and banks the current for redo.
        let target = stack.pop_undo(snap(Selection::Material(3), 0, 0)).unwrap();
        assert_eq!(target.selection, Selection::Node(1));
        assert_eq!(stack.redo.len(), 1);
        stack.set_baseline(target);

        // Redo returns to where we were.
        let target = stack.pop_redo(snap(Selection::Node(1), 0, 0)).unwrap();
        assert_eq!(target.selection, Selection::Material(3));
        stack.set_baseline(target);

        // A fresh edit after an undo clears the redo branch.
        let _ = stack.pop_undo(snap(Selection::Material(3), 0, 0));
        stack.set_baseline(snap(Selection::Node(1), 0, 0));
        assert!(!stack.redo.is_empty());
        stack.observe(snap(Selection::Node(9), 0, 0), false);
        assert!(stack.redo.is_empty());
    }

    #[test]
    fn a_continuous_drag_collapses_to_one_step() {
        let mut stack = UndoStack::new();
        stack.reset(snap(Selection::None, 0, 0));

        // A material slider drag bumps the revision every frame while `dragging`.
        stack.observe(snap(Selection::None, 1, 0), true);
        stack.observe(snap(Selection::None, 2, 0), true);
        stack.observe(snap(Selection::None, 3, 0), true);
        // No steps recorded mid-drag.
        assert!(stack.undo.is_empty());

        // Releasing the drag commits exactly one step (pre-drag → final).
        stack.observe(snap(Selection::None, 3, 0), false);
        assert_eq!(stack.undo.len(), 1);
        assert_eq!(stack.undo[0].material_revision, 0);
    }

    #[test]
    fn a_drag_that_nets_no_change_records_nothing() {
        let mut stack = UndoStack::new();
        stack.reset(snap(Selection::None, 5, 0));
        // Pointer held but the value never moved (same revision throughout).
        stack.observe(snap(Selection::None, 5, 0), true);
        stack.observe(snap(Selection::None, 5, 0), false);
        assert!(stack.undo.is_empty());
    }
}
