//! The toolbar's Colliders tile: there only for a model with collision meshes,
//! as the skeleton's is only for a rigged one. A click shows or hides them, and
//! a right-click opens the Colliders panel, where their options are - which the
//! menu opens too, for a model whose collision meshes no name matches yet.
//!
//! These drive the real overlay through a headless `egui::Context`, and find
//! what to click by the icon or the text painted on it.

use egui::{Event, PointerButton, Pos2, RawInput};
use review_model::ModelData;
use review_render::OrbitCamera;
use review_ui::UiState;

/// The texture name the Colliders tile's icon is loaded under (`assets.rs`).
const COLLIDERS_ICON: &str = "icon_colliders";

/// Runs the real overlay headlessly, one pass at a time.
struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: ModelData,
    time: f64,
    /// Every textured rect the last pass painted - the toolbar's icons - with
    /// the name of its texture.
    icons: Vec<(String, egui::Rect)>,
    /// Every text the last pass painted, with where it went.
    texts: Vec<(String, egui::Rect)>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        Self {
            ctx,
            state: UiState::default(),
            model: ModelData::default(),
            time: 0.0,
            icons: Vec::new(),
            texts: Vec::new(),
        }
    }

    /// One egui pass at a 1280pt window - the narrowest the toolbar is laid
    /// out for - delivering `events`, a 60 Hz frame after the last: a press
    /// held longer than egui's click limit would not count as a click.
    fn pass(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let state = &mut self.state;
        let model = &self.model;
        let mut output = self.ctx.run_ui(input, |ui| {
            review_ui::draw_overlay(ui, state, OrbitCamera::default(), model, None, None);
        });
        // No renderer here, so the texture deltas are deliberately dropped.
        output.textures_delta.clear();

        let names: Vec<(egui::TextureId, String)> = self
            .ctx
            .tex_manager()
            .read()
            .allocated()
            .map(|(id, meta)| (*id, meta.name.clone()))
            .collect();
        self.icons.clear();
        self.texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &names, &mut self.icons, &mut self.texts);
        }
    }

    /// Where the Colliders tile's icon was painted, if it was.
    fn colliders_tile(&self) -> Option<egui::Rect> {
        self.icons
            .iter()
            .find(|(name, _)| name == COLLIDERS_ICON)
            .map(|&(_, rect)| rect)
    }

    /// Where `text` was painted, if it was: the first place, in paint order.
    fn text(&self, text: &str) -> Option<egui::Rect> {
        self.texts
            .iter()
            .find(|(painted, _)| painted == text)
            .map(|&(_, rect)| rect)
    }

    fn painted(&self, text: &str) -> bool {
        self.text(text).is_some()
    }

    /// Press and release `button` over the Colliders tile, a pass for each.
    fn click_tile(&mut self, button: PointerButton) {
        let pos = self
            .colliders_tile()
            .expect("the Colliders tile is on the toolbar")
            .center();
        self.click_at(pos, button);
    }

    /// Press and release the primary button over `text`.
    fn click_text(&mut self, text: &str) {
        let pos = self
            .text(text)
            .unwrap_or_else(|| panic!("`{text}` is on screen"))
            .center();
        self.click_at(pos, PointerButton::Primary);
    }

    /// Move to `pos`, then press and release `button` there, a pass for each.
    fn click_at(&mut self, pos: Pos2, button: PointerButton) {
        self.pass(vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.pass(vec![Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        // Once more, so whatever the click opened is drawn.
        self.pass(Vec::new());
    }
}

fn collect(
    shape: &egui::Shape,
    names: &[(egui::TextureId, String)],
    icons: &mut Vec<(String, egui::Rect)>,
    texts: &mut Vec<(String, egui::Rect)>,
) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, names, icons, texts);
            }
        }
        egui::Shape::Rect(rect) if rect.brush.is_some() => {
            let texture = rect.fill_texture_id();
            if let Some((_, name)) = names.iter().find(|(id, _)| *id == texture) {
                icons.push((name.clone(), rect.rect));
            }
        }
        egui::Shape::Text(text) => {
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            texts.push((text.galley.text().to_owned(), rect));
        }
        _ => {}
    }
}

#[test]
fn the_tile_is_only_there_for_a_model_with_collision_meshes() {
    let mut harness = Harness::new();
    harness.pass(Vec::new());
    assert_eq!(
        harness.colliders_tile(),
        None,
        "a model with no collision meshes has no Colliders tile",
    );

    harness.state.has_colliders = true;
    harness.pass(Vec::new());
    assert!(harness.colliders_tile().is_some());
}

/// With the skeleton's tile as well, the view group is at its widest: six
/// tiles. Every icon on the bar still has a place of its own on the screen.
#[test]
fn the_widest_view_group_still_fits_the_bar() {
    let mut harness = Harness::new();
    harness.state.has_bones = true;
    harness.state.has_colliders = true;
    harness.pass(Vec::new());

    let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0));
    for (name, rect) in &harness.icons {
        assert!(
            screen.contains_rect(*rect),
            "{name} is drawn off the window"
        );
    }
    for (index, (name, rect)) in harness.icons.iter().enumerate() {
        for (other_name, other) in &harness.icons[index + 1..] {
            assert!(
                !rect.intersects(*other),
                "{name} and {other_name} are drawn over each other",
            );
        }
    }
    let tile = harness
        .colliders_tile()
        .expect("the Colliders tile is drawn");
    let rightmost_view_tile = harness
        .icons
        .iter()
        .filter(|(name, _)| name == "icon_grid" || name == "icon_node_bone")
        .map(|&(_, rect)| rect.max.x)
        .fold(f32::MIN, f32::max);
    assert!(
        tile.min.x > rightmost_view_tile,
        "the Colliders tile ends the view group",
    );
}

#[test]
fn a_click_hides_the_collision_meshes_and_another_shows_them() {
    let mut harness = Harness::new();
    harness.state.has_colliders = true;
    harness.pass(Vec::new());
    assert!(harness.state.show_colliders, "they are shown to start with");

    harness.click_tile(PointerButton::Primary);
    assert!(!harness.state.show_colliders);
    harness.click_tile(PointerButton::Primary);
    assert!(harness.state.show_colliders);
}

#[test]
fn a_right_click_opens_the_panel_holding_their_options() {
    let mut harness = Harness::new();
    harness.state.has_colliders = true;
    harness.pass(Vec::new());
    assert!(!harness.painted("Collider names"));

    harness.click_tile(PointerButton::Secondary);
    assert!(
        harness.painted("Collider names"),
        "the Colliders panel, with the collider names, is open",
    );
    assert!(
        harness.state.show_colliders,
        "a right-click leaves them shown"
    );
}

/// With no collision mesh matched there is no tile to right-click, so the
/// menu's Preferences is the way to the names that would make one appear.
#[test]
fn the_menu_opens_the_panel_when_there_is_no_tile() {
    let mut harness = Harness::new();
    harness.pass(Vec::new());
    assert_eq!(harness.colliders_tile(), None);

    harness.open_from_preferences("Colliders...");
    assert!(
        harness.painted("Collider names"),
        "the Colliders panel is open",
    );
}

/// The name rows are edited through the real fields: one typed into the field
/// at the end is added, and a row's `X` takes it out again.
#[test]
fn a_name_is_added_at_the_end_and_crossed_out_by_its_x() {
    let mut harness = Harness::new();
    harness.state.colliders.names = vec!["*_box".into(), "*_hull".into()];
    harness.open_from_preferences("Colliders...");

    harness.click_text("Add new");
    harness.pass(vec![Event::Text("COL_*".into())]);
    harness.pass(vec![Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    harness.pass(Vec::new());
    assert_eq!(harness.state.colliders.names, ["*_box", "*_hull", "COL_*"]);
    assert!(harness.painted("COL_*"), "the new name has a row");

    // The `X` on the first name's row.
    let row = harness.text("*_box").expect("the first name has a row");
    let cross = harness
        .texts
        .iter()
        .filter(|(text, rect)| text == "X" && (rect.center().y - row.center().y).abs() < 4.0)
        .map(|&(_, rect)| rect.center())
        .next()
        .expect("the first name's row has its X");
    harness.click_at(cross, PointerButton::Primary);
    assert_eq!(harness.state.colliders.names, ["*_hull", "COL_*"]);
}

impl Harness {
    /// Pick `entry` from the menu's Preferences: the menu tile, Preferences,
    /// then the entry, a click each.
    fn open_from_preferences(&mut self, entry: &str) {
        self.pass(Vec::new());
        let menu = self
            .icons
            .iter()
            .find(|(name, _)| name == "icon_menu")
            .map(|&(_, rect)| rect.center())
            .expect("the menu tile is on the toolbar");
        self.click_at(menu, PointerButton::Primary);
        self.click_text("Preferences");
        self.click_text(entry);
    }
}
