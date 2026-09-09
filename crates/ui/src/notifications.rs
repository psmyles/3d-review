//! The app's toast notification system, built on `egui-notify`.
//!
//! A single facility the rest of the app posts messages to: transient *result*
//! toasts (success / error / info) that expire on their own, a persistent
//! *activity card* that stays up while a background job runs (a model import, a
//! texture decode) and reports what it is doing, and a single-slot *mode* toast
//! that names the current view mode and replaces itself on each switch (so rapid
//! mode flipping doesn't stack toasts up).
//!
//! **The activity card is drawn here rather than by egui-notify**, which is the
//! one thing this module doesn't hand to that crate. A running job rewrites its
//! line every few frames, and egui-notify gives a live toast no way to change its
//! caption — the only way to "update" one is to dismiss it and push another,
//! which costs a slide-out and a slide-in each time and made a load read as a
//! flicker rather than as progress. So the card is an `egui::Area` pinned to the
//! same corner: the text and the bar change inside a box that never moves, and a
//! fixed width ([`size::ACTIVITY_WIDTH`]) keeps even its left edge still while
//! the text under it changes length.
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
//! by [`theme::px`] exactly as the bar itself is, and the activity card sits one
//! toast row above the event stack so the two never overlap.
//!
//! [`theme`]: crate::theme
//! [`theme::px`]: crate::theme::px

use egui::{Align2, Context, CornerRadius, Frame, Id, Order, Rect, TextWrapMode, vec2};
use egui_notify::{Anchor, Toasts};

use crate::theme::{self, color, motion, size};

/// The app-wide toast collector. Owned by `app`; shown once per frame.
pub struct Notifications {
    /// Transient result toasts (success / error / info); each expires on its own.
    events: Toasts,
    /// What the persistent activity card says, or `None` while nothing is
    /// running. Drawn by [`Notifications::show_activity`], not by egui-notify —
    /// see the module docs for why.
    activity: Option<Activity>,
    /// The single-slot mode-switch toast (e.g. the material mode name), in its
    /// **own** collector for the same reason as `activity`: each switch dismisses
    /// the prior toast before pushing the new one, so flipping modes quickly
    /// replaces the visible toast rather than stacking a fresh one each time.
    mode: Toasts,
    /// In-flight background jobs. The activity card shows while this is `> 0` and
    /// is dismissed when it returns to `0`, so overlapping jobs share one indicator.
    active: usize,
}

/// What the activity card is showing.
struct Activity {
    /// The job, named once when it starts (`Loading big.fbx…`). Steady for the
    /// job's whole life, so the card always says *what* is running even while the
    /// line under it changes.
    title: String,
    /// What the job is doing right now (`Reading… 47%`), or empty until it says.
    detail: String,
    /// How far along, when the job can say. Drives the bar under the detail line.
    fraction: Option<f32>,
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
            activity: None,
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

    /// Mark the start of a background job, raising the persistent activity card on
    /// the first concurrent job. Pair every call with [`Notifications::end_activity`].
    ///
    /// `title` names the job; what it is *doing* goes to
    /// [`Notifications::update_activity`].
    ///
    /// Overlapping jobs share one card and the newest one names it, since it is
    /// also the one whose reports are arriving. That is cheap now that the card is
    /// steady — while it was an egui-notify toast, retitling meant dismissing one
    /// and animating another in, so the *first* job kept the name to avoid the
    /// churn, and a second load started mid-measure left the card naming the file
    /// it was no longer reporting on.
    pub fn begin_activity(&mut self, title: impl Into<String>) {
        self.activity = Some(Activity {
            title: title.into(),
            detail: String::new(),
            fraction: None,
        });
        self.active += 1;
    }

    /// The running job reporting where it has got to: a short line under the
    /// title (`Reading… 47%`) and, when it can say, how far along it is.
    ///
    /// This rewrites the card in place — no dismiss, no re-push, nothing moves —
    /// so `app` may call it as often as it has something new to say. A no-op when
    /// nothing is running, so a report that arrives after its job ended can't
    /// raise a card.
    pub fn update_activity(&mut self, detail: impl Into<String>, fraction: Option<f32>) {
        if let Some(activity) = self.activity.as_mut() {
            activity.detail = detail.into();
            activity.fraction = fraction.map(|value| value.clamp(0.0, 1.0));
        }
    }

    /// Mark a background job finished; hides the activity card once all are done.
    pub fn end_activity(&mut self) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.activity = None;
        }
    }

    /// Render the toast stacks and the activity card. Call once per frame inside
    /// the egui pass. While a toast is appearing / disappearing / counting down,
    /// egui-notify requests a repaint, which the app's on-demand loop honors
    /// (invariant 6); the activity card is static paint and requests none, so a
    /// job that reports nothing new never spins the loop.
    pub fn show(&mut self, ctx: &Context) {
        self.sync_margins(ctx);
        self.show_activity(ctx);
        self.mode.show(ctx);
        self.events.show(ctx);
    }

    /// Paint the activity card: the job's title, the line under it, and a bar when
    /// the job knows its own denominator. Anchored to the same corner as the toast
    /// stacks, one row above them.
    fn show_activity(&mut self, ctx: &Context) {
        let Some(activity) = self.activity.as_ref() else {
            return;
        };
        let visuals = ctx.global_style().visuals.widgets.noninteractive;
        let offset = vec2(
            -theme::px(ctx, size::NOTIFICATION_MARGIN_X),
            -(theme::px(ctx, size::NOTIFICATION_EVENT_MARGIN_Y) + size::NOTIFICATION_ROW),
        );
        egui::Area::new(Id::new("activity_card"))
            .anchor(Align2::RIGHT_BOTTOM, offset)
            .order(Order::Foreground)
            // Purely a status readout: clicks belong to whatever is under it.
            .interactable(false)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(visuals.bg_fill)
                    .corner_radius(CornerRadius::same(size::ACTIVITY_CORNER_RADIUS))
                    .inner_margin(size::ACTIVITY_PADDING)
                    .show(ui, |ui| {
                        // Fixed width, so a longer or shorter line never shifts the
                        // card's left edge; the title truncates rather than wraps,
                        // since a second title line would move everything below it.
                        ui.set_width(size::ACTIVITY_WIDTH);
                        ui.spacing_mut().item_spacing.y = size::ACTIVITY_LINE_GAP;
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                        ui.label(&activity.title);
                        if !activity.detail.is_empty() {
                            ui.label(
                                egui::RichText::new(&activity.detail).color(color::TEXT_MUTED),
                            );
                        }
                        if let Some(fraction) = activity.fraction {
                            progress_bar(ui, fraction);
                        }
                    });
            });
    }

    /// Re-derive the stacks' margins for the current display scale. The insets are
    /// design pixels (invariant 8), so what clears the status bar depends on the
    /// scale — which is known here and not at construction.
    fn sync_margins(&mut self, ctx: &Context) {
        let x = theme::px(ctx, size::NOTIFICATION_MARGIN_X);
        let event_y = theme::px(ctx, size::NOTIFICATION_EVENT_MARGIN_Y);
        set_margin(&mut self.events, vec2(x, event_y));
        set_margin(&mut self.mode, vec2(x, event_y));
        // (The activity card rides one toast row higher, so the work indicator and
        // the result toasts never land on each other; it places itself in
        // `show_activity` from the same two tokens.)
    }
}

/// A slim filled track, the width of the card. Not `egui::ProgressBar`: that one
/// sizes itself to the available width *and* reserves a text row, which would
/// change the card's height the moment a fraction became known.
fn progress_bar(ui: &mut egui::Ui, fraction: f32) {
    let (track, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), size::ACTIVITY_PROGRESS_HEIGHT),
        egui::Sense::hover(),
    );
    let radius = CornerRadius::same(size::ACTIVITY_CORNER_RADIUS);
    ui.painter()
        .rect_filled(track, radius, color::ACTIVITY_PROGRESS_TRACK);
    let filled = Rect::from_min_size(
        track.min,
        vec2(track.width() * fraction.clamp(0.0, 1.0), track.height()),
    );
    ui.painter().rect_filled(filled, radius, color::ACCENT);
}

/// Apply `margin` to an existing stack. egui-notify exposes the margin only
/// through a consuming builder, so the collector is moved out, rebuilt and put
/// back — the queued toasts ride along untouched.
fn set_margin(toasts: &mut Toasts, margin: egui::Vec2) {
    let updated = std::mem::replace(toasts, Toasts::new()).with_margin(margin);
    *toasts = updated;
}
