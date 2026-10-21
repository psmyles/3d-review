//! The Outliner's Scene tab lays out only the rows in view, and keyboard
//! navigation still scrolls to a row that was not laid out when it was chosen.
//!
//! Both are easy to break invisibly: a list that lays out every row looks the
//! same, only slower, and a scroll aimed at a row's response silently does
//! nothing once that row is not drawn. These drive the real overlay through a
//! headless `egui::Context` and read back what it painted.

use egui::{Event, Pos2, RawInput};
use review_model::{ModelData, NodeKind, SceneNode};
use review_render::OrbitCamera;
use review_ui::{Selection, UiState};

/// Far more rows than the window can show.
const NODES: usize = 400;

fn name(index: usize) -> String {
    format!("node {index}")
}

/// `NODES` sibling roots, so the tree lists every one of them in order.
fn many_nodes() -> ModelData {
    ModelData {
        nodes: (0..NODES)
            .map(|index| SceneNode {
                name: name(index),
                parent: None,
                mesh_part: None,
                source_vertex_count: 0,
                transform: glam::Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: NodeKind::Empty,
                bone: None,
            })
            .collect(),
        ..ModelData::default()
    }
}

/// Runs the real overlay headlessly, one pass at a time.
struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: ModelData,
    time: f64,
    /// Every text the last pass painted, with where it went and the rect it
    /// was clipped to.
    texts: Vec<(String, egui::Rect, egui::Rect)>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        Self {
            ctx,
            state: UiState::default(),
            model: many_nodes(),
            time: 0.0,
            texts: Vec::new(),
        }
    }

    /// One egui pass, a second after the last (long enough for any scroll
    /// animation to arrive), delivering `events`.
    fn pass(&mut self, events: Vec<Event>) {
        self.time += 1.0;
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
        // No renderer here, so the font-atlas deltas are deliberately dropped.
        output.textures_delta.clear();
        self.texts.clear();
        for clipped in &output.shapes {
            collect_texts(&clipped.shape, clipped.clip_rect, &mut self.texts);
        }
    }

    /// Where `text` was painted, and what it was clipped to, if it was.
    fn painted(&self, text: &str) -> Option<(egui::Rect, egui::Rect)> {
        self.texts
            .iter()
            .find(|(painted, _, _)| painted == text)
            .map(|&(_, rect, clip)| (rect, clip))
    }
}

fn collect_texts(
    shape: &egui::Shape,
    clip: egui::Rect,
    out: &mut Vec<(String, egui::Rect, egui::Rect)>,
) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_texts(shape, clip, out);
            }
        }
        egui::Shape::Text(text) => {
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            out.push((text.galley.text().to_owned(), rect, clip));
        }
        _ => {}
    }
}

fn key(key: egui::Key) -> Vec<Event> {
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    vec![event(true), event(false)]
}

#[test]
fn navigating_to_a_row_out_of_view_scrolls_it_in_and_lays_out_only_the_rows_shown() {
    let mut harness = Harness::new();
    harness.pass(Vec::new());
    // The Outliner takes the arrow keys while the pointer is over it.
    harness.pass(vec![Event::PointerMoved(Pos2::new(60.0, 400.0))]);
    assert!(
        harness.painted(&name(0)).is_some(),
        "the list starts at its first row"
    );
    assert!(
        harness.painted(&name(NODES - 1)).is_none(),
        "a row far below the window is not laid out at all"
    );

    // With nothing selected, Up enters the list at its last row.
    harness.pass(key(egui::Key::ArrowUp));
    assert_eq!(harness.state.selection, Selection::Node(NODES - 1));
    for _ in 0..4 {
        harness.pass(Vec::new());
    }

    let (rect, clip) = harness
        .painted(&name(NODES - 1))
        .expect("the selected row is laid out once it is scrolled to");
    assert!(
        clip.contains_rect(rect),
        "the selected row is in view: {rect:?} inside {clip:?}"
    );
    assert!(
        harness.painted(&name(0)).is_none(),
        "the first row, now scrolled away, is no longer laid out"
    );
}
