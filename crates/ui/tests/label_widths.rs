//! Every label has to fit the column it is drawn in.
//!
//! The chrome pins its widths: an option panel's label column, the workspace
//! segments, the stats card. That is a deliberate choice — it is what keeps every
//! panel the same shape — but it means a longer word does not push the layout
//! out, it gets **truncated**, and a truncated label says less than the one it
//! replaced.
//!
//! In English that is not a live risk; the widths were chosen around these very
//! words. It becomes one the moment a second locale exists, because German and
//! Finnish routinely run half again as long as English. This is the test that
//! reports it as a failure rather than as a screenshot someone notices later.

use review_localization::catalog;
use review_ui::theme::{font, size};

/// Lay text out the way the chrome will and report its width in points.
fn measure(ctx: &egui::Context, text: &str, font_id: egui::FontId) -> f32 {
    let galley =
        ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font_id, egui::Color32::WHITE));
    galley.rect.width()
}

/// A context with the app's real fonts and style installed — the measurement is
/// only as good as the face it is taken in, and the bundled Inter is wider than
/// egui's default.
fn styled_context() -> egui::Context {
    let ctx = egui::Context::default();
    review_ui::init_style(&ctx);
    // One pass, so the font atlas exists before anything is measured.
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        },
        |_| {},
    );
    output.textures_delta.clear();
    ctx
}

/// Messages from one catalog file, as they will be drawn.
///
/// Messages that take variables are left out: their width depends on the value
/// substituted, so measuring one against a made-up argument would pin the test
/// to the argument rather than to the message.
fn messages_from(file: &str) -> Vec<(&'static str, String)> {
    catalog::MESSAGES
        .iter()
        .filter(|message| message.file == file && message.attr.is_none() && message.vars.is_empty())
        .map(|message| {
            let key = review_localization::Key::new(message.id, None);
            (message.id, review_localization::tr(key).into_owned())
        })
        .collect()
}

/// An option panel's label column is a fixed width and truncates rather than
/// widening, so every row label has to fit inside it.
#[test]
fn every_panel_row_label_fits_its_column() {
    let ctx = styled_context();
    let body = egui::FontId::proportional(font::PANEL_BODY);
    let mut overflowing = Vec::new();

    for (id, text) in messages_from("ui-panels") {
        let width = measure(&ctx, &text, body.clone());
        if width > size::PANEL_LABEL_COL_WIDTH {
            overflowing.push(format!(
                "{id}: {width:.0}pt > {:.0}pt — {text:?}",
                size::PANEL_LABEL_COL_WIDTH
            ));
        }
    }

    assert!(
        overflowing.is_empty(),
        "these panel row labels are wider than the label column and will be \
         truncated on screen:\n{}",
        overflowing.join("\n")
    );
}

/// The four workspace segments share one fixed group, so each has a quarter of it.
#[test]
fn every_workspace_segment_fits_its_tile() {
    let ctx = styled_context();
    let segment = egui::FontId::proportional(font::MODE_SEGMENT);
    // The tile's own width less a little padding either side.
    let available = size::MODE_SEGMENT_WIDTH - 6.0;
    let mut overflowing = Vec::new();

    for (id, text) in messages_from("ui-enums") {
        if !id.starts_with("ui-enums-workspace-") {
            continue;
        }
        let width = measure(&ctx, &text, segment.clone());
        if width > available {
            overflowing.push(format!("{id}: {width:.0}pt > {available:.0}pt — {text:?}"));
        }
    }

    assert!(
        overflowing.is_empty(),
        "these workspace segment labels overflow their tile:\n{}",
        overflowing.join("\n")
    );
}

/// The stats card's row names share the card with three fixed value columns.
#[test]
fn every_stats_row_label_fits_the_card() {
    let ctx = styled_context();
    let stats = egui::FontId::monospace(font::STATS);
    let available = size::STATS_PANEL_WIDTH - 3.0 * size::STATS_VALUE_COLUMN;
    let mut overflowing = Vec::new();

    for (id, text) in messages_from("ui-stats") {
        // Not row names: the column headings, the Opt card's own heading (which
        // spans the whole card rather than the name column), the "not measured"
        // dash, and the channel-layout words, which are values.
        let not_a_row_name = id.contains("-scope-")
            || id.ends_with("-unmeasured")
            || id.contains("-channels-")
            || id.ends_with("-source-nothing-applied")
            || id.ends_with("-processed");
        if not_a_row_name {
            continue;
        }
        let width = measure(&ctx, &text, stats.clone());
        if width > available {
            overflowing.push(format!("{id}: {width:.0}pt > {available:.0}pt — {text:?}"));
        }
    }

    assert!(
        overflowing.is_empty(),
        "these stats row labels leave no room for their value columns:\n{}",
        overflowing.join("\n")
    );
}

/// At the side panel's default width, the 3D workspace's tabs for a file
/// without animations — Scene, Materials, Textures, Comments — all fit in the
/// strip with their padding, so nobody has to open the overflow menu for a tab
/// they reach for every session. (With an Animations tab added the strip moves
/// the trailing tab into its overflow menu, which is what that menu is for.)
#[test]
fn the_everyday_outliner_tabs_fit_the_default_panel() {
    let ctx = styled_context();
    let tab_font = egui::TextStyle::Button.resolve(&ctx.global_style());
    // The side panel's frame takes egui's 8pt margin each side.
    let available = size::SIDE_PANEL_DEFAULT_WIDTH - 16.0;
    let labels = [
        "ui-outliner-tab-scene",
        "ui-outliner-tab-materials",
        "ui-outliner-tab-textures",
        "ui-outliner-tab-comments",
    ];
    let needed: f32 = labels
        .iter()
        .map(|id| {
            let text = review_localization::tr(review_localization::Key::new(id, None));
            measure(&ctx, &text, tab_font.clone()) + 2.0 * size::OUTLINER_TAB_PAD_X
        })
        .sum();
    assert!(
        needed <= available,
        "the everyday tabs need {needed:.0}pt but the default panel's strip has {available:.0}pt"
    );
}
