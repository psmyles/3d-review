//! The notice column's geometry, driven through the *real* overlay.
//!
//! The column is an `egui::Area` of `Frame::window` cards laid out against
//! whatever space egui hands it, and the failure mode that matters is a card
//! that takes the size of the screen instead of the size of its text: a
//! full-height strip down the middle of the viewport with the title stranded
//! near the top, and a title too long to fit spilling outside the frame because
//! there was no finite width to truncate it against. Both are invisible to a
//! unit test that shows the column on a bare context — the bug needs the real
//! chrome, which is what this harness supplies.

use egui::{Pos2, RawInput};
use review_model::demo_cube_model;
use review_render::OrbitCamera;
use review_ui::{Notifications, UiState};

/// The window this harness lays the chrome out in.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Runs the real overlay plus the notice column, one pass at a time — the same
/// order `app`'s frame loop uses: chrome first, then the column, inside one
/// `run_ui`.
struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: review_model::ModelData,
    camera: OrbitCamera,
    notifications: Notifications,
    time: f64,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        let state = UiState {
            show_help_overlay: false,
            ..UiState::default()
        };
        Self {
            ctx,
            state,
            model: demo_cube_model(),
            camera: OrbitCamera::default(),
            notifications: Notifications::new(),
            time: 0.0,
        }
    }

    fn pass(&mut self, advance: f64) {
        self.time += advance;
        let input = RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, SCREEN)),
            ..Default::default()
        };
        let state = &mut self.state;
        let model = &self.model;
        let camera = self.camera;
        let notifications = &mut self.notifications;
        let mut output = self.ctx.run_ui(input, |ui| {
            review_ui::draw_overlay(ui, state, camera, model, None, None);
            notifications.show(ui.ctx(), state.chrome_insets);
        });
        // No renderer here, so the font-atlas deltas are deliberately dropped;
        // a `TexturesDelta` panics on drop unless that is said explicitly.
        output.textures_delta.clear();
    }

    /// The column's rect, once it has been laid out.
    fn column(&self) -> egui::Rect {
        egui::AreaState::load(&self.ctx, egui::Id::new("notifications"))
            .map(|area| area.rect())
            .expect("notice column laid out")
    }
}

/// A card is as tall as its text, not as tall as the window it floats over.
#[test]
fn a_card_does_not_take_the_height_of_the_window() {
    let mut harness = Harness::new();
    harness
        .notifications
        .success("Loaded cpg_pedestal_pebbles.fbx");
    for _ in 0..3 {
        harness.pass(0.01);
    }

    let column = harness.column();
    assert!(
        column.height() < SCREEN.y * 0.25,
        "one card grew to {}pt tall on a {}pt window",
        column.height(),
        SCREEN.y
    );
}

/// A second load in the same session must lay out exactly like the first. The
/// column is an `Area`, and an `Area` sizes the next frame's layout from the
/// size it measured on the last one — so a card that ever comes out wrong stays
/// wrong, and the symptom only shows on the *second* notice.
#[test]
fn a_later_notice_lays_out_like_the_first() {
    let mut harness = Harness::new();
    harness.notifications.success("Loaded first.fbx");
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let first = harness.column();

    // Let it expire, then post the next load's notice into the same column.
    harness.pass(5.0);
    harness
        .notifications
        .success("Loaded cpg_pedestal_pebbles.fbx");
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let second = harness.column();

    assert!(
        (second.height() - first.height()).abs() < 1.0,
        "the second notice is {}pt tall against the first's {}pt",
        second.height(),
        first.height()
    );
    assert!(
        (second.width() - first.width()).abs() < 1.0,
        "the second notice is {}pt wide against the first's {}pt",
        second.width(),
        first.width()
    );
}

/// A message longer than one line **wraps** rather than truncating, so the card
/// grows downward instead of cutting the text — and it still never grows wider.
///
/// This is the whole point of the header/body split. The message used to be the
/// card's header, clipped to a single line, so
/// `Couldn't load pedestal.fbx: unexpected end of file` reached the user as
/// `Couldn't load pedestal.fbx: unexpected en…` — everything after the colon,
/// which is the only part that says what went wrong, was the part thrown away.
#[test]
fn a_long_message_wraps_instead_of_being_cut_off() {
    let mut harness = Harness::new();
    harness.notifications.error("Couldn't load a.fbx");
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let short = harness.column();

    let mut harness = Harness::new();
    harness.notifications.error(
        "Couldn't load pedestal.fbx: unexpected end of file while reading the \
         geometry of 'stone_007_mesh', which is the kind of sentence a user has \
         to read all of.",
    );
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let long = harness.column();

    assert!(
        long.height() > short.height() + 10.0,
        "the long message did not wrap: {}pt against the short one's {}pt",
        long.height(),
        short.height()
    );
    assert!(
        (long.width() - short.width()).abs() < 1.0,
        "a long message widened the card from {}pt to {}pt",
        short.width(),
        long.width()
    );
}

/// A mode notice keeps its bare one-line shape: no kind header, no divider. It
/// names the view you just switched into, in a word, because you pressed the key
/// that switched it — so it has nothing to classify and nothing to explain.
#[test]
fn a_mode_notice_stays_compact() {
    let mut harness = Harness::new();
    harness.notifications.mode("Wireframe");
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let compact = harness.column().height();

    let mut harness = Harness::new();
    harness.notifications.info("Wireframe");
    for _ in 0..3 {
        harness.pass(0.01);
    }
    let headed = harness.column().height();

    assert!(
        compact < headed,
        "the mode notice ({compact}pt) should be shorter than the same text \
         under a kind header ({headed}pt)"
    );
}

/// A one-line notice is exactly its parts: a padded header row, the divider, and
/// a padded line of text. Nothing between them.
///
/// The card's blocks each carry their own padding, the way `egui::Window`'s
/// title bar and body do — so the spacing *between* them must be zero. It was
/// not: the column's `Area` sets `item_spacing.y` to the gap it wants between
/// cards, a `Frame`'s content inherits the style it is shown into, and that gap
/// was landing twice inside every card. The measurement below is built from the
/// live style rather than from constants, so it keeps meaning whatever the
/// theme does.
#[test]
fn a_card_is_its_parts_and_no_spacing_between_them() {
    let mut harness = Harness::new();
    harness.notifications.success("Loaded a.fbx");
    for _ in 0..3 {
        harness.pass(0.01);
    }

    let style = harness.ctx.global_style();
    let pad = style.spacing.window_margin.sum().y;
    let heading = harness
        .ctx
        .fonts_mut(|f| f.row_height(&egui::TextStyle::Heading.resolve(&style)));
    let body = harness
        .ctx
        .fonts_mut(|f| f.row_height(&egui::TextStyle::Body.resolve(&style)));
    let stroke = style.visuals.window_stroke().width;
    // Header (padded heading row) + divider + body (padded text row), plus the
    // frame's own stroke top and bottom.
    let expected = (pad + heading) + stroke + (pad + body) + stroke * 2.0;

    let measured = harness.column().height();
    assert!(
        measured <= expected + 2.0,
        "the card is {measured}pt against {expected}pt of actual content — \
         something is inserting space between the header, the rule and the body"
    );
}

/// The real sequence a model load produces: the progress card goes up, reports
/// stages against a bar, comes down, and the success notice replaces it in the
/// same column — twice, because the second load is where the report came from.
#[test]
fn a_full_load_sequence_leaves_a_normal_sized_card() {
    let mut harness = Harness::new();
    let mut sizes = Vec::new();

    for name in ["first.fbx", "cpg_pedestal_pebbles.fbx"] {
        harness
            .notifications
            .begin_activity(format!("Loading {name}…"));
        harness.pass(0.01);
        harness
            .notifications
            .update_activity("Reading… 12%", Some(0.12));
        harness.pass(0.01);
        harness
            .notifications
            .update_activity("Building geometry…", None);
        harness.pass(0.01);
        harness
            .notifications
            .update_activity("Reading… 94%", Some(0.94));
        harness.pass(0.01);
        harness.notifications.end_activity();
        harness.notifications.success(format!("Loaded {name}"));
        for _ in 0..3 {
            harness.pass(0.01);
        }
        sizes.push(harness.column());
        // Let it expire before the next load, as a real session would.
        harness.pass(5.0);
    }

    for (index, size) in sizes.iter().enumerate() {
        assert!(
            size.height() < SCREEN.y * 0.25,
            "load {index}: the card grew to {}pt tall on a {}pt window",
            size.height(),
            SCREEN.y
        );
    }
    assert!(
        (sizes[1].height() - sizes[0].height()).abs() < 1.0,
        "the second load's card is {}pt tall against the first's {}pt",
        sizes[1].height(),
        sizes[0].height()
    );
}
