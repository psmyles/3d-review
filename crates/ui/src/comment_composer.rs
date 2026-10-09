//! The comment composer: a popover beside the spot a new comment points at,
//! where it is written, given its frame and its view, and posted.
//!
//! It is egui's own `Area` with the popup frame, anchored to the draft's pin —
//! projected through the live camera every frame, so it stays by the spot while
//! the view turns — or, for a comment with no point of its own, to where it was
//! started. The first comment also asks for a name to sign with.

use crate::comment_pins::PinView;
use crate::docs::Page;
use crate::keys;
use crate::state::{Draft, DraftAnchor, FrameError, NotWritable, UiState};
use crate::theme::{color, size};
use crate::widgets::{Tip, tip};

/// Draw the draft's pin and its composer, if a comment is being written.
/// `view` projects a surface draft's pin (absent outside the 3D view, where no
/// draft has a point); `viewport` keeps the popover inside the free area.
pub(crate) fn draw_composer(
    ctx: &egui::Context,
    state: &mut UiState,
    view: Option<&PinView<'_>>,
    viewport: egui::Rect,
) {
    let Some(draft) = state.comments.draft.as_ref() else {
        return;
    };

    // Where the popover opens: beside the draft's pin when it has one on screen,
    // else where the draft was started.
    let pin = draft
        .anchor
        .point()
        .zip(view)
        .and_then(|(point, view)| view.project(point));
    if let Some(center) = pin {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("comment_draft_pin"),
        ));
        painter.circle(
            center,
            size::COMMENT_PIN_RADIUS,
            color::COMMENT_PIN_DRAFT,
            egui::Stroke::new(size::COMMENT_PIN_STROKE, color::COMMENT_PIN_OUTLINE),
        );
    }
    let anchor = pin.unwrap_or(draft.screen)
        + egui::vec2(size::COMMENT_COMPOSER_OFFSET, size::COMMENT_COMPOSER_OFFSET);

    let mut post = false;
    let mut cancel = false;
    egui::Area::new(egui::Id::new("comment_composer"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor)
        .constrain_to(viewport)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(size::COMMENT_COMPOSER_WIDTH);
                (post, cancel) = composer_body(ui, state);
            });
        });

    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if cancel {
        state.comments.draft = None;
    } else if post {
        state.post_comment();
    }
}

/// The popover's contents. Returns (post, cancel).
fn composer_body(ui: &mut egui::Ui, state: &mut UiState) -> (bool, bool) {
    let writable = state.comments.not_writable;
    let comments = &mut state.comments;
    let Some(draft) = comments.draft.as_mut() else {
        return (false, true);
    };

    // What it is about.
    let about: egui::WidgetText = match &draft.anchor {
        DraftAnchor::Surface { node, .. } | DraftAnchor::Object { node } => {
            let name = comments
                .node_objects
                .get(*node)
                .cloned()
                .flatten()
                .map(|object| object.name)
                .unwrap_or_default();
            keys::ui_comments::composer_on(name).into()
        }
        DraftAnchor::Uv {
            node: Some(node), ..
        } => {
            let name = comments
                .node_objects
                .get(*node)
                .cloned()
                .flatten()
                .map(|object| object.name)
                .unwrap_or_default();
            keys::ui_comments::composer_on(name).into()
        }
        DraftAnchor::Uv { node: None, .. } => keys::ui_comments::COMPOSER_ABOUT_UV.into(),
        DraftAnchor::View => keys::ui_comments::COMPOSER_ABOUT_VIEW.into(),
        DraftAnchor::File => keys::ui_comments::COMPOSER_ABOUT_FILE.into(),
    };
    ui.label(egui::RichText::new(about.text()).strong());

    if let Some(reason) = writable {
        ui.weak(match reason {
            NotWritable::NoFile => keys::ui_comments::NOT_WRITABLE_NO_FILE,
            NotWritable::Unreadable => keys::ui_comments::NOT_WRITABLE_UNREADABLE,
        });
        let cancel = ui.button(keys::ui_comments::COMPOSER_CANCEL).clicked();
        return (false, cancel);
    }

    // The name, asked for once.
    if comments.author.trim().is_empty() {
        ui.horizontal(|ui| {
            let label = ui.label(keys::ui_comments::COMPOSER_NAME);
            tip(
                label,
                Tip::new(keys::ui_comments::COMPOSER_NAME)
                    .describe(keys::ui_comments::COMPOSER_NAME_DESCRIPTION)
                    .page(Page::Comments),
            );
            ui.add(
                egui::TextEdit::singleline(&mut comments.author)
                    .hint_text(keys::ui_comments::COMPOSER_NAME_HINT)
                    .desired_width(f32::INFINITY),
            );
        });
    }

    let text = ui.add(
        egui::TextEdit::multiline(&mut draft.text)
            .hint_text(keys::ui_comments::COMPOSER_HINT)
            .desired_rows(size::COMMENT_COMPOSER_ROWS)
            .desired_width(f32::INFINITY),
    );
    if !draft.focused && !comments.author.trim().is_empty() {
        text.request_focus();
        draft.focused = true;
    }

    // The frames, the view, and how a surface pin behaves.
    frames_row(ui, draft);
    let view = ui.checkbox(
        &mut draft.include_view,
        keys::ui_comments::COMPOSER_SAVE_VIEW,
    );
    tip(
        view,
        Tip::new(keys::ui_comments::COMPOSER_SAVE_VIEW)
            .describe(keys::ui_comments::COMPOSER_SAVE_VIEW_DESCRIPTION)
            .page(Page::Comments),
    );
    if matches!(draft.anchor, DraftAnchor::Surface { .. }) {
        ui.horizontal(|ui| {
            let follow = ui.radio_value(
                &mut draft.follow_surface,
                true,
                keys::ui_comments::COMPOSER_FOLLOW_SURFACE,
            );
            tip(
                follow,
                Tip::new(keys::ui_comments::COMPOSER_FOLLOW_SURFACE)
                    .describe(keys::ui_comments::COMPOSER_FOLLOW_SURFACE_DESCRIPTION)
                    .page(Page::Comments),
            );
            let fixed = ui.radio_value(
                &mut draft.follow_surface,
                false,
                keys::ui_comments::COMPOSER_FIXED_POINT,
            );
            tip(
                fixed,
                Tip::new(keys::ui_comments::COMPOSER_FIXED_POINT)
                    .describe(keys::ui_comments::COMPOSER_FIXED_POINT_DESCRIPTION)
                    .page(Page::Comments),
            );
        });
    }

    let ready = !draft.text.trim().is_empty()
        && !comments.author.trim().is_empty()
        && draft.frames().is_ok();
    let mut post = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        let post_button =
            ui.add_enabled(ready, egui::Button::new(keys::ui_comments::COMPOSER_POST));
        post = post_button.clicked();
        tip(
            post_button,
            Tip::new(keys::ui_comments::COMPOSER_POST).describe(
                keys::ui_comments::composer_post_description(
                    crate::primary_modifier().into_owned(),
                ),
            ),
        );
        cancel = ui.button(keys::ui_comments::COMPOSER_CANCEL).clicked();
    });
    let chord = ui.input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.command);
    (post || (ready && chord), cancel)
}

/// The clip the comment is about and its first and last frame, typed. While the
/// two fields don't name a range of the clip's frames, the wrong one is outlined,
/// a line says why, and the comment can't be posted.
fn frames_row(ui: &mut egui::Ui, draft: &mut Draft) {
    let Some(clip) = draft.frames.as_ref().map(|fields| fields.clip.clone()) else {
        return;
    };
    let label = keys::ui_comments::composer_frames(clip);
    let include = ui.checkbox(&mut draft.include_frames, label.clone());
    tip(
        include,
        Tip::new(label)
            .describe(keys::ui_comments::COMPOSER_FRAMES_DESCRIPTION)
            .page(Page::Comments),
    );

    let enabled = draft.include_frames;
    let error = draft.frames().err();
    let (start_wrong, end_wrong) = match error {
        Some(FrameError::OutsideClip { start, end }) => (start, end),
        Some(FrameError::Backwards) => (true, true),
        None => (false, false),
    };
    let Some(fields) = draft.frames.as_mut() else {
        return;
    };
    let last = fields.last;
    ui.indent("comment_composer_frames", |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let changed = ui
                .horizontal(|ui| {
                    let start = frame_field(ui, &mut fields.start, start_wrong);
                    ui.label(keys::ui_comments::COMPOSER_TO_FRAME);
                    let end = frame_field(ui, &mut fields.end, end_wrong);
                    start.changed() || end.changed()
                })
                .inner;
            // The outline and the line below were decided before this frame's
            // typing reached the fields; one more pass shows them for what is
            // in the fields now.
            if changed {
                ui.ctx().request_repaint();
            }
            let reason = match error {
                Some(FrameError::OutsideClip { .. }) => {
                    keys::ui_comments::composer_frames_outside(f64::from(last))
                }
                Some(FrameError::Backwards) => {
                    review_localization::tr(keys::ui_comments::COMPOSER_FRAMES_BACKWARDS)
                        .into_owned()
                }
                None => return,
            };
            ui.label(egui::RichText::new(reason).color(ui.visuals().error_fg_color));
        });
    });
}

/// One frame-number field: monospace and a fixed width, as the numeric boxes
/// elsewhere are, and outlined in the error colour while `wrong`.
fn frame_field(ui: &mut egui::Ui, text: &mut String, wrong: bool) -> egui::Response {
    ui.scope(|ui| {
        if wrong {
            let stroke = egui::Stroke::new(
                size::COMMENT_FRAME_FIELD_ERROR_STROKE,
                ui.visuals().error_fg_color,
            );
            let visuals = ui.visuals_mut();
            visuals.widgets.inactive.bg_stroke = stroke;
            visuals.widgets.hovered.bg_stroke = stroke;
            visuals.widgets.active.bg_stroke = stroke;
            visuals.selection.stroke = stroke;
        }
        ui.add(
            egui::TextEdit::singleline(text)
                .font(egui::TextStyle::Monospace)
                .desired_width(size::COMMENT_FRAME_FIELD_WIDTH),
        )
    })
    .inner
}
