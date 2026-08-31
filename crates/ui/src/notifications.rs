//! The app's toast notification system, built on `egui-notify`.
//!
//! A single facility the rest of the app posts messages to: transient *result*
//! toasts (success / error / info) that expire on their own, a persistent
//! *activity* toast that stays up while a background job runs (texture decode),
//! and a single-slot *mode* toast that names the current view mode and replaces
//! itself on each switch (so rapid mode flipping doesn't stack toasts up).
//!
//! Per invariant 2 the UI never drives work: `app` owns this, calls
//! [`Notifications::begin_activity`] / [`Notifications::end_activity`] around a
//! background job and pushes the result toast, and calls [`Notifications::show`]
//! once per frame inside the egui pass. Layout comes from the central [`theme`]
//! (invariant 8); the toast surface/text colors are egui-notify's own, inherited
//! from the app's egui visuals (so toasts match the rest of the chrome).
//!
//! **Placement.** Toasts anchor to the bottom-right corner, clear of the status
//! bar: the bottom margin is the bar's own height plus a margin, scaled to points
//! by [`theme::px`] exactly as the bar itself is, and the activity toast sits one
//! toast row above the event stack so the two never overlap.
//!
//! [`theme`]: crate::theme
//! [`theme::px`]: crate::theme::px

use egui::Context;
use egui_notify::{Anchor, Toasts};

use crate::theme::{self, motion, size};

/// The app-wide toast collector. Owned by `app`; shown once per frame.
pub struct Notifications {
    /// Transient result toasts (success / error / info); each expires on its own.
    events: Toasts,
    /// The persistent "background work in progress" toast, kept in its **own**
    /// collector so it can be dismissed precisely: this collector only ever holds
    /// that one toast, so [`Toasts::dismiss_all_toasts`] targets exactly it (egui-
    /// notify gives toasts no stable id, so a shared collector couldn't).
    activity: Toasts,
    /// The single-slot mode-switch toast (e.g. the material mode name), in its
    /// **own** collector for the same reason as `activity`: each switch dismisses
    /// the prior toast before pushing the new one, so flipping modes quickly
    /// replaces the visible toast rather than stacking a fresh one each time.
    mode: Toasts,
    /// In-flight background jobs. The activity toast shows while this is `> 0` and
    /// is dismissed when it returns to `0`, so overlapping jobs share one indicator.
    active: usize,
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    /// Create the collector. All three stacks anchor `BottomRight`; their margins
    /// are applied per-frame in [`Notifications::show`], which is where the
    /// display scale the design-pixel insets need is known.
    pub fn new() -> Self {
        let stack = || {
            Toasts::new()
                .with_anchor(Anchor::BottomRight)
                .with_spacing(size::NOTIFICATION_SPACING)
        };
        Self {
            events: stack(),
            // The mode toast shares the event row's anchor/margin: it is a
            // transient event-class toast, just one kept to a single
            // replaceable slot.
            mode: stack(),
            activity: stack(),
            active: 0,
        }
    }

    /// Push a transient success toast (e.g. a model or texture finished loading).
    pub fn success(&mut self, message: impl Into<String>) {
        self.events
            .success(message.into())
            .duration(Some(motion::NOTIFICATION_EVENT));
    }

    /// Push a transient error toast (e.g. a decode or load failed).
    pub fn error(&mut self, message: impl Into<String>) {
        self.events
            .error(message.into())
            .duration(Some(motion::NOTIFICATION_EVENT));
    }

    /// Push a transient info toast.
    pub fn info(&mut self, message: impl Into<String>) {
        self.events
            .info(message.into())
            .duration(Some(motion::NOTIFICATION_EVENT));
    }

    /// Show the current view mode (e.g. the material mode name) as a transient
    /// toast, **replacing** any mode toast still on screen. The dedicated `mode`
    /// collector holds only this toast, so dismissing it first means rapid mode
    /// switching cross-fades one toast in place instead of stacking a new one per
    /// switch.
    pub fn mode(&mut self, message: impl Into<String>) {
        self.mode.dismiss_all_toasts();
        self.mode
            .info(message.into())
            .duration(Some(motion::NOTIFICATION_EVENT));
    }

    /// Mark the start of a background job, raising the persistent activity toast on
    /// the first concurrent job. Pair every call with [`Notifications::end_activity`].
    pub fn begin_activity(&mut self, message: impl Into<String>) {
        if self.active == 0 {
            self.activity
                .info(message.into())
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
        }
    }

    /// Render the toast stacks. Call once per frame inside the egui pass. While a
    /// toast is appearing / disappearing / counting down, egui-notify requests a
    /// repaint, which the app's on-demand loop honors (invariant 6); a steady
    /// persistent activity toast requests none, so it never spins the loop.
    pub fn show(&mut self, ctx: &Context) {
        self.sync_margins(ctx);
        self.activity.show(ctx);
        self.mode.show(ctx);
        self.events.show(ctx);
    }

    /// Re-derive the stacks' margins for the current display scale. The insets are
    /// design pixels (invariant 8), so what clears the status bar depends on the
    /// scale — which is known here and not at construction.
    fn sync_margins(&mut self, ctx: &Context) {
        let x = theme::px(ctx, size::NOTIFICATION_MARGIN_X);
        let event_y = theme::px(ctx, size::NOTIFICATION_EVENT_MARGIN_Y);
        set_margin(&mut self.events, egui::vec2(x, event_y));
        set_margin(&mut self.mode, egui::vec2(x, event_y));
        // The activity toast rides one toast row higher, so the work indicator and
        // the result toasts never land on each other.
        set_margin(
            &mut self.activity,
            egui::vec2(x, event_y + size::NOTIFICATION_ROW),
        );
    }
}

/// Apply `margin` to an existing stack. egui-notify exposes the margin only
/// through a consuming builder, so the collector is moved out, rebuilt and put
/// back — the queued toasts ride along untouched.
fn set_margin(toasts: &mut Toasts, margin: egui::Vec2) {
    let updated = std::mem::replace(toasts, Toasts::new()).with_margin(margin);
    *toasts = updated;
}
