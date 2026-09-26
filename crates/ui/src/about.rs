//! The About box: what this build is, what it is drawing with, and where it
//! lives — plus the product links the menu's Help entries open.
//!
//! Everything it shows is a plain value `app` hands over once at startup
//! ([`AboutInfo`], invariant 2): the product name, version, copyright and
//! homepage come from `product.json` through `app`'s build script, and the
//! renderer line from the GPU layer. The box itself is an `egui::Modal`, egui's
//! own primitive for a window that blocks the rest of the chrome until it is
//! dismissed.

use crate::keys;
use crate::theme::size;

/// What the About box reports. Filled in by `app`; empty until it has been.
#[derive(Debug, Clone, Default)]
pub struct AboutInfo {
    /// The product's display name (`product.json`'s `productName`).
    pub product: String,
    /// This build's version, as `product.json` states it (`0.4.1`).
    pub version: String,
    /// The copyright line (`product.json`'s `copyright`).
    pub copyright: String,
    /// The project's home, `https://github.com/<owner>/<repo>`, which every
    /// product link is derived from.
    pub homepage: String,
    /// The graphics API sokol_gfx is drawing through (`Direct3D 11`, `Metal`).
    /// Empty until the GPU is up.
    pub renderer: String,
}

impl AboutInfo {
    /// The page for filing a new issue.
    pub fn new_issue_url(&self) -> String {
        format!("{}/issues/new", self.homepage)
    }

    /// The credits page, as the repository renders it.
    pub fn credits_url(&self) -> String {
        format!("{}/blob/main/docs/CREDITS.md", self.homepage)
    }
}

/// Whether the About box is up, and what it says.
#[derive(Debug, Clone, Default)]
pub struct AboutState {
    pub open: bool,
    pub info: AboutInfo,
}

/// Draw the About box while it is open. Escape, a click outside it and its
/// Close button all dismiss it.
pub(crate) fn draw(ctx: &egui::Context, about: &mut AboutState) {
    if !about.open {
        return;
    }
    let info = &about.info;
    let response = egui::Modal::new(egui::Id::new("about_modal")).show(ctx, |ui| {
        ui.set_width(size::ABOUT_WIDTH);
        ui.heading(info.product.as_str());
        ui.label(keys::ui_about::version(info.version.clone()));
        ui.add_space(size::ABOUT_SECTION_GAP);
        if !info.renderer.is_empty() {
            ui.label(keys::ui_about::renderer(info.renderer.clone()));
        }
        ui.add_space(size::ABOUT_SECTION_GAP);
        ui.hyperlink_to(keys::ui_about::SOURCE_CODE, &info.homepage);
        ui.weak(info.copyright.as_str());
        ui.add_space(size::ABOUT_SECTION_GAP);
        ui.separator();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.button(keys::ui_about::CLOSE).clicked()
        })
        .inner
    });
    if response.inner || response.should_close() {
        about.open = false;
    }
}
