//! The material and texture half of the [`Renderer`] façade.
//!
//! Materials are edited as *previews of a look*, not as authored asset data —
//! which is why an export writes the materials the source file declared rather
//! than these. The uploaded table and its path-keyed cache live in
//! [`crate::material`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use review_model::MaterialImportDefaults;

use crate::Renderer;
use crate::material::{
    AlphaMode, MaterialChange, MaterialEdit, MaterialSnapshot, MaterialState, TextureBinding,
};
use crate::texture::DecodedImage;
use crate::texture::{ChannelSelect, TextureSlot};

impl Renderer {
    /// Seed the editable material table from a freshly loaded model's import
    /// defaults (or clear it for an empty model). Bumps the material revision so
    /// the GPU table is rebuilt/re-uploaded on the next frame.
    pub fn set_model_materials(&mut self, materials: &[MaterialImportDefaults]) {
        self.material_states = materials
            .iter()
            .map(|material| MaterialState {
                base_color: material.base_color,
                metallic: material.metallic,
                // Roughness is the complement of the imported glossiness.
                roughness: (1.0 - material.smoothness).clamp(0.0, 1.0),
                emissive: material.emissive,
                ..MaterialState::default()
            })
            .collect();
        self.material_names = materials
            .iter()
            .map(|material| material.name.clone())
            .collect();
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Apply one UI material-edit intent to the editable table, bumping the
    /// revision so the renderer re-uploads. Out-of-range indices are ignored.
    pub fn set_material_param(&mut self, edit: MaterialEdit) {
        let Some(state) = self.material_states.get_mut(edit.index) else {
            return;
        };
        match edit.change {
            MaterialChange::BaseColor(rgb) => state.base_color = Vec3::from_array(rgb),
            MaterialChange::Metallic(value) => state.metallic = value.clamp(0.0, 1.0),
            MaterialChange::Roughness(value) => state.roughness = value.clamp(0.0, 1.0),
            MaterialChange::Emissive(rgb) => state.emissive = Vec3::from_array(rgb),
            MaterialChange::Channel(slot, channel) => {
                // Re-route an already-assigned slot; ignored if the slot is empty
                // or out of range.
                if let Some(Some(binding)) = state.textures.get_mut(slot) {
                    binding.channel = channel;
                }
            }
            MaterialChange::AlphaMode(mode) => state.alpha_mode = mode,
            MaterialChange::AlphaCutoff(value) => state.alpha_cutoff = value.clamp(0.0, 1.0),
            MaterialChange::Workflow(workflow) => state.workflow = workflow,
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// The editable material parameters, carried into the scene render each frame.
    pub fn material_states(&self) -> &[MaterialState] {
        &self.material_states
    }

    /// The current material revision (bumped on edit / load).
    pub fn material_revision(&self) -> u64 {
        self.material_revision
    }

    /// A name+value snapshot of the editable materials for the UI (invariant 2:
    /// the UI reads this plain value, never renderer-owned state).
    pub fn material_snapshot(&self) -> Vec<MaterialSnapshot> {
        self.material_names
            .iter()
            .cloned()
            .zip(self.material_states.iter().cloned())
            .map(|(name, state)| MaterialSnapshot { name, state })
            .collect()
    }

    /// Replace the entire editable material table with a captured set of states
    /// (the undo/redo restore path). Bumps the revision so the GPU table
    /// re-uploads on the next frame. The names are left untouched: the material
    /// count only changes on model load (which clears the undo history), so the
    /// restored states always line up with the current `material_names`.
    pub fn restore_materials(&mut self, states: Vec<MaterialState>) {
        self.material_states = states;
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Assign (or replace) a decoded image to one of a material's seven texture
    /// slots, with the chosen channel routing. The image is shared by `Arc` (the
    /// app decodes once and may reuse it across slots / materials). Bumps the
    /// revision so the GPU table uploads + rebinds on the next frame. Out-of-range
    /// material indices are ignored.
    pub fn set_texture_slot(
        &mut self,
        material: usize,
        slot: TextureSlot,
        path: PathBuf,
        image: Arc<DecodedImage>,
        channel: ChannelSelect,
    ) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = Some(TextureBinding {
            path,
            image,
            channel,
        });
        // Assigning an opacity map switches the material to alpha-blend so the
        // translucency shows; the Inspector can switch it to Clip.
        if slot == TextureSlot::Opacity && state.alpha_mode == AlphaMode::Opaque {
            state.alpha_mode = AlphaMode::Blend;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Clear a material's texture slot back to the shader's neutral fallback.
    pub fn clear_texture_slot(&mut self, material: usize, slot: TextureSlot) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = None;
        // Clearing the opacity map restores opaque compositing.
        if slot == TextureSlot::Opacity {
            state.alpha_mode = AlphaMode::Opaque;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Replace the decoded image of every texture binding that references `path`
    /// (across all materials / slots) with `image` — the disk-auto-reload path.
    /// Keeps each binding's channel routing. Returns `true` (and bumps the
    /// revision) when at least one binding matched.
    pub fn reload_texture(&mut self, path: &Path, image: Arc<DecodedImage>) -> bool {
        let mut changed = false;
        for state in &mut self.material_states {
            for binding in state.textures.iter_mut().flatten() {
                if binding.path == path {
                    binding.image = Arc::clone(&image);
                    changed = true;
                }
            }
        }
        if changed {
            self.material_revision = self.material_revision.wrapping_add(1);
        }
        changed
    }

    /// Every distinct source path currently bound to a material slot (for the disk
    /// watcher to register / reconcile).
    pub fn texture_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        for state in &self.material_states {
            for binding in state.textures.iter().flatten() {
                if !paths.contains(&binding.path) {
                    paths.push(binding.path.clone());
                }
            }
        }
        paths
    }
}
