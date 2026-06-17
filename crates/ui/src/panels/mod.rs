//! Tool option-panels. Each tool's controls live in its own submodule; this
//! module owns the shared panel chrome ([`option_panel`]) and the dispatch that
//! routes the active [`OptionPanel`] to the right tool.

mod anti_aliasing;
mod bloom;
mod bounding_box;
mod environment;
mod normals;
mod uv_checker;
mod vertex_colors;
mod wireframe;

use crate::assets::{self, ICON_CLOSE};
use crate::state::{OptionPanel, UiState};
use crate::theme::{color, font, size};

/// Result of drawing an [`option_panel`]: how far its header was dragged this
/// frame, plus the one-shot header interactions (collapse toggle / close).
pub(crate) struct PanelOutcome {
    pub(crate) drag_delta: egui::Vec2,
    pub(crate) toggle_collapse: bool,
    pub(crate) close: bool,
}

/// Draw the currently-active tool panel. Returns the header interactions so the
/// caller (overlay) can apply collapse/close/drag against [`UiState`].
pub(crate) fn draw_active_panel(
    ui: &mut egui::Ui,
    state: &mut UiState,
    panel: OptionPanel,
) -> PanelOutcome {
    let collapsed = state.panel_collapsed;
    match panel {
        OptionPanel::Wireframe => {
            option_panel(ui, "Wireframe", collapsed, |ui| wireframe::body(ui, state))
        }
        OptionPanel::BoundingBox => option_panel(ui, "Bounding Box", collapsed, |ui| {
            bounding_box::body(ui, state)
        }),
        OptionPanel::UvChecker => option_panel(ui, "UV Checker", collapsed, |ui| {
            uv_checker::body(ui, state)
        }),
        OptionPanel::FaceNormals => option_panel(ui, "Face Normals", collapsed, |ui| {
            normals::face_body(ui, state)
        }),
        OptionPanel::VertexNormals => option_panel(ui, "Vertex Normals", collapsed, |ui| {
            normals::vertex_body(ui, state)
        }),
        OptionPanel::VertexColors => option_panel(ui, "Vertex Colors", collapsed, |ui| {
            vertex_colors::body(ui, state)
        }),
        OptionPanel::AntiAliasing => option_panel(ui, "Anti Aliasing", collapsed, |ui| {
            anti_aliasing::body(ui, state)
        }),
        OptionPanel::Environment => option_panel(ui, "Environment", collapsed, |ui| {
            environment::body(ui, state)
        }),
        OptionPanel::Bloom => option_panel(ui, "Bloom", collapsed, |ui| bloom::body(ui, state)),
    }
}

/// The shared option-panel shell: a draggable header bar (single click toggles
/// collapse, drag moves, the X closes) and a controls body shown only when
/// expanded. Header and body share fill/stroke so they read as one card.
fn option_panel(
    ui: &mut egui::Ui,
    title: &str,
    collapsed: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> PanelOutcome {
    let mut outcome = PanelOutcome {
        drag_delta: egui::Vec2::ZERO,
        toggle_collapse: false,
        close: false,
    };

    let panel_fill = color::PANEL_CARD_BG;
    let panel_stroke = egui::Stroke::new(size::HAIRLINE, color::PANEL_BORDER);

    // The panel is two stacked elements sitting flush (no inter-frame spacing)
    // so their touching borders read as a single full-width divider.
    ui.spacing_mut().item_spacing.y = 0.0;

    // ── Header bar ──────────────────────────────────────────────────────────
    // Rounds all four corners when collapsed (a standalone pill); only the top
    // corners when expanded so it meets the body squarely.
    let header_corner = if collapsed {
        egui::CornerRadius::same(size::PANEL_CORNER_RADIUS)
    } else {
        egui::CornerRadius {
            nw: size::PANEL_CORNER_RADIUS,
            ne: size::PANEL_CORNER_RADIUS,
            sw: 0,
            se: 0,
        }
    };
    egui::Frame::NONE
        .fill(panel_fill)
        .stroke(panel_stroke)
        .corner_radius(header_corner)
        .show(ui, |ui| {
            // Allocate the full-width bar so the frame actually paints its
            // background. The bar is the interaction handle: a single click
            // toggles collapse, a drag moves.
            let (header_rect, header_response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), size::PANEL_HEADER_HEIGHT),
                egui::Sense::click_and_drag(),
            );
            if header_response.dragged() {
                outcome.drag_delta = header_response.drag_delta();
            }
            if header_response.clicked() {
                outcome.toggle_collapse = true;
            }
            if header_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }

            // Title: left-aligned and vertically centered in the compact bar.
            ui.painter().text(
                egui::pos2(
                    header_rect.left() + size::PANEL_HEADER_PAD_X,
                    header_rect.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(font::PANEL_TITLE),
                color::TEXT_TITLE,
            );

            // Close button, right-aligned in the bar. Its own Sense::click()
            // wins over the header's, so clicking the X closes rather than
            // toggling collapse.
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(
                    header_rect.right() - size::PANEL_HEADER_HEIGHT,
                    header_rect.top(),
                ),
                egui::vec2(size::PANEL_HEADER_HEIGHT, size::PANEL_HEADER_HEIGHT),
            );
            let close_response = ui.interact(
                close_rect,
                ui.make_persistent_id(("option_panel_close", title)),
                egui::Sense::click(),
            );
            if close_response.hovered() {
                ui.painter().rect_filled(
                    close_rect.shrink(size::CLOSE_HOVER_INSET),
                    size::CLOSE_HOVER_RADIUS,
                    color::CLOSE_HOVER_BG,
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some(texture) = assets::load_icon_texture(ui, &ICON_CLOSE) {
                let icon_rect = egui::Rect::from_center_size(
                    close_rect.center(),
                    egui::vec2(size::PANEL_CLOSE_ICON_SIZE, size::PANEL_CLOSE_ICON_SIZE),
                );
                let tint = if close_response.hovered() {
                    color::ICON_CLOSE_HOVERED
                } else {
                    color::ICON_CLOSE_IDLE
                };
                ui.painter().image(
                    texture.id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }
            if close_response.clicked() {
                outcome.close = true;
            }
        });

    // ── Controls body ──────────────────────────────────────────────────────
    // Hidden entirely when collapsed. The divider between the two is their
    // shared (full-width) border.
    if !collapsed {
        egui::Frame::NONE
            .fill(panel_fill)
            .stroke(panel_stroke)
            .corner_radius(egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: size::PANEL_CORNER_RADIUS,
                se: size::PANEL_CORNER_RADIUS,
            })
            .inner_margin(egui::Margin {
                left: size::PANEL_CONTENT_MARGIN,
                right: size::PANEL_CONTENT_MARGIN,
                top: size::PANEL_BODY_TOP_MARGIN,
                bottom: size::PANEL_CONTENT_MARGIN,
            })
            .show(ui, |ui| {
                // Rows space themselves via add_space; kill the implicit gap.
                ui.spacing_mut().item_spacing.y = 0.0;
                add_contents(ui);
            });
    }

    outcome
}
