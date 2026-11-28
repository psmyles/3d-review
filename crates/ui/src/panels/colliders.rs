//! Colliders tool options: how collision meshes are drawn, their colour, the
//! names that mark one, and a reset to defaults.
//!
//! The names are a list, a row each: a row is edited in place, its `X` removes
//! it, and the field under the last one adds another. Every edit goes through
//! [`apply`], so the list only ever holds names that parse, each once.

use review_audit::glob::GlobSet;

use super::aud_inspector::{committed_text_edit, parse_patterns};
use crate::docs::Page;
use crate::keys;
use crate::state::{ColliderDisplay, CollidersPanelState, UiState};
use crate::theme::{self, color, size};
use crate::widgets::{
    Tip, color_swatch_row, grid_control, grid_label_tip, labeled_combo, panel_footer, panel_grid,
};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsColliders;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    let colliders = &mut state.colliders;
    panel_grid(ui, "colliders", |ui| {
        display_row(ui, &mut colliders.display);
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::COLLIDER_COLOR)
                .describe(keys::ui_panels::COLLIDER_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut colliders.color,
            &color::COLLIDER_SWATCHES,
        );
        if let Some(edit) = name_rows(ui, &colliders.names) {
            apply(&mut colliders.names, edit);
        }
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.colliders = CollidersPanelState::default();
    }
}

/// How collision meshes are drawn while the toolbar's Colliders tile shows
/// them. Not tied to the wireframe toggle: drawn as edges, they show whether
/// the wireframe is on or not.
fn display_row(ui: &mut egui::Ui, display: &mut ColliderDisplay) {
    let label = |display: ColliderDisplay| match display {
        ColliderDisplay::Wireframe => keys::ui_panels::COLLIDERS_WIREFRAME,
        ColliderDisplay::Shaded => keys::ui_panels::COLLIDERS_SHADED,
    };
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::COLLIDERS)
            .describe(keys::ui_panels::COLLIDERS_DESCRIPTION)
            .page(PAGE),
        "colliders_display",
        label(*display),
        |ui| {
            crate::widgets::style_combo_popup(ui);
            for option in [ColliderDisplay::Wireframe, ColliderDisplay::Shaded] {
                ui.selectable_value(display, option, label(option));
            }
        },
    );
}

/// A change to the list of names, made by one of its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NameEdit {
    /// The row at this index now says these: none when it was emptied, more
    /// than one when a comma-separated list was typed or pasted into it.
    Replace(usize, Vec<String>),
    /// The row at this index's `X`.
    Remove(usize),
    /// What was typed into the field that adds one.
    Add(Vec<String>),
}

/// The names, a row each - the first beside the label - and then the field that
/// adds one. Returns the edit a row finished this frame, if any; the list is
/// left alone while it is drawn, so an index always names the row it came from.
fn name_rows(ui: &mut egui::Ui, names: &[String]) -> Option<NameEdit> {
    let label = || {
        Tip::new(keys::ui_panels::COLLIDER_NAMES)
            .describe(keys::ui_panels::COLLIDER_NAMES_DESCRIPTION)
            .page(PAGE)
    };
    let mut edit = None;
    for (index, name) in names.iter().enumerate() {
        if index == 0 {
            grid_label_tip(ui, label());
        } else {
            empty_label_cell(ui);
        }
        grid_control(ui, |ui| {
            // Laid out from the right, so the `X` is added first.
            if ui
                .button(theme::REMOVE_GLYPH)
                .on_hover_text(keys::ui_panels::COLLIDER_NAME_REMOVE)
                .clicked()
            {
                edit = Some(NameEdit::Remove(index));
            }
            let id = egui::Id::new(("collider_name", index));
            if let Some(patterns) = pattern_field(ui, id, name, None) {
                edit = Some(NameEdit::Replace(index, patterns));
            }
        });
        ui.end_row();
    }

    // With no names left, the field that adds one is the row the label goes on.
    if names.is_empty() {
        grid_label_tip(ui, label());
    } else {
        empty_label_cell(ui);
    }
    grid_control(ui, |ui| {
        let id = egui::Id::new("collider_name_add");
        if let Some(patterns) = pattern_field(ui, id, "", Some(keys::ui_panels::COLLIDER_NAMES_ADD))
        {
            edit = Some(NameEdit::Add(patterns));
        }
    });
    ui.end_row();
    edit
}

/// The label cell of a row that continues the one above: nothing in it, but
/// the column's width, as [`grid_label_tip`] gives every labelled row.
fn empty_label_cell(ui: &mut egui::Ui) {
    ui.allocate_exact_size(
        egui::vec2(size::PANEL_LABEL_COL_WIDTH, 0.0),
        egui::Sense::hover(),
    );
}

/// A name's field, filling what the row leaves it: red while what is typed
/// does not parse, and once the edit is finished, the names it holds. `None`
/// until then, and for text that does not parse, which is not taken - the
/// field goes back to what it said.
fn pattern_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    committed: &str,
    hint: Option<review_localization::Key>,
) -> Option<Vec<String>> {
    let mut valid = true;
    let (response, finished) = committed_text_edit(ui, id, committed, |text| {
        valid = parses(text);
        let mut field = egui::TextEdit::singleline(text)
            // The cell's width, margins included: a set width is the text's
            // alone, and would widen the window past its fellows.
            .desired_width(f32::INFINITY)
            .text_color(if valid {
                color::TEXT_BODY
            } else {
                color::SEVERITY_ERROR
            });
        if let Some(hint) = hint {
            field = field.hint_text(hint);
        }
        field
    });
    if !valid {
        let _ = response.on_hover_text(keys::ui_audit::PATTERN_INVALID);
    }
    finished
        .filter(|text| parses(text))
        .map(|text| parse_patterns(&text))
}

/// Whether every name in `text` (comma-separated) is a pattern that parses.
fn parses(text: &str) -> bool {
    GlobSet::new(&parse_patterns(text)).is_ok()
}

/// Make `edit` to `names`, then drop any name that is there twice, keeping the
/// first: typing a name that is already listed adds nothing.
fn apply(names: &mut Vec<String>, edit: NameEdit) {
    match edit {
        NameEdit::Replace(index, patterns) => {
            if index < names.len() {
                names.splice(index..=index, patterns);
            }
        }
        NameEdit::Remove(index) => {
            if index < names.len() {
                names.remove(index);
            }
        }
        NameEdit::Add(patterns) => names.extend(patterns),
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn a_row_edited_in_place_keeps_its_place() {
        let mut names = list(&["*_box", "*_sphere", "*_capsule"]);
        apply(&mut names, NameEdit::Replace(1, list(&["*_ball"])));
        assert_eq!(names, ["*_box", "*_ball", "*_capsule"]);
    }

    #[test]
    fn a_row_emptied_or_crossed_out_is_gone() {
        let mut names = list(&["*_box", "*_sphere", "*_capsule"]);
        apply(&mut names, NameEdit::Replace(1, Vec::new()));
        assert_eq!(names, ["*_box", "*_capsule"]);
        apply(&mut names, NameEdit::Remove(0));
        assert_eq!(names, ["*_capsule"]);
    }

    /// A comma-separated list typed or pasted into one row becomes a row each,
    /// where that row was.
    #[test]
    fn a_list_typed_into_a_row_becomes_a_row_each() {
        let mut names = list(&["*_box", "*_capsule"]);
        apply(
            &mut names,
            NameEdit::Replace(0, parse_patterns("COL_*, *_phys")),
        );
        assert_eq!(names, ["COL_*", "*_phys", "*_capsule"]);
        apply(&mut names, NameEdit::Add(parse_patterns("UCX_*,UBX_*")));
        assert_eq!(names, ["COL_*", "*_phys", "*_capsule", "UCX_*", "UBX_*"]);
    }

    #[test]
    fn a_name_already_listed_is_not_listed_again() {
        let mut names = list(&["*_box", "*_sphere"]);
        apply(&mut names, NameEdit::Add(list(&["*_sphere"])));
        assert_eq!(names, ["*_box", "*_sphere"]);
        // Edited into one further down: the row keeps it, the later one goes.
        apply(&mut names, NameEdit::Replace(0, list(&["*_sphere"])));
        assert_eq!(names, ["*_sphere"]);
    }

    /// The first field the list is short of is the one that adds a name, so
    /// the last name can go and a first can be added to an empty list.
    #[test]
    fn the_list_can_be_emptied_and_filled_again() {
        let mut names = list(&["*_box"]);
        apply(&mut names, NameEdit::Remove(0));
        assert!(names.is_empty());
        apply(&mut names, NameEdit::Add(list(&["COL_*"])));
        assert_eq!(names, ["COL_*"]);
    }

    #[test]
    fn a_pattern_that_does_not_parse_is_refused() {
        assert!(parses("COL_*, *_phys"));
        assert!(!parses("COL_[*"));
    }
}
