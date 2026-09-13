//! The chrome's one tooltip shape: a title, a paragraph saying what the thing is
//! for, and a link into the manual.
//!
//! The single-line tooltip this replaced could only ever restate the button's
//! name. A tool's *reason* — why you would reach for face normals rather than
//! vertex normals, what a stalled simplify means — did not fit, so it lived only
//! in the README, which is not open while someone is deciding which button to
//! press. A [`Tip`] puts the one-line answer where it already was, the paragraph
//! under it, and the long answer one click away.
//!
//! egui keeps a tooltip open while the pointer is over it, or heading towards
//! it, whenever it holds an interactive widget (`Tooltip::should_show_tooltip`),
//! so the link is genuinely reachable rather than vanishing as the pointer
//! leaves the button.

use review_localization::Key;

use crate::docs::Page;
use crate::help;
use crate::keys;
use crate::theme::{color, size};

/// What a control's tooltip says.
///
/// Built from [`review_localization::Key`]s in the common case, but the title and body
/// take anything that converts into an `egui::WidgetText`, so a tooltip that has
/// to name a file or fold in the platform's modifier passes the formatted string
/// a typed message formatter gave it.
pub(crate) struct Tip {
    title: egui::WidgetText,
    description: Option<egui::WidgetText>,
    /// Notes about how the control is *operated* rather than what it does — that
    /// right-clicking opens options, or that clicking again cycles. They used to
    /// be spelled out inside each tooltip string ("(right-click for options)",
    /// ten times over), which meant translating the same parenthesis ten times
    /// and no way to keep them consistent.
    notes: Vec<Key>,
    page: Option<Page>,
}

impl Tip {
    /// A tooltip that says only what the control is — the old one-line shape.
    /// Reach for [`Tip::describe`] and [`Tip::page`] unless the name really is
    /// the whole answer.
    pub(crate) fn new(title: impl Into<egui::WidgetText>) -> Self {
        Self {
            title: title.into(),
            description: None,
            notes: Vec::new(),
            page: None,
        }
    }

    /// The paragraph under the title: what this control is for, in a sentence or
    /// two. Longer than that belongs on the manual page.
    pub(crate) fn describe(mut self, description: impl Into<egui::WidgetText>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The manual page the tooltip's last line links to.
    pub(crate) fn page(mut self, page: Page) -> Self {
        self.page = Some(page);
        self
    }

    /// Note that right-clicking this control opens its options panel.
    pub(crate) fn with_options(mut self) -> Self {
        self.notes.push(keys::ui_toolbar::HAS_OPTIONS);
        self
    }

    /// Note that clicking this control again cycles to the next value.
    pub(crate) fn cycles(mut self) -> Self {
        self.notes.push(keys::ui_toolbar::CYCLES);
        self
    }

    /// The tooltip's title, for a caller that also paints it — a form row's label
    /// *is* its tooltip's title, and spelling it twice at the call site is how
    /// the two drift apart.
    pub(crate) fn title(&self) -> egui::WidgetText {
        self.title.clone()
    }

    fn show(self, ui: &mut egui::Ui) {
        // Without a width the tooltip lays a paragraph out on one line and runs
        // off the side of the window; with one, egui wraps it.
        ui.set_max_width(size::TOOLTIP_MAX_WIDTH);
        ui.label(self.title.strong());
        if let Some(description) = self.description {
            ui.add_space(size::TOOLTIP_TITLE_GAP);
            ui.label(description);
        }
        for note in self.notes {
            ui.add_space(size::TOOLTIP_TITLE_GAP);
            ui.label(egui::RichText::from(note).color(color::TEXT_MUTED));
        }
        if let Some(page) = self.page {
            ui.add_space(size::TOOLTIP_TITLE_GAP);
            if ui.link(keys::common::LEARN_MORE).clicked() {
                help::request_page(ui.ctx(), page);
            }
        }
    }
}

/// Attach a [`Tip`] to a widget.
///
/// Free function rather than a `Response` extension trait: it is one call, and a
/// trait would have to be imported everywhere a tooltip is written.
pub(crate) fn tip(response: egui::Response, tip: Tip) -> egui::Response {
    response.on_hover_ui(|ui| tip.show(ui))
}

/// A title-and-paragraph tooltip body, for a widget whose response is handed
/// straight to egui's `on_hover_ui` rather than through [`tip`].
///
/// Same shape as a [`Tip`] without the manual link: these are the small controls
/// — a stack row's enable box, its reorder arrows — where a link would be a third
/// line under two short ones.
pub(crate) fn tip_body(ui: &mut egui::Ui, title: Key, description: Key) {
    ui.set_max_width(size::TOOLTIP_MAX_WIDTH);
    ui.label(egui::RichText::from(title).strong());
    ui.add_space(size::TOOLTIP_TITLE_GAP);
    ui.label(description);
}
