//! The Inspector: for a selected material, a three-section layout —
//! **Material** (shader type, transparency mode, base color / roughness /
//! metallic / emissive), **Texture mapping** (one texture + channel dropdown per
//! PBR property, drawing from the scene texture pool), and **Texture files** (the
//! pooled images with thumbnails + remove) — each under a collapsible header. For
//! a selected node it shows read-only stats instead, and for a texture picked in
//! the Outliner's Textures tab (always, in the Tex workspace) that image's
//! measured properties and the materials that read it.
//!
//! Textures live in a **scene-wide pool** (`app`-owned, decoded once, shared by
//! `Arc`): the Inspector lists the pool and lets each property *reference* a
//! pooled image plus a channel, rather than each slot owning its own file. All
//! filesystem work (the import picker, decode, disk-watch) stays in `app`
//! (invariant 2); the Inspector only emits import / assign / clear / remove
//! intents and the live [`MaterialEdit`]s for the overlay to forward.

use std::path::Path;

use review_model::ModelData;
use review_render::{
    AlphaMode, ChannelSelect, MaterialChange, MaterialEdit, MaterialState, RoughnessWorkflow,
    Selection, TextureSlot,
};

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::{
    CommentIntent, TextureAssign, TextureIntent, TexturePoolEntry, TextureSlotRef, UiState,
    WorkspaceMode,
};
use crate::theme::size;
use crate::widgets::{
    Tip, labeled_color_button, labeled_combo, labeled_slider_with_value, panel_grid, tip,
};
use review_annotate::thread::{Anchor, Scope, Status};

/// What the Inspector emitted this frame, forwarded by the overlay into the
/// [`crate::state::UiOutput`].
#[derive(Debug, Clone, Default)]
pub(crate) struct InspectorOutput {
    pub material_edit: Option<MaterialEdit>,
    /// A texture-pool command emitted this frame (import / assign / clear /
    /// remove), forwarded by the overlay into [`crate::state::UiOutput`].
    pub texture: Option<TextureIntent>,
    /// Whether a material slider / color-picker is being actively dragged this
    /// frame, so `app` can coalesce a continuous drag into one undo step.
    pub material_edit_active: bool,
    /// "Go to comment" was clicked on the selected review comment.
    pub comment: Option<CommentIntent>,
}

pub(crate) fn body(ui: &mut egui::Ui, state: &mut UiState, model: &ModelData) -> InspectorOutput {
    if state.comment_inspected()
        && let Some(index) = state.comments.selected
    {
        return egui::ScrollArea::vertical()
            .show(ui, |ui| comment_inspector(ui, state, index))
            .inner;
    }
    let state: &UiState = state;
    if state.texture_inspected() {
        return egui::ScrollArea::vertical()
            .show(ui, |ui| texture_inspector(ui, state))
            .inner;
    }
    match state.selection {
        Selection::None => {
            ui.weak(keys::ui_inspector::EMPTY);
            InspectorOutput::default()
        }
        Selection::Material(index) => {
            egui::ScrollArea::vertical()
                .show(ui, |ui| material_inspector(ui, state, index))
                .inner
        }
        Selection::Node(index) => {
            // A multi-selection is a set, not a node, so it gets its own summary
            // rather than an arbitrary member's detail view. Bones and parts get
            // different ones: a bone set is described by what it *moves*, a part
            // set by what it *is*.
            if state.selected_bones.len() > 1 {
                bone_selection_inspector(ui, state, model);
            } else if state.selected_nodes.len() > 1 {
                node_selection_inspector(ui, state, model);
            } else {
                node_inspector(ui, state, model, index);
            }
            InspectorOutput::default()
        }
    }
}

/// The selected material's three collapsible sections.
fn material_inspector(ui: &mut egui::Ui, state: &UiState, index: usize) -> InspectorOutput {
    let Some(snapshot) = state.materials_snapshot.get(index) else {
        ui.weak(keys::ui_inspector::MATERIAL_GONE);
        return InspectorOutput::default();
    };

    let current = &snapshot.state;
    let mut out = InspectorOutput::default();

    let title = if snapshot.name.is_empty() {
        review_localization::tr(keys::ui_inspector::MATERIAL).into_owned()
    } else {
        snapshot.name.clone()
    };

    egui::CollapsingHeader::new(title)
        .id_salt(("inspector_material_section", index))
        .default_open(true)
        .show(ui, |ui| material_section(ui, current, index, &mut out));

    let mapping = egui::CollapsingHeader::new(keys::ui_inspector::TEXTURE_MAPPING)
        .id_salt(("inspector_texture_mapping", index))
        .default_open(true)
        .show(ui, |ui| {
            for slot in TextureSlot::ALL {
                texture_mapping_row(ui, current, &state.texture_pool, index, slot, &mut out);
            }
        });

    // A collapsing header's own header response is what carries the section's
    // explanation: the header is the only part of it always on screen.
    tip(
        mapping.header_response,
        Tip::new(keys::ui_inspector::TEXTURE_MAPPING)
            .describe(keys::ui_inspector::TEXTURE_MAPPING_DESCRIPTION)
            .page(Page::Materials),
    );

    let files = egui::CollapsingHeader::new(keys::ui_inspector::TEXTURE_FILES)
        .id_salt("inspector_texture_files")
        .default_open(true)
        .show(ui, |ui| {
            texture_files_section(ui, &state.texture_pool, &mut out)
        });
    tip(
        files.header_response,
        Tip::new(keys::ui_inspector::TEXTURE_FILES)
            .describe(keys::ui_inspector::TEXTURE_FILES_DESCRIPTION)
            .page(Page::Materials),
    );

    // A material slider handle / color-picker being dragged keeps egui's pointer
    // captured (the background 3D viewport isn't an egui widget, so camera orbit
    // never sets this). `app` coalesces the whole drag into one undo step.
    out.material_edit_active = ui.ctx().egui_is_using_pointer();

    out
}

/// Section 1 — the material's shader type (display-only), transparency mode, and
/// the editable scalar/color PBR parameters, on the striped two-column grid.
fn material_section(
    ui: &mut egui::Ui,
    state: &MaterialState,
    index: usize,
    out: &mut InspectorOutput,
) {
    let mut base = [state.base_color.x, state.base_color.y, state.base_color.z];
    let mut metallic = state.metallic;
    let mut roughness = state.roughness;
    let mut emissive = [state.emissive.x, state.emissive.y, state.emissive.z];
    let mut cutoff = state.alpha_cutoff;

    panel_grid(ui, "inspector_material", |ui| {
        // Workflow: how roughness is authored. Roughness is the native
        // metallic-roughness convention; Smoothness flips the slider + relabels the
        // map row and inverts a bound map in the shader (Unity-style).
        labeled_combo(
            ui,
            Tip::new(keys::ui_inspector::WORKFLOW)
                .describe(keys::ui_inspector::WORKFLOW_DESCRIPTION)
                .page(Page::Materials),
            "inspector_workflow",
            labels::workflow(state.workflow),
            |ui| {
                for workflow in RoughnessWorkflow::ALL {
                    if ui
                        .selectable_label(workflow == state.workflow, labels::workflow(workflow))
                        .clicked()
                        && workflow != state.workflow
                    {
                        out.material_edit = Some(MaterialEdit {
                            index,
                            change: MaterialChange::Workflow(workflow),
                        });
                    }
                }
            },
        );

        // Transparency mode = the material's alpha-compositing mode.
        labeled_combo(
            ui,
            Tip::new(keys::ui_inspector::TRANSPARENCY)
                .describe(keys::ui_inspector::TRANSPARENCY_DESCRIPTION)
                .page(Page::Materials),
            "inspector_alpha_mode",
            labels::alpha_mode(state.alpha_mode),
            |ui| {
                for mode in AlphaMode::ALL {
                    if ui
                        .selectable_label(mode == state.alpha_mode, labels::alpha_mode(mode))
                        .clicked()
                        && mode != state.alpha_mode
                    {
                        out.material_edit = Some(MaterialEdit {
                            index,
                            change: MaterialChange::AlphaMode(mode),
                        });
                    }
                }
            },
        );
        if state.alpha_mode == AlphaMode::Clip
            && labeled_slider_with_value(
                ui,
                Tip::new(keys::ui_inspector::CUTOFF)
                    .describe(keys::ui_inspector::CUTOFF_DESCRIPTION)
                    .page(Page::Materials),
                &mut cutoff,
                0.0..=1.0,
                3,
            )
        {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::AlphaCutoff(cutoff),
            });
        }

        if labeled_color_button(
            ui,
            Tip::new(keys::ui_inspector::BASE_COLOR).page(Page::Materials),
            &mut base,
        )
        .changed()
        {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::BaseColor(base),
            });
        }
        // The scalar always stores roughness; the Smoothness workflow displays its
        // complement and writes it back inverted (a smoothness slider).
        match state.workflow {
            RoughnessWorkflow::Roughness => {
                if labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_inspector::ROUGHNESS).page(Page::Materials),
                    &mut roughness,
                    0.0..=1.0,
                    3,
                ) {
                    out.material_edit = Some(MaterialEdit {
                        index,
                        change: MaterialChange::Roughness(roughness),
                    });
                }
            }
            RoughnessWorkflow::Smoothness => {
                let mut smoothness = 1.0 - roughness;
                if labeled_slider_with_value(
                    ui,
                    Tip::new(keys::ui_inspector::SMOOTHNESS).page(Page::Materials),
                    &mut smoothness,
                    0.0..=1.0,
                    3,
                ) {
                    out.material_edit = Some(MaterialEdit {
                        index,
                        change: MaterialChange::Roughness(1.0 - smoothness),
                    });
                }
            }
        }
        if labeled_slider_with_value(
            ui,
            Tip::new(keys::ui_inspector::METALLIC)
                .describe(keys::ui_inspector::METALLIC_DESCRIPTION)
                .page(Page::Materials),
            &mut metallic,
            0.0..=1.0,
            3,
        ) {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Metallic(metallic),
            });
        }
        if labeled_color_button(
            ui,
            Tip::new(keys::ui_inspector::EMISSIVE).page(Page::Materials),
            &mut emissive,
        )
        .changed()
        {
            out.material_edit = Some(MaterialEdit {
                index,
                change: MaterialChange::Emissive(emissive),
            });
        }
    });
}

/// Section 2 — one property row: the slot name and a texture dropdown listing the
/// pool ("select texture" = unbound), plus a channel dropdown. The packed **scalar**
/// slots (roughness / metallic / AO / opacity) route a single channel (R/G/B/A); the
/// **color** slots (base color / emissive) offer full RGB *or* a single channel scaled
/// by the color value. **Normal** always reads full RGB, so its channel dropdown is
/// shown disabled at "RGB" — present for row alignment, never editable. Under the
/// Smoothness workflow the Roughness row is relabelled "Smoothness".
fn texture_mapping_row(
    ui: &mut egui::Ui,
    state: &MaterialState,
    pool: &[TexturePoolEntry],
    index: usize,
    slot: TextureSlot,
    out: &mut InspectorOutput,
) {
    let slot_ref = TextureSlotRef {
        material: index,
        slot: slot.index(),
    };
    let binding = state.textures[slot.index()].as_ref();
    // The roughness slot follows the material's workflow: on a smoothness
    // material the shader inverts what the map holds, so labelling the row
    // "Roughness" would name the opposite of what it feeds.
    let slot_label = labels::texture_slot_for(slot, state.workflow);
    // Every slot carries a channel dropdown: scalar slots route a single channel,
    // color slots (base color / emissive) offer RGB or a single channel, and Normal
    // shows a disabled "RGB" combo (it always reads full RGB).
    let is_normal = slot == TextureSlot::Normal;

    ui.horizontal(|ui| {
        ui.scope(|ui| {
            ui.set_width(size::TEXTURE_MAP_LABEL_W);
            ui.add(egui::Label::new(slot_label).truncate());
        });

        let gap = ui.spacing().item_spacing.x;
        let channel_w = size::TEXTURE_CHANNEL_COMBO_W;
        let texture_w = (ui.available_width() - channel_w - gap).max(size::TEXTURE_COMBO_MIN_W);

        let selected_text = binding
            .map(|binding| pool_name(&binding.path))
            .unwrap_or_else(|| {
                review_localization::tr(keys::ui_inspector::SELECT_TEXTURE).into_owned()
            });
        egui::ComboBox::from_id_salt(("inspector_tex", index, slot.index()))
            .selected_text(selected_text)
            .width(texture_w)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(binding.is_none(), keys::ui_inspector::SELECT_TEXTURE)
                    .clicked()
                    && binding.is_some()
                {
                    out.texture = Some(TextureIntent::Clear(slot_ref));
                }
                for entry in pool {
                    let is_selected = binding.is_some_and(|binding| binding.path == entry.path);
                    if ui
                        .selectable_label(is_selected, pool_name(&entry.path))
                        .clicked()
                        && !is_selected
                    {
                        out.texture = Some(TextureIntent::Assign(TextureAssign {
                            slot: slot_ref,
                            path: entry.path.clone(),
                        }));
                    }
                }
            });

        // Scalar slots offer single channels; color slots add the full-RGB option.
        // Show the dropdown disabled (at the slot's neutral default) when unbound, and
        // always disabled at "RGB" for Normal (which has no channel choice).
        let choices: &[ChannelSelect] = if slot.is_scalar() {
            &ChannelSelect::SCALAR
        } else {
            &ChannelSelect::COLOR
        };
        let default_channel = if slot.is_scalar() {
            ChannelSelect::R
        } else {
            ChannelSelect::Rgb
        };
        let current_channel = if is_normal {
            ChannelSelect::Rgb
        } else {
            binding.map_or(default_channel, |binding| binding.channel)
        };
        ui.add_enabled_ui(binding.is_some() && !is_normal, |ui| {
            egui::ComboBox::from_id_salt(("inspector_ch", index, slot.index()))
                .selected_text(labels::channel(current_channel))
                .width(channel_w)
                .show_ui(ui, |ui| {
                    for &choice in choices {
                        if ui
                            .selectable_label(choice == current_channel, labels::channel(choice))
                            .clicked()
                            && choice != current_channel
                        {
                            out.material_edit = Some(MaterialEdit {
                                index,
                                change: MaterialChange::Channel(slot.index(), choice),
                            });
                        }
                    }
                });
        });
    });
}

/// Section 3 — the scene texture pool: each imported file as a thumbnail + name +
/// remove (✕), then an "Add textures" button and the drop-to-add hint.
fn texture_files_section(ui: &mut egui::Ui, pool: &[TexturePoolEntry], out: &mut InspectorOutput) {
    if pool.is_empty() {
        ui.weak(keys::ui_inspector::NO_TEXTURES);
    } else {
        for entry in pool {
            texture_file_row(ui, entry, out);
        }
    }

    add_textures_controls(ui, out);
}

/// One Texture files row: thumbnail (left), name (fills, truncating), remove
/// button (right).
fn texture_file_row(ui: &mut egui::Ui, entry: &TexturePoolEntry, out: &mut InspectorOutput) {
    ui.horizontal(|ui| {
        let side = size::TEXTURE_THUMB_SIZE;
        let thumb_size = egui::vec2(side, side);
        match crate::widgets::texture_thumbnail(ui, entry, side) {
            Some(texture) => {
                ui.add(egui::Image::from_texture(texture).fit_to_exact_size(thumb_size));
            }
            None => {
                // Decode hiccup: a neutral placeholder square keeps the row aligned.
                let (rect, _) = ui.allocate_exact_size(thumb_size, egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    size::SWATCH_CORNER_RADIUS,
                    crate::theme::color::GROUP_BG,
                );
            }
        }

        let gap = ui.spacing().item_spacing.x;
        let button_w = size::TEXTURE_REMOVE_BTN_W;
        let name_w = (ui.available_width() - button_w - gap).max(0.0);
        ui.scope(|ui| {
            ui.set_width(name_w);
            ui.add(egui::Label::new(pool_name(&entry.path)).truncate());
        });
        if ui
            .button("\u{2715}")
            .on_hover_text(keys::ui_inspector::REMOVE_TEXTURE)
            .clicked()
        {
            out.texture = Some(TextureIntent::Remove(entry.path.clone()));
        }
    });
}

/// The display name of a pooled texture (its file name).
fn pool_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| review_localization::tr(keys::ui_inspector::NO_TEXTURE).into_owned())
}

/// The selected review comment: its status, what it is on and points at, its
/// frames, every message of the thread, and — unless the file can't be written or
/// the thread came from a newer version — the ways to change it, edited in place
/// on [`UiState::comments`].
fn comment_inspector(ui: &mut egui::Ui, state: &mut UiState, index: usize) -> InspectorOutput {
    let mut out = InspectorOutput::default();
    let Some(entry) = state.comments.threads.get(index) else {
        return out;
    };
    let editable = state.comments.writable() && !entry.read_only;
    let thread = entry.thread.clone();
    let thread = &thread;
    ui.heading(keys::ui_comments::heading(index as f64 + 1.0));

    panel_grid(ui, "inspector_comment", |ui| {
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_comments::STATUS)
                .describe(keys::ui_comments::STATUS_DESCRIPTION)
                .page(Page::Comments),
            match thread.status {
                Status::Open => keys::ui_comments::STATUS_OPEN,
                Status::Resolved => keys::ui_comments::STATUS_RESOLVED,
            },
        );
        let on: egui::WidgetText = match thread.scope {
            Scope::File => keys::ui_comments::WHOLE_FILE.into(),
            Scope::Object => entry.object_name.clone().into(),
        };
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_comments::ON)
                .describe(keys::ui_comments::ON_DESCRIPTION)
                .page(Page::Comments),
            on,
        );
        let points_at = match thread.anchor.as_ref() {
            None => keys::ui_comments::ANCHOR_NONE,
            Some(anchor) => match anchor.known() {
                Some(Anchor::Surface { .. }) => keys::ui_comments::ANCHOR_SURFACE,
                Some(Anchor::World { .. }) => keys::ui_comments::ANCHOR_WORLD,
                Some(Anchor::Uv { .. }) => keys::ui_comments::ANCHOR_UV,
                None => keys::ui_comments::ANCHOR_UNKNOWN,
            },
        };
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_comments::POINTS_AT)
                .describe(keys::ui_comments::POINTS_AT_DESCRIPTION)
                .page(Page::Comments),
            points_at,
        );
        if let Some(frames) = &thread.frames {
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_comments::FRAMES_LABEL)
                    .describe(keys::ui_comments::FRAMES_LABEL_DESCRIPTION)
                    .page(Page::Comments),
                crate::panels::outliner::comments::frames_label(frames),
            );
        }
    });

    ui.add_space(ui.spacing().item_spacing.y);
    ui.horizontal_wrapped(|ui| {
        if thread.view.is_some() || thread.frames.is_some() || thread.anchor.is_some() {
            let go = ui.button(keys::ui_comments::GO_TO);
            if go.clicked() {
                out.comment = Some(CommentIntent::Show(index));
            }
            tip(
                go,
                Tip::new(keys::ui_comments::GO_TO)
                    .describe(keys::ui_comments::GO_TO_DESCRIPTION)
                    .page(Page::Comments),
            );
        }
        if !editable {
            return;
        }
        let (label, description, next) = match thread.status {
            Status::Open => (
                keys::ui_comments::RESOLVE,
                keys::ui_comments::RESOLVE_DESCRIPTION,
                Status::Resolved,
            ),
            Status::Resolved => (
                keys::ui_comments::REOPEN,
                keys::ui_comments::REOPEN_DESCRIPTION,
                Status::Open,
            ),
        };
        let status = ui.button(label);
        if status.clicked() {
            state.comments.set_status(index, next);
        }
        tip(
            status,
            Tip::new(label).describe(description).page(Page::Comments),
        );
        if matches!(state.mode, WorkspaceMode::ThreeD | WorkspaceMode::Uv) {
            let repin = ui.button(keys::ui_comments::REPIN);
            if repin.clicked() {
                state.comments.repin = Some(index);
                state.tool = crate::state::ViewportTool::Comment;
            }
            tip(
                repin,
                Tip::new(keys::ui_comments::REPIN)
                    .describe(keys::ui_comments::REPIN_DESCRIPTION)
                    .page(Page::Comments),
            );
        }
        let view = ui.button(keys::ui_comments::UPDATE_VIEW);
        if view.clicked()
            && let Some(now) = state.comments.view_now
        {
            state.comments.set_view(index, now);
        }
        tip(
            view,
            Tip::new(keys::ui_comments::UPDATE_VIEW)
                .describe(keys::ui_comments::UPDATE_VIEW_DESCRIPTION)
                .page(Page::Comments),
        );
    });
    if state.comments.repin == Some(index) {
        ui.weak(keys::ui_comments::REPIN_WAITING);
    }
    if entry_read_only(state, index) {
        ui.weak(keys::ui_comments::THREAD_READ_ONLY);
    }

    for (message_index, message) in thread.messages.iter().enumerate() {
        ui.add_space(size::COMMENT_MESSAGE_GAP);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&message.author).strong());
            if !message.time.is_empty() {
                ui.label(
                    egui::RichText::new(display_time(&message.time))
                        .size(crate::theme::font::COMMENT_META)
                        .color(crate::theme::color::TEXT_MUTED),
                );
            }
            if editable
                && state.comments.editing.is_none()
                && ui.small_button(keys::ui_comments::EDIT).clicked()
            {
                state.comments.editing = Some((index, message_index, message.text.clone()));
            }
        });
        let editing_this = matches!(
            &state.comments.editing,
            Some((thread_index, edited, _)) if *thread_index == index && *edited == message_index
        );
        if editing_this {
            let mut save = false;
            let mut cancel = false;
            if let Some((_, _, text)) = state.comments.editing.as_mut() {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(size::COMMENT_COMPOSER_ROWS)
                        .desired_width(f32::INFINITY),
                );
            }
            ui.horizontal(|ui| {
                save = ui.button(keys::ui_comments::SAVE_EDIT).clicked();
                cancel = ui.button(keys::ui_comments::COMPOSER_CANCEL).clicked();
            });
            if save && let Some((_, _, text)) = state.comments.editing.take() {
                state.comments.edit_message(index, message_index, text);
            } else if cancel {
                state.comments.editing = None;
            }
        } else {
            ui.add(egui::Label::new(&message.text).wrap());
        }
    }

    if editable {
        ui.add_space(size::COMMENT_MESSAGE_GAP);
        ui.separator();
        if state.comments.author.trim().is_empty() {
            ui.horizontal(|ui| {
                ui.label(keys::ui_comments::COMPOSER_NAME);
                ui.add(
                    egui::TextEdit::singleline(&mut state.comments.author)
                        .hint_text(keys::ui_comments::COMPOSER_NAME_HINT)
                        .desired_width(f32::INFINITY),
                );
            });
        }
        ui.add(
            egui::TextEdit::multiline(&mut state.comments.reply)
                .hint_text(keys::ui_comments::REPLY_HINT)
                .desired_rows(size::COMMENT_REPLY_ROWS)
                .desired_width(f32::INFINITY),
        );
        let ready =
            !state.comments.reply.trim().is_empty() && !state.comments.author.trim().is_empty();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new(keys::ui_comments::REPLY))
                .clicked()
            {
                state.comments.post_reply(index);
            }
        });

        ui.add_space(size::COMMENT_MESSAGE_GAP);
        let confirming = state.comments.confirm_delete == Some(index);
        let delete = ui.button(if confirming {
            keys::ui_comments::DELETE_CONFIRM
        } else {
            keys::ui_comments::DELETE
        });
        if delete.clicked() {
            if confirming {
                state.comments.delete(index);
            } else {
                state.comments.confirm_delete = Some(index);
            }
        }
    }
    out
}

/// Whether thread `index` came from a newer version and is shown read-only.
fn entry_read_only(state: &UiState, index: usize) -> bool {
    state
        .comments
        .threads
        .get(index)
        .is_some_and(|entry| entry.read_only)
}

/// A stored RFC 3339 UTC time (`2026-10-08T12:00:00Z`) as `2026-10-08 12:00 UTC`;
/// anything else as written.
fn display_time(time: &str) -> String {
    let bytes = time.as_bytes();
    let shaped = bytes.len() >= 17
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && time.ends_with('Z');
    if shaped {
        keys::ui_comments::time_utc(format!("{} {}", &time[..10], &time[11..16]))
    } else {
        time.to_owned()
    }
}

/// The current texture: a preview (outside Tex, whose viewport already is one),
/// its measured properties, the materials that read it, and the pool's remove /
/// add commands. With nothing in the pool it offers only the add.
fn texture_inspector(ui: &mut egui::Ui, state: &UiState) -> InspectorOutput {
    let mut out = InspectorOutput::default();
    let Some(entry) = state.texture_pool.get(state.texture_view.selected) else {
        ui.weak(if state.texture_pool.is_empty() {
            keys::ui_inspector::NO_TEXTURES
        } else {
            keys::ui_inspector::NO_TEXTURE_SELECTED
        });
        add_textures_controls(ui, &mut out);
        return out;
    };

    ui.heading(entry.name());

    if state.mode != WorkspaceMode::Texture {
        let edge = ui.available_width().min(size::TEXTURE_PREVIEW_MAX);
        if let Some(texture) = crate::widgets::texture_thumbnail(ui, entry, edge) {
            // Fit the longest edge to the preview size, keeping the aspect.
            let scale = edge / texture.size.x.max(texture.size.y).max(1.0);
            ui.add(egui::Image::from_texture(texture).fit_to_exact_size(texture.size * scale));
        }
    }

    panel_grid(ui, "inspector_texture", |ui| {
        for (row, value) in crate::stats::texture_stat_rows(entry) {
            crate::widgets::value_row(ui, row, value);
        }
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::TEXTURE_PATH)
                .describe(keys::ui_inspector::TEXTURE_PATH_DESCRIPTION)
                .page(Page::Tex),
            egui::RichText::new(entry.path.display().to_string()),
        );
    });

    // Every material with this image bound to one of its slots, by name.
    let users: Vec<&str> = state
        .materials_snapshot
        .iter()
        .filter(|material| {
            material
                .state
                .textures
                .iter()
                .flatten()
                .any(|binding| binding.path == entry.path)
        })
        .map(|material| material.name.as_str())
        .collect();
    let used_by = egui::CollapsingHeader::new(keys::ui_inspector::TEXTURE_USED_BY)
        .id_salt("inspector_texture_used_by")
        .default_open(true)
        .show(ui, |ui| {
            if users.is_empty() {
                ui.weak(keys::ui_inspector::TEXTURE_UNUSED);
            }
            for name in &users {
                ui.label(*name);
            }
        });
    tip(
        used_by.header_response,
        Tip::new(keys::ui_inspector::TEXTURE_USED_BY)
            .describe(keys::ui_inspector::TEXTURE_USED_BY_DESCRIPTION)
            .page(Page::Materials),
    );

    ui.add_space(ui.spacing().item_spacing.y);
    if ui.button(keys::ui_inspector::REMOVE_TEXTURE).clicked() {
        out.texture = Some(TextureIntent::Remove(entry.path.clone()));
    }
    add_textures_controls(ui, &mut out);
    out
}

/// The pool's "Add textures..." button and its drop hint, shared by the material
/// Inspector's Texture files section and the texture Inspector.
fn add_textures_controls(ui: &mut egui::Ui, out: &mut InspectorOutput) {
    ui.add_space(ui.spacing().item_spacing.y);
    if ui.button(keys::ui_inspector::ADD_TEXTURES).clicked() {
        out.texture = Some(TextureIntent::Import);
    }
    ui.weak(keys::ui_inspector::DROP_TEXTURES);
}

/// Read-only stats for a selected node: name, type, child count, triangle count,
/// and world position (display metadata only — invariant 1).
fn node_inspector(ui: &mut egui::Ui, state: &UiState, model: &ModelData, index: usize) {
    let Some(node) = model.nodes.get(index) else {
        ui.weak(keys::ui_inspector::NODE_GONE);
        return;
    };

    let name = if node.name.is_empty() {
        keys::ui_outliner::unnamed_node(index as f64)
    } else {
        node.name.clone()
    };
    ui.heading(keys::ui_inspector::node_heading(name));

    // "Mesh part" is more informative than the bare kind for a node that actually
    // carries geometry; every other node reports what the importer classified it as.
    let kind = if node.mesh_part.is_some() {
        keys::ui_inspector::MESH_PART
    } else {
        labels::node_kind(node.kind)
    };
    let child_count = model
        .nodes
        .iter()
        .filter(|child| child.parent == Some(index))
        .count();
    // Own triangles (this node's mesh), not the whole subtree — a quick audit
    // figure that matches what selecting just this node would isolate.
    let triangle_count = model
        .triangles
        .node
        .iter()
        .filter(|&&owner| owner as usize == index)
        .count();
    let position = node.transform.w_axis.truncate();

    panel_grid(ui, "inspector_node", |ui| {
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::TYPE).page(Page::OutlinerInspector),
            kind,
        );
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::CHILDREN).page(Page::OutlinerInspector),
            child_count.to_string(),
        );
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::TRIANGLES).page(Page::OutlinerInspector),
            triangle_count.to_string(),
        );
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::POSITION).page(Page::OutlinerInspector),
            format!("{:.3}, {:.3}, {:.3}", position.x, position.y, position.z),
        );

        // Skinned-mesh rows: the skinning method the file declared (the viewer
        // evaluates every skin as linear blend, and says so when the file asked
        // for something else) and its per-vertex influence cap, plus the
        // blend-shape channels this mesh carries.
        if let Some(deformer) = model.skin.as_ref().and_then(|skin| {
            skin.deformers
                .iter()
                .find(|deformer| deformer.mesh_node as usize == index)
        }) {
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_inspector::SKINNING)
                    .describe(keys::ui_inspector::SKINNING_DESCRIPTION)
                    .page(Page::OutlinerInspector),
                labels::skinning_method(deformer.method),
            );
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_inspector::MAX_INFLUENCES)
                    .describe(keys::ui_inspector::MAX_INFLUENCES_DESCRIPTION)
                    .page(Page::OutlinerInspector),
                deformer.max_weights_per_vertex.to_string(),
            );
        }
        let blend_channels = model.morph.as_ref().map_or(0, |morph| {
            morph
                .channels
                .iter()
                .filter(|channel| channel.mesh_node as usize == index)
                .count()
        });
        if blend_channels > 0 {
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_inspector::BLEND_SHAPES).page(Page::OutlinerInspector),
                blend_channels.to_string(),
            );
        }

        // Bone-only rows: what the rig authored, and how much of the mesh this
        // bone actually moves.
        if let Some(bone) = node.bone {
            // A file that declared neither would only add two zero rows of noise.
            if bone.radius > 0.0 {
                crate::widgets::value_row(
                    ui,
                    Tip::new(keys::ui_inspector::RADIUS).page(Page::OutlinerInspector),
                    format!("{:.3}", bone.radius),
                );
            }
            if bone.relative_length > 0.0 {
                crate::widgets::value_row(
                    ui,
                    Tip::new(keys::ui_inspector::RELATIVE_LENGTH).page(Page::OutlinerInspector),
                    format!("{:.3}", bone.relative_length),
                );
            }
            if model.skin.is_some() {
                // Measured once per selection change by `sync_bone_influence`.
                crate::widgets::value_row(
                    ui,
                    Tip::new(keys::ui_inspector::INFLUENCED_VERTS)
                        .describe(keys::ui_inspector::INFLUENCED_VERTS_DESCRIPTION)
                        .page(Page::OutlinerInspector),
                    state.caches.bone_influence.to_string(),
                );
                if model.stats.vertex_count > 0 {
                    let share = state.caches.bone_influence as f32
                        / model.stats.vertex_count as f32
                        * 100.0;
                    crate::widgets::value_row(
                        ui,
                        Tip::new(keys::ui_inspector::SHARE_OF_MESH)
                            .describe(keys::ui_inspector::SHARE_OF_MESH_DESCRIPTION)
                            .page(Page::OutlinerInspector),
                        format!("{share:.1}%"),
                    );
                }
            }
        }
    });
}

/// The summary shown when several parts are selected at once: how many, what
/// they add up to, and which ones they are.
///
/// The counts come from the stats card's own scoped sums, already measured this
/// frame against exactly this selection — so the panel states the same figures
/// the card does rather than walking the mesh a second time (invariant 6). They
/// are absent only until the model's measured table lands, which is the same
/// moment the card's own columns fill in.
fn node_selection_inspector(ui: &mut egui::Ui, state: &UiState, model: &ModelData) {
    let count = state.selected_nodes.len();
    ui.heading(keys::ui_inspector::parts_selected(count as f64));

    panel_grid(ui, "inspector_node_selection", |ui| {
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::PARTS).page(Page::Selection),
            count.to_string(),
        );
        if let Some(scoped) = state.caches.scoped_stats.and_then(|stats| stats.selected) {
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_inspector::SELECTED_TRIS)
                    .describe(keys::ui_inspector::SELECTED_TRIS_DESCRIPTION)
                    .page(Page::Selection),
                scoped.triangle_count.to_string(),
            );
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Click order, not sorted order: the list should read the way the user
        // built it, with the primary (last-clicked) selection at the bottom.
        for &node in &state.selected_nodes {
            ui.label(node_label(model, node));
        }
    });
}

/// A node's name for a selection list, or a positional fallback for the
/// unnamed ones a rebuilt mesh can carry.
fn node_label(model: &ModelData, node: usize) -> String {
    match model.nodes.get(node) {
        Some(scene_node) if !scene_node.name.is_empty() => scene_node.name.clone(),
        _ => keys::ui_outliner::unnamed_node(node as f64),
    }
}

/// The summary shown when several bones are selected at once: what is selected,
/// how much of the mesh it moves, and which bones they are.
fn bone_selection_inspector(ui: &mut egui::Ui, state: &UiState, model: &ModelData) {
    let count = state.selected_bones.len();
    ui.heading(keys::ui_inspector::bones_selected(count as f64));

    // The union count, measured once per selection change by `sync_bone_influence`.
    let influenced = state.caches.bone_influence;
    panel_grid(ui, "inspector_bone_selection", |ui| {
        crate::widgets::value_row(
            ui,
            Tip::new(keys::ui_inspector::BONES).page(Page::OutlinerInspector),
            count.to_string(),
        );
        if model.skin.is_some() {
            crate::widgets::value_row(
                ui,
                Tip::new(keys::ui_inspector::INFLUENCED_VERTS)
                    .describe(keys::ui_inspector::INFLUENCED_VERTS_DESCRIPTION)
                    .page(Page::OutlinerInspector),
                influenced.to_string(),
            );
            if model.stats.vertex_count > 0 {
                let share = influenced as f32 / model.stats.vertex_count as f32 * 100.0;
                crate::widgets::value_row(
                    ui,
                    Tip::new(keys::ui_inspector::SHARE_OF_MESH)
                        .describe(keys::ui_inspector::SHARE_OF_MESH_DESCRIPTION)
                        .page(Page::OutlinerInspector),
                    format!("{share:.1}%"),
                );
            }
        }
    });

    ui.add_space(size::PANEL_ROW_GAP);
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Click order, not sorted order: the list should read the way the user
        // built it, with the primary (last-clicked) selection at the bottom.
        for &node in &state.selected_bones {
            ui.label(node_label(model, node));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::display_time;

    #[test]
    fn a_stored_time_reads_as_minutes_in_utc() {
        assert_eq!(display_time("2026-10-08T12:34:56Z"), "2026-10-08 12:34 UTC");
        assert_eq!(display_time("yesterday"), "yesterday");
    }
}
