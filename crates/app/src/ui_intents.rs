//! Application of the UI's emitted intents — the UI→app command direction of
//! invariant 2.
//!
//! `ui` never owns or mutates renderer or model internals: it emits a `UiOutput`
//! of plain values and *intents*, and `app` is the only coordinator that applies
//! them to the `Renderer`. That application is this file: `apply_ui_output`, run
//! once per frame by `frame.rs` right after the egui pass, plus the
//! `refresh_materials` snapshot that carries the renderer's answer back the other
//! way (app→UI data).

use review_render::TextureSlot;
use review_ui::{AxisGizmoAction, MenuIntent, TextureIntent, UiOutput};

use crate::App;

impl App {
    pub(crate) fn apply_ui_output(&mut self, output: UiOutput) {
        // Mirror this frame's drag state for the undo observer (read at the top of
        // the next frame), so a continuous slider / color drag — a material
        // parameter or an Opt one — coalesces into a single undo step. Tracked
        // even when the renderer isn't ready yet.
        self.drag_in_progress = output.material_edit_active || output.opt_edit_active;

        // Opt's file-dialog actions don't need the renderer, and must run even
        // before it exists.
        if let Some(intent) = output.opt {
            self.apply_opt_intent(intent);
        }

        // The menu's commands land on the handlers their shortcuts already reach
        // (`shortcuts.rs`), so the menu is a second door onto each command rather
        // than a second implementation of it.
        if let Some(intent) = output.menu {
            self.apply_menu_intent(intent);
        }

        if self.renderer.is_none() {
            return;
        }

        let mut redraw = false;

        // Camera + scalar material edits need the renderer borrow; scope it so the
        // texture intents below can call `&mut self` helpers.
        {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            if let Some(action) = output.axis_gizmo_action {
                match action {
                    AxisGizmoAction::Orbit(delta) => renderer.orbit_camera(delta),
                    AxisGizmoAction::Snap(axis) => {
                        renderer.animate_camera_to_offset_direction(axis.offset_direction());
                    }
                    AxisGizmoAction::ResetView => renderer.animate_camera_to_home(),
                }
                redraw = true;
            }
            if let Some(edit) = output.material_edit {
                renderer.set_material_param(edit);
            }
        }

        // Refresh the UI snapshot + revision so the editor reflects the new value
        // and the scene callback re-uploads the table.
        if output.material_edit.is_some() {
            self.refresh_materials();
            redraw = true;
        }

        // One texture-pool command per frame (the Inspector emits at most one).
        if let Some(intent) = output.texture {
            match intent {
                // Clear a property's slot back to its fallback ("select texture").
                TextureIntent::Clear(slot_ref) => {
                    if let Some(slot) = TextureSlot::from_index(slot_ref.slot) {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.clear_texture_slot(slot_ref.material, slot);
                        }
                        self.refresh_materials();
                        redraw = true;
                    }
                }
                // Bind an already-decoded pooled texture to a property (applies now).
                TextureIntent::Assign(assign) => {
                    self.assign_pooled_texture(assign.slot, assign.path);
                    redraw = true;
                }
                // Remove a pooled texture (the ✕) — also unbinds every slot using it.
                TextureIntent::Remove(path) => {
                    self.remove_texture(&path);
                    redraw = true;
                }
                // Import textures into the pool ("Add textures…"). The picker
                // runs on a worker (`dialog.rs`) and nothing has changed yet, so
                // this schedules no redraw; the answer's own handler does.
                TextureIntent::Import => self.import_textures(),
            }
        }

        if redraw {
            self.redraw.requested = true;
        }
    }

    fn apply_menu_intent(&mut self, intent: MenuIntent) {
        match intent {
            MenuIntent::OpenFile => self.open_model_from_dialog(),
            MenuIntent::CloseFile => self.reset_to_start_state(),
            MenuIntent::ToggleRememberSettings => {
                self.ui.remember_settings = !self.ui.remember_settings;
                // Written now rather than only on exit, so the choice holds even
                // for a session that never exits cleanly.
                self.persist_settings();
                self.redraw.requested = true;
            }
            MenuIntent::CheckForUpdates => self.check_for_updates(),
            MenuIntent::Exit => self.exit_requested = true,
        }
    }

    /// Pull the renderer's editable-material snapshot + revision back into the UI
    /// (after any material edit / texture assignment / reload).
    pub(crate) fn refresh_materials(&mut self) {
        if let Some(renderer) = self.renderer.as_ref() {
            self.ui.materials_snapshot = renderer.material_snapshot();
            self.ui.material_revision = renderer.material_revision();
        }
    }
}
