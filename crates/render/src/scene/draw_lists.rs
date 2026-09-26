//! The per-draw index ordering: which parts are drawn, and in what order.
//!
//! Selection and visibility reorder the index buffer rather than rebuilding
//! geometry, so toggling either costs an index upload and nothing else.

use review_model::ModelData;

use crate::MaterialMode;
use crate::geometry::{selection_geometry, visible_geometry};
use crate::rhi::{GpuResult, IndexBuffer};
use crate::selection::{Selection, SelectionView};

use super::gpu::SceneGpu;
use super::slot::{HoverBaked, SelectionBaked, VisibilityBaked};

impl SceneGpu {
    /// The per-triangle grouping key for `mode`: the cached mesh-part key in Unique
    /// mode (when the model carries per-triangle node info), else `None` to group by
    /// material slot. Call after `sync_unique_parts`.
    pub(super) fn grouping_key(&self, mode: MaterialMode) -> Option<&[u32]> {
        match mode {
            MaterialMode::Unique if !self.active.unique_part_key.is_empty() => {
                Some(&self.active.unique_part_key)
            }
            _ => None,
        }
    }

    /// Build (or free) the selected-triangle draw list (the solo isolate list + the
    /// highlight fill source) when the selection / model / hidden set / mode drifts
    /// (invariant 3). The highlight color + fill opacity ride in the uniform, so they
    /// never trigger a rebuild — only a change of *what* is selected does.
    pub(super) fn sync_selection(
        &mut self,
        model: &ModelData,
        model_revision: u64,
        view: SelectionView,
        selected_nodes: &[u32],
        hidden: &[u32],
        mode: MaterialMode,
    ) -> GpuResult<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let active = view.selection.is_active();
        let unchanged = match (&self.active.selection_baked, active) {
            (None, false) => true,
            (Some(baked), true) => {
                baked.model_revision == model_revision
                    && baked.selection == view.selection
                    && baked.selected_nodes == selected_nodes
                    && baked.hidden == hidden
                    && baked.mode == mode
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        // Resolve the visible selected triangles, grouped by the same key as the main
        // mesh so each range binds the right effective material. `None` (no usable
        // geometry) or an empty list both clear to a no-draw selection.
        let geometry = if active {
            let key = self.grouping_key(mode);
            selection_geometry(model, view.selection, selected_nodes, hidden, key)
        } else {
            None
        };
        match geometry {
            Some((indices, ranges)) if !indices.is_empty() => {
                self.active.selection_index = Some(IndexBuffer::new(&indices, c"mesh")?);
                self.active.selection_ranges = ranges;
            }
            _ => {
                self.active.selection_index = None;
                self.active.selection_ranges = Vec::new();
            }
        }
        self.active.selection_baked = active.then(|| SelectionBaked {
            model_revision,
            selection: view.selection,
            selected_nodes: selected_nodes.to_vec(),
            hidden: hidden.to_vec(),
            mode,
        });
        Ok(())
    }

    /// Build (or free) the hovered node's draw list — the fill that previews what
    /// a click would select (invariant 3: it exists only while something is
    /// hovered).
    ///
    /// The same geometry the selection uses, over one node: hovering is exactly
    /// "what would be selected", so sharing [`selection_geometry`] is what keeps
    /// the preview and the result the same shape. The tint rides in the uniform,
    /// so only a change of *which* node is under the pointer costs an index
    /// upload — moving within one node costs nothing at all.
    pub(super) fn sync_hover(
        &mut self,
        model: &ModelData,
        model_revision: u64,
        hover: Option<u32>,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> GpuResult<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let unchanged = match (&self.active.hover_baked, hover) {
            (None, None) => true,
            (Some(baked), Some(node)) => {
                baked.model_revision == model_revision
                    && baked.node == node
                    && baked.hidden == hidden
                    && baked.mode == mode
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        let geometry = hover.and_then(|node| {
            let key = self.grouping_key(mode);
            selection_geometry(model, Selection::Node(node as usize), &[], hidden, key)
        });
        match geometry {
            Some((indices, _)) if !indices.is_empty() => {
                self.active.hover_index = Some(IndexBuffer::new(&indices, c"hover")?);
            }
            _ => self.active.hover_index = None,
        }
        self.active.hover_baked = hover.map(|node| HoverBaked {
            model_revision,
            node,
            hidden: hidden.to_vec(),
            mode,
        });
        Ok(())
    }

    /// Build (or free) the per-mesh visibility draw list when the hidden set / model /
    /// mode drifts (invariant 3). `hidden` empty means nothing is hidden (full mesh,
    /// no filter). When every mesh is hidden the filter is active but the list empty
    /// (draw nothing); when the model carries no per-triangle node info the filter is
    /// off (full mesh).
    pub(super) fn sync_visibility(
        &mut self,
        model: &ModelData,
        model_revision: u64,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> GpuResult<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let active = !hidden.is_empty();
        let unchanged = match (&self.active.visibility_baked, active) {
            (None, false) => true,
            (Some(baked), true) => {
                baked.model_revision == model_revision
                    && baked.hidden == hidden
                    && baked.mode == mode
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        // Past here the list is genuinely being rebuilt, so anything keyed on the
        // visibility (the AO accumulation) has to start over.
        self.active.visibility_generation = self.active.visibility_generation.wrapping_add(1);
        let geometry = if active {
            let key = self.grouping_key(mode);
            visible_geometry(model, hidden, key)
        } else {
            None
        };
        match geometry {
            // Some unhidden geometry: draw the filtered list.
            Some((indices, ranges)) if !indices.is_empty() => {
                self.active.visible_index = Some(IndexBuffer::new(&indices, c"mesh")?);
                self.active.visible_ranges = ranges;
                self.active.visible_active = true;
            }
            // Every mesh hidden: the filter is active but draws nothing.
            Some(_) => {
                self.active.visible_index = None;
                self.active.visible_ranges = Vec::new();
                self.active.visible_active = true;
            }
            // Nothing hidden, or the model carries no per-triangle node info: draw the
            // full mesh (no filter).
            None => {
                self.active.visible_index = None;
                self.active.visible_ranges = Vec::new();
                self.active.visible_active = false;
            }
        }
        self.active.visibility_baked = active.then(|| VisibilityBaked {
            model_revision,
            hidden: hidden.to_vec(),
            mode,
        });
        Ok(())
    }
}
