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
    /// The widest the reading pane's content has overhung it on *any* frame so
    /// far, not merely once the layout settled. The first frame is its own case:
    /// the viewer redraws on demand (invariant 6), so a page that only comes
    /// right on the second frame may never get one.
    worst_overflow: f32,
}

impl Harness {
    fn new(screen: egui::Vec2) -> Self {
        Self::at_scale(screen, 1.0)
    }

    /// At a given display scale. Scale matters here because egui rounds a
    /// measurement to the display's *pixel* grid, and at any scale but 1.0 a
    /// point is not a whole pixel - which is the whole of the bug
    /// `the_pane_fits_at_every_display_scale` exists for.
    fn at_scale(screen: egui::Vec2, pixels_per_point: f32) -> Self {
        let ctx = egui::Context::default();
        review_ui::init_style(&ctx);
        ctx.set_pixels_per_point(pixels_per_point);
        Self {
            ctx,
            state: UiState::default(),
            model: demo_cube_model(),
            camera: OrbitCamera::default(),
            screen,
            time: 0.0,
            worst_overflow: 0.0,
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
        self.worst_overflow = self.worst_overflow.max(self.state.help.page_overflow);
    }

    /// Show `page` and let the layout settle. Three passes: `Resize` reads the
    /// size the *previous* frame measured, so a growth caused by a page shows on
    /// the frame after it is first drawn.
    fn show(&mut self, page: Page) {
        self.state.help.open_page(page);
        self.worst_overflow = 0.0;
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

/// No page may overhang the reading pane, so no horizontal scrollbar is ever
/// drawn under the manual.
///
/// Three separate things used to break this, and each is worth naming because
/// none of them is visible from outside egui.
///
/// The page's margin was a `Frame` *inside* the scroll area, and
/// `CommonMarkViewer` does not wrap against the `Ui` it is handed - it wraps
/// against the scroll viewport. So the margin bought the text nothing: a
/// paragraph ran the viewport's full width and the frame then added its two
/// margins around it, leaving every page exactly twice the margin too wide.
///
/// A markdown table is an `egui::Grid` of plain horizontal rows, which in egui
/// means `TextWrapMode::Extend` - one unbroken line per cell. The widest table
/// in the manual came out four times the width of the window. There are no
/// tables in the manual now, and `help_pages.rs` holds that.
///
/// And the wrap width could not be made to equal the measured width, because
/// egui rounds a measurement up to the pixel grid: see
/// `the_pane_fits_at_every_display_scale`.
///
/// The tolerance is a hundredth of a point rather than something forgiving,
/// because egui raises the scrollbar on *any* overflow, and the bug that got
/// past this test the first time overhung by six hundredths.
///
/// Checked at a window the viewer is actually used at, and deliberately not at
/// [`SMALL_SCREEN`]. Squeezed that far the reading pane is a couple of dozen
/// points wide, and not even a wrapped word fits. That case is what horizontal
/// scrolling is *for*, and `no_page_widens_the_window_on_a_small_screen` above
/// holds the line that matters there: the window must not grow.
#[test]
fn no_page_overhangs_the_reading_pane() {
    let mut harness = Harness::new(SCREEN);
    let mut overflowing = Vec::new();

    for (_, page) in TOC {
        harness.show(page);
        if harness.worst_overflow > 0.01 {
            overflowing.push(format!("{}: {:.3}pt", page.id(), harness.worst_overflow));
        }
    }

    assert!(
        overflowing.is_empty(),
        "these pages overhang the reading pane, so the manual is drawn with a \
         horizontal scrollbar under it: {overflowing:#?}"
    );
}

/// And the same at every display scale, on the first frame as well as the last.
///
/// The bug: text is wrapped at a width, but the galley that comes back is
/// *measured*, and egui rounds a measurement up to the display's pixel grid. At
/// 100% a point is a pixel and the two agree; at 125% or 150% they do not, and a
/// paragraph wrapped at exactly the viewport's width reports back a fraction of a
/// point wider than it. egui raises a horizontal scrollbar on any overflow at
/// all, so the manual opened with a scrollbar that had almost nothing to scroll.
///
/// It survived the sweep above twice over: that one runs at 100%, where the
/// rounding cancels, and it read the overhang only after the layout had settled,
/// while the viewer redraws on demand and often shows the first frame and no
/// other. Hence both axes here.
///
/// A few pages rather than all of them: the overhang is an artefact of rounding,
/// so it is the same on every page, and the scales multiply the run time.
#[test]
fn the_pane_fits_at_every_display_scale() {
    let pages = [
        Page::default(),
        Page::Shading,
        Page::Keyboard,
        Page::OptOperations,
    ];
    let mut overflowing = Vec::new();

    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        // A width that is not a round number of pixels at any of these scales,
        // which is the case the rounding actually bites in.
        for width in [1101.0, 1280.0, 1717.0] {
            let mut harness = Harness::at_scale(egui::vec2(width, 900.0), scale);
            for page in pages {
                harness.show(page);
                if harness.worst_overflow > 0.01 {
                    overflowing.push(format!(
                        "{} at {scale}x, {width}pt wide: {:.3}pt",
                        page.id(),
                        harness.worst_overflow
                    ));
                }
            }
        }
    }

    assert!(
        overflowing.is_empty(),
        "the reading pane overhangs at these scales, so the manual opens with a \
         horizontal scrollbar that has a fraction of a point to scroll: \
         {overflowing:#?}"
    );
}
