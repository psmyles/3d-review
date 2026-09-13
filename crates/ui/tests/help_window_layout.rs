//! The Help window must be the size the reader put it at, whatever page is open.
//!
//! `egui::Window` is resizable through `egui::Resize`, and on every frame it is
//! not being dragged `Resize` does `desired_size = desired_size.max(content)`.
//! That makes a window's width a running maximum over everything ever shown in
//! it: one manual page with a wide table stretched the window on arrival, left it
//! stretched (nothing shrinks a `max`), and pinned the resize handle, because
//! egui also refuses to let a drag shrink a window below what its content
//! measures. Browsing the contents list therefore resized the window under the
//! reader, a page at a time, and the width could not be put back.
//!
//! The fix is that nothing inside the window may report a width of its own: the
//! reading pane scrolls in both directions so overflow becomes a scrollbar, the
//! contents rows wrap inside their column, and the page heading truncates. This
//! walks every page in the manual and checks the window never moves.

use egui::{Pos2, RawInput};
use review_model::demo_cube_model;
use review_render::OrbitCamera;
use review_ui::UiState;
use review_ui::docs::{Page, TOC};

/// The window the chrome is laid out in. Deliberately not much larger than the
/// Help window's default size, so a page that overflows has nowhere to hide.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A window small enough that `Resize` clamps the Help window's default size
/// down to it - the same state a reader who has dragged the window small is in,
/// and the case a wide page would silently push back out.
const SMALL_SCREEN: egui::Vec2 = egui::vec2(560.0, 460.0);

/// The Help window's own id, as `help::draw` sets it.
const HELP_WINDOW: &str = "help_window";

struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: review_model::ModelData,
    camera: OrbitCamera,
    screen: egui::Vec2,
    time: f64,
}

impl Harness {
    fn new(screen: egui::Vec2) -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        Self {
            ctx,
            state: UiState::default(),
            model: demo_cube_model(),
            camera: OrbitCamera::default(),
            screen,
            time: 0.0,
        }
    }

    fn pass(&mut self) {
        self.time += 0.016;
        let input = RawInput {
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.screen)),
            ..Default::default()
        };
        let state = &mut self.state;
        let model = &self.model;
        let camera = self.camera;
        let mut output = self.ctx.run_ui(input, |ui| {
            review_ui::draw_overlay(ui, state, camera, model, None, None);
        });
        // No renderer here, so the font-atlas deltas are deliberately dropped; a
        // `TexturesDelta` panics on drop unless that is said explicitly.
        output.textures_delta.clear();
    }

    /// Show `page` and let the layout settle. Three passes: `Resize` reads the
    /// size the *previous* frame measured, so a growth caused by a page shows on
    /// the frame after it is first drawn.
    fn show(&mut self, page: Page) {
        self.state.help.open_page(page);
        for _ in 0..3 {
            self.pass();
        }
    }

    fn window_width(&self) -> f32 {
        self.ctx
            .memory(|memory| memory.area_rect(egui::Id::new(HELP_WINDOW)))
            .expect("the Help window was laid out")
            .width()
    }
}

/// The regression: open every page the contents list offers and check the window
/// is still the width it started at.
#[test]
fn browsing_the_contents_does_not_resize_the_window() {
    let mut harness = Harness::new(SCREEN);
    harness.show(Page::default());
    let opening_width = harness.window_width();

    let mut widest = (opening_width, Page::default());
    for (_, page) in TOC {
        harness.show(page);
        let width = harness.window_width();
        if width > widest.0 {
            widest = (width, page);
        }
    }

    assert!(
        widest.0 <= opening_width + 1.0,
        "the Help window opened at {opening_width}pt and grew to {}pt on the \
         `{}` page - `Resize` never gives that width back, and the resize handle \
         will not shrink past it either",
        widest.0,
        widest.1.id(),
    );
}

/// And the same must hold on a window the Help window barely fits in, which is
/// where `Resize`'s growth clamp bites hardest: the reading pane is then narrower
/// than any table in the manual, so every page that has one is a chance to push
/// the window back out.
#[test]
fn no_page_widens_the_window_on_a_small_screen() {
    let mut harness = Harness::new(SMALL_SCREEN);
    harness.show(Page::default());
    let opening_width = harness.window_width();
    assert!(
        opening_width < review_ui::theme::size::HELP_WINDOW_DEFAULT[0],
        "the small-screen case did not actually clamp the window down: {opening_width}pt"
    );

    for (_, page) in TOC {
        harness.show(page);
        assert!(
            harness.window_width() <= opening_width + 1.0,
            "`{}` pushed the Help window from {opening_width}pt out to {}pt on a {}pt screen",
            page.id(),
            harness.window_width(),
            SMALL_SCREEN.x,
        );
    }
}
