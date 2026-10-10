//! The stats overlay's rows: hovering one explains it, clicking one copies it.
//!
//! Both are easy to break invisibly. egui's labels are selectable by default,
//! and a selectable label senses drags — so it becomes an interactive widget
//! that swallows the row's hover, and the explanation silently stops appearing
//! anywhere the pointer lands on text. These drive the real overlay through a
//! headless `egui::Context` to pin the behaviour.

use egui::{Event, Pos2, RawInput};
use review_model::demo_cube_model;
use review_render::OrbitCamera;
use review_ui::UiState;

/// Runs the real overlay headlessly, one pass at a time.
struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: review_model::ModelData,
    camera: OrbitCamera,
    time: f64,
    copied: Vec<String>,
    cursor: egui::CursorIcon,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        let state = UiState::default();
        Self {
            ctx,
            state,
            model: demo_cube_model(),
            camera: OrbitCamera::default(),
            time: 0.0,
            copied: Vec::new(),
            cursor: egui::CursorIcon::Default,
        }
    }

    /// One egui pass, `advance` seconds after the last, delivering `events`.
    fn pass(&mut self, advance: f64, events: Vec<Event>) {
        self.time += advance;
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
        let camera = self.camera;
        let mut output = self.ctx.run_ui(input, |ui| {
            review_ui::draw_overlay(
                ui,
                state,
                camera,
                review_render::UvCamera::default(),
                model,
                None,
                None,
            );
        });
        // This harness lays the chrome out to inspect its geometry; it has no
        // renderer, so the font-atlas deltas are deliberately discarded. A
        // `TexturesDelta` panics on drop unless that is said explicitly.
        output.textures_delta.clear();
        self.cursor = output.platform_output.cursor_icon;
        for command in output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.copied.push(text);
            }
        }
    }

    /// The stats card's rect, once it has been laid out.
    fn card(&self) -> egui::Rect {
        egui::AreaState::load(&self.ctx, egui::Id::new("stats_overlay"))
            .map(|area| area.rect())
            .expect("stats card laid out")
    }

    fn tooltip_shown(&self) -> bool {
        self.ctx.memory(|mem| {
            mem.areas()
                .visible_layer_ids()
                .iter()
                .any(|layer| layer.order == egui::Order::Tooltip)
        })
    }
}

/// A point over the *text* of a stats row — the case a selectable label breaks,
/// as opposed to the blank space between the row's label and its value.
fn over_row_text(card: egui::Rect) -> Pos2 {
    Pos2::new(card.left() + 16.0, card.top() + 40.0)
}

#[test]
fn hovering_a_stats_row_explains_it() {
    let mut harness = Harness::new();
    // A few passes to settle the layout: the card is an `Area`, so its rect is
    // only known after it has been laid out once.
    for _ in 0..3 {
        harness.pass(0.05, Vec::new());
    }
    let pos = over_row_text(harness.card());

    harness.pass(0.0, vec![Event::PointerMoved(pos)]);
    assert!(
        !harness.tooltip_shown(),
        "the tooltip should wait out egui's hover delay, not flash up instantly"
    );

    // Rest the pointer there: past the delay, the row explains itself.
    for _ in 0..4 {
        harness.pass(0.25, Vec::new());
    }
    assert!(
        harness.tooltip_shown(),
        "a stats row should explain itself when the pointer rests on its text"
    );
    // The stats are display-only: a text cursor over them offers a selection
    // that leads nowhere, and the selectable label behind it swallows the row's
    // hover.
    assert_ne!(
        harness.cursor,
        egui::CursorIcon::Text,
        "stats text should not be selectable"
    );
}

#[test]
fn clicking_a_stats_row_copies_it() {
    let mut harness = Harness::new();
    // A few passes to settle the layout: the card is an `Area`, so its rect is
    // only known after it has been laid out once.
    for _ in 0..3 {
        harness.pass(0.05, Vec::new());
    }
    let pos = over_row_text(harness.card());
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };

    harness.pass(0.0, vec![Event::PointerMoved(pos)]);
    harness.pass(0.05, vec![button(true)]);
    harness.pass(0.05, vec![button(false)]);

    let copied = harness.copied.join(" | ");
    assert_eq!(
        harness.copied.len(),
        1,
        "one click should copy exactly one row, got: {copied}"
    );
    // The demo cube's stats card: whichever row was hit, it copies as
    // "<label>: <value>".
    assert!(
        copied.contains(": "),
        "a copied row should read as `label: value`, got: {copied}"
    );
}
