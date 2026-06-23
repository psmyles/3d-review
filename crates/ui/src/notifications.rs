//! The app's toast notification system, built on `egui-notify`.
//!
//! A single facility the rest of the app posts messages to: transient *result*
//! toasts (success / error / info) that expire on their own, plus a persistent
//! *activity* toast that stays up while a background job runs (texture decode).
//!
//! Per invariant 2 the UI never drives work: `app` owns this, calls
//! [`Notifications::begin_activity`] / [`Notifications::end_activity`] around a
//! background job and pushes the result toast, and calls [`Notifications::show`]
//! once per frame inside the egui pass. Layout comes from the central [`theme`]
//! (invariant 8); the toast surface/text colors are egui-notify's own, inherited
//! from the app's egui visuals (so toasts match the rest of the chrome).
//!
//! **Centering.** egui-notify only anchors to the four screen corners — it has no
//! bottom-center option. To honor a centered placement we anchor `BottomRight`
//! and, each frame, set the horizontal margin to `(screen_width - toast_width)/2`
//! so the toast's center lands on the viewport center. The toast's on-screen width
//! isn't exposed by the library, so we reconstruct it by measuring the caption
//! galley and adding egui-notify's fixed chrome (icon + close button + box
//! padding). This is an estimate that mirrors egui-notify's own layout math; if it
//! ever drifts the toast is merely a few pixels off-center, never broken.
//! `with_margin` consumes and returns the collector with every other field — the
//! live `toasts` included — intact, so re-applying it per frame keeps animation /
//! duration state.
//!
//! [`theme`]: crate::theme

use egui::Context;
use egui_notify::{Anchor, Toasts};

use crate::theme::size;

/// The app-wide toast collector. Owned by `app`; shown once per frame.
pub struct Notifications {
    /// Transient result toasts (success / error / info); each expires on its own.
    events: Toasts,
    /// The persistent "background work in progress" toast, kept in its **own**
    /// collector so it can be dismissed precisely: this collector only ever holds
    /// that one toast, so [`Toasts::dismiss_all_toasts`] targets exactly it (egui-
    /// notify gives toasts no stable id, so a shared collector couldn't).
    activity: Toasts,
    /// In-flight background jobs. The activity toast shows while this is `> 0` and
    /// is dismissed when it returns to `0`, so overlapping jobs share one indicator.
    active: usize,
    /// Caption of the live activity toast, retained so its width can be measured
    /// each frame for centering; cleared when the activity toast is dismissed.
    activity_caption: Option<String>,
    /// Caption of the most recent event toast, retained for the same reason and
    /// cleared once the event stack drains.
    event_caption: Option<String>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    /// Create the collector. Both stacks anchor `BottomRight` and are re-centered
    /// horizontally each frame in [`Notifications::show`]; the bottom margins clear
    /// the status bar at every display scale (the bar is at most
    /// [`size::STATUS_BAR_HEIGHT`] points tall, and the margin is that plus slack),
    /// and the activity toast sits one row above the event stack so the two never
    /// overlap.
    pub fn new() -> Self {
        let events = Toasts::new()
            .with_anchor(Anchor::BottomRight)
            .with_margin(egui::vec2(
                size::NOTIFICATION_MARGIN_X,
                size::NOTIFICATION_EVENT_MARGIN_Y,
            ))
            .with_spacing(size::NOTIFICATION_SPACING);
        let activity = Toasts::new()
            .with_anchor(Anchor::BottomRight)
            .with_margin(egui::vec2(
                size::NOTIFICATION_MARGIN_X,
                size::NOTIFICATION_ACTIVITY_MARGIN_Y,
            ))
            .with_spacing(size::NOTIFICATION_SPACING);
        Self {
            events,
            activity,
            active: 0,
            activity_caption: None,
            event_caption: None,
        }
    }

    /// Push a transient success toast (e.g. a model or texture finished loading).
    pub fn success(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.event_caption = Some(message.clone());
        self.events.success(message);
    }

    /// Push a transient error toast (e.g. a decode or load failed).
    pub fn error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.event_caption = Some(message.clone());
        self.events.error(message);
    }

    /// Push a transient info toast.
    pub fn info(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.event_caption = Some(message.clone());
        self.events.info(message);
    }

    /// Mark the start of a background job, raising the persistent activity toast on
    /// the first concurrent job. Pair every call with [`Notifications::end_activity`].
    pub fn begin_activity(&mut self, message: impl Into<String>) {
        if self.active == 0 {
            let message = message.into();
            self.activity_caption = Some(message.clone());
            self.activity
                .info(message)
                // Persistent (no expiry), not user-closable, no countdown bar: it
                // represents live work and is dismissed in `end_activity`.
                .duration(None)
                .closable(false)
                .show_progress_bar(false);
        }
        self.active += 1;
    }

    /// Mark a background job finished; hides the activity toast once all are done.
    pub fn end_activity(&mut self) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.activity.dismiss_all_toasts();
            self.activity_caption = None;
        }
    }

    /// Render the toast stacks. Call once per frame inside the egui pass. Each stack
    /// is re-centered on the viewport before it draws. While a toast is appearing /
    /// disappearing / counting down, egui-notify requests a repaint, which the app's
    /// on-demand loop honors (invariant 6); a steady persistent activity toast
    /// requests none, so it never spins the loop.
    pub fn show(&mut self, ctx: &Context) {
        let screen_w = ctx.screen_rect().width();

        if let Some(caption) = self.activity_caption.clone() {
            // The activity toast is not closable: no close button in its width.
            let width = toast_width(ctx, &caption, false);
            self.activity = std::mem::take(&mut self.activity).with_margin(egui::vec2(
                center_margin(screen_w, width),
                size::NOTIFICATION_ACTIVITY_MARGIN_Y,
            ));
        }
        self.activity.show(ctx);

        if let Some(caption) = self.event_caption.clone() {
            // success / info toasts are closable by default (error toasts are not,
            // but the close button only adds width — over-estimating nudges a
            // non-closable error toast a few px, which is imperceptible).
            let width = toast_width(ctx, &caption, true);
            self.events = std::mem::take(&mut self.events).with_margin(egui::vec2(
                center_margin(screen_w, width),
                size::NOTIFICATION_EVENT_MARGIN_Y,
            ));
        }
        self.events.show(ctx);
        if self.events.is_empty() {
            self.event_caption = None;
        }
    }
}

/// The `BottomRight` horizontal margin that centers a toast of the given width on a
/// viewport of the given width, never pulled closer than [`size::NOTIFICATION_MARGIN_X`]
/// to the edge. With a right anchor, the toast's right edge sits at
/// `screen_w - margin`, so `margin = (screen_w - toast_w) / 2` centers it.
fn center_margin(screen_w: f32, toast_w: f32) -> f32 {
    ((screen_w - toast_w) / 2.0).max(size::NOTIFICATION_MARGIN_X)
}

/// Reconstruct a toast's on-screen width, mirroring egui-notify's layout: box
/// padding on both sides, the level icon (sized to the caption height) with its
/// trailing padding, the caption galley, and — when `closable` — the close button
/// with its leading padding. egui-notify doesn't expose the laid-out width, so this
/// estimate is what lets us center the toast; see the module docs.
fn toast_width(ctx: &Context, caption: &str, closable: bool) -> f32 {
    let font = egui::TextStyle::Body.resolve(&ctx.style());
    let galley =
        ctx.fonts(|fonts| fonts.layout_no_wrap(caption.to_owned(), font, egui::Color32::WHITE));
    let caption_w = galley.rect.width();
    // egui-notify sizes the icon (and close button) to the caption line height.
    let icon = galley.rect.height();
    let pad = size::NOTIFICATION_PADDING;
    // box padding ×2 + icon + its trailing pad + caption [+ close button + its leading pad]
    let mut width = pad * 2.0 + icon + pad + caption_w;
    if closable {
        width += icon + pad;
    }
    width
}
