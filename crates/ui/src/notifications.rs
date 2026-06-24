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
//! **Placement.** Toasts anchor to the bottom-right corner. The bottom margins
//! clear the status bar at every display scale (the bar is at most
//! [`size::STATUS_BAR_HEIGHT`] points tall, and the margin is that plus slack),
//! and the activity toast sits one row above the event stack so the two never
//! overlap.
//!
//! [`theme`]: crate::theme

use std::time::Duration;

use egui::Context;
use egui_notify::{Anchor, Toasts};

use crate::theme::size;

/// How long a transient result toast stays up before it dismisses itself.
const EVENT_DURATION: Duration = Duration::from_secs(2);

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
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    /// Create the collector. Both stacks anchor `BottomRight`; the bottom margins
    /// clear the status bar at every display scale (the bar is at most
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
        }
    }

    /// Push a transient success toast (e.g. a model or texture finished loading).
    pub fn success(&mut self, message: impl Into<String>) {
        self.events
            .success(message.into())
            .duration(Some(EVENT_DURATION));
    }

    /// Push a transient error toast (e.g. a decode or load failed).
    pub fn error(&mut self, message: impl Into<String>) {
        self.events
            .error(message.into())
            .duration(Some(EVENT_DURATION));
    }

    /// Push a transient info toast.
    pub fn info(&mut self, message: impl Into<String>) {
        self.events
            .info(message.into())
            .duration(Some(EVENT_DURATION));
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
        self.activity.show(ctx);
        self.events.show(ctx);
    }
}
