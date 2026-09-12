//! The app's notification system: one column of cards at the bottom centre of
//! the free viewport, drawn directly on egui.
//!
//! Everything the app has to say arrives here — a transient result (success /
//! info), something the user should act on (warning / error), a *grouped*
//! report whose title carries a body of detail lines, the single-slot mode
//! toast that names the current view mode, and the progress card that stays up
//! while a background job runs and reports what it is doing.
//!
//! **This used to be `egui-notify`, and three things it could not do are why it
//! isn't any more.** Every toast slid in and out on a hard-coded cubic ease with
//! no switch to turn it off, which reads as distraction rather than as feedback
//! on chrome that is up this often. Its anchors are the four screen corners, so
//! bottom centre — where the eye already is, over the model — was unreachable.
//! And a multi-part result had to fan out into one toast per line, so a
//! three-mesh FBX export stacked four boxes down the corner. The activity card
//! had *already* been pulled out of it for the same class of reason: a running
//! job rewrites its line every few frames and that crate gives a live toast no
//! way to change its caption, so "updating" one cost a slide-out and a slide-in
//! and read as a flicker. Now every card is drawn the same way, and nothing
//! animates at all.
//!
//! **A card is framed like an option window** ([`egui::Frame::window`], which is
//! the same builder `egui::Window` uses — minus its shadow, see [`card_frame`]),
//! so the notices read as part of the same chrome rather than as a second
//! surface style. The one part that is ours is the title row: the title is tinted by [`NoticeKind`] — blue while
//! a job runs, green on success, amber on a warning, red on an error — so the
//! kind is legible before the sentence is read, and a ✕ sits at its right,
//! painted with egui's own two-stroke glyph so it matches the window close
//! button beside it.
//!
//! **Dismissal.** A routine confirmation ([`Notifications::success`] /
//! [`Notifications::info`] / [`Notifications::mode`]) expires on its own after
//! [`motion::NOTIFICATION_EVENT`]; anything the user should read carefully —
//! a warning, an error, or any report carrying body lines — stays until the ✕
//! is clicked. Hovering a card pauses its countdown, so a notice can't expire
//! out from under the pointer that reached for it.
//!
//! **Timing is egui's clock, not `Instant`.** A notice is pushed from `app`
//! outside the egui pass, where there is no context to ask, so a deadline is
//! *resolved* on the first frame the notice is shown, from `ctx.input().time`.
//! That is also what makes expiry testable headlessly: a test drives the clock
//! by handing `RawInput::time` whatever it likes. Each frame the column asks for
//! one [`egui::Context::request_repaint_after`] at the soonest deadline, so the
//! on-demand loop (invariant 6) wakes when a notice expires and at no other
//! time — a sticky card costs nothing to leave up.
//!
//! Per invariant 2 the UI never drives work: `app` owns this, calls
//! [`Notifications::begin_activity`] / [`Notifications::end_activity`] around a
//! background job, pushes the result, and calls [`Notifications::show`] once per
//! frame inside the egui pass. Layout and colour come from the central [`theme`]
//! (invariant 8).
//!
//! [`theme`]: crate::theme

use std::time::Duration;

use egui::{Align, Align2, Color32, Context, Id, Layout, Order, Rect, Sense, TextWrapMode, vec2};

use crate::state::ChromeInsets;
use crate::theme::{color, motion, size};

/// What a notice is telling the user, which is the only thing that varies
/// between cards: it picks the title's colour and the default dismissal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoticeKind {
    /// A background job is running. Only the activity card uses this; it is
    /// never dismissible, since the job's own end is what takes it down.
    Progress,
    /// Something worth saying that needs no action.
    Info,
    /// Something finished and did what was asked.
    Success,
    /// The work went through, but not entirely as asked — a level written as
    /// triangles, a watcher that couldn't start, overrides that were dropped.
    Warning,
    /// The work did not happen.
    Error,
}

impl NoticeKind {
    /// The title tint for this kind.
    fn color(self) -> Color32 {
        match self {
            Self::Progress => color::NOTICE_PROGRESS,
            Self::Info => color::NOTICE_INFO,
            Self::Success => color::NOTICE_SUCCESS,
            Self::Warning => color::NOTICE_WARNING,
            Self::Error => color::NOTICE_ERROR,
        }
    }

    /// Whether a plain one-line notice of this kind expires on its own. A
    /// warning or an error is something the user should act on, so it waits for
    /// the ✕ (a report carrying body lines is sticky whatever its kind — see
    /// [`Notifications::report`]).
    fn auto_dismisses(self) -> bool {
        matches!(self, Self::Info | Self::Success)
    }
}

/// When a notice goes away.
enum Dismiss {
    /// After `remaining` of *shown* time. `deadline` is the egui timestamp it
    /// expires at, resolved on the first frame the card is drawn — a notice
    /// pushed while the window is idle must not have its life spent before it
    /// has been on screen — and pushed forward again while the pointer is over
    /// the card.
    After {
        remaining: Duration,
        deadline: Option<f64>,
    },
    /// Stays until the user clicks the ✕.
    Sticky,
}

/// One card in the column.
struct Notice {
    /// Identity for the ✕, since the vector's indices shift as notices expire.
    id: u64,
    kind: NoticeKind,
    /// The headline, tinted by `kind`. Truncated rather than wrapped: a second
    /// title line would shift everything below it.
    title: String,
    /// Detail under the title, one wrapped line each. Empty for a plain notice.
    lines: Vec<String>,
    dismiss: Dismiss,
    /// A slot name. A keyed push *replaces* the notice already holding the key
    /// rather than stacking beside it, so rapid mode switching rewrites one card
    /// in place and a device fault that repeats every frame can't bury the
    /// viewport. `None` for the ordinary case, where every push is its own card.
    key: Option<&'static str>,
}

/// The app-wide notification collector. Owned by `app`; shown once per frame.
pub struct Notifications {
    /// The live cards, oldest first — which is the order they are drawn in, so
    /// the newest sits nearest the status bar, closest to where the eye was.
    notices: Vec<Notice>,
    /// Source of [`Notice::id`].
    next_id: u64,
    /// What the progress card says, or `None` while nothing is running. It is
    /// drawn above the notices rather than among them: it is the one card that
    /// is *about the future*, and it must not shuffle down the column as
    /// results land beneath it.
    activity: Option<Activity>,
    /// In-flight background jobs. The progress card shows while this is `> 0`
    /// and is dismissed when it returns to `0`, so overlapping jobs share one
    /// indicator.
    active: usize,
    /// The egui timestamp of the previous [`Notifications::show`], so a hovered
    /// card's deadline can be pushed forward by exactly the elapsed time.
    last_now: f64,
}

/// What the progress card is showing.
struct Activity {
    /// The job, named once when it starts (`Loading big.fbx…`). Steady for the
    /// job's whole life, so the card always says *what* is running even while
    /// the line under it changes.
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
    /// Create the collector.
    pub fn new() -> Self {
        Self {
            notices: Vec::new(),
            next_id: 0,
            activity: None,
            active: 0,
            last_now: 0.0,
        }
    }

    /// Push a transient success notice (e.g. a model or texture finished
    /// loading). Expires on its own.
    pub fn success(&mut self, title: impl Into<String>) {
        self.push(NoticeKind::Success, title.into(), Vec::new(), None);
    }

    /// Push a transient info notice. Expires on its own.
    pub fn info(&mut self, title: impl Into<String>) {
        self.push(NoticeKind::Info, title.into(), Vec::new(), None);
    }

    /// Push a warning: the work went through, but not entirely as asked. Stays
    /// until dismissed — the user is meant to read it, and it is usually the
    /// only sign that something silently differs from what they expect.
    pub fn warning(&mut self, title: impl Into<String>) {
        self.push(NoticeKind::Warning, title.into(), Vec::new(), None);
    }

    /// Push an error notice. Stays until dismissed.
    pub fn error(&mut self, title: impl Into<String>) {
        self.push(NoticeKind::Error, title.into(), Vec::new(), None);
    }

    /// Push an error into a named slot, replacing whatever error already holds
    /// it. For a failure that *repeats* — a wedged device reports itself every
    /// frame — so the viewport gets one card rather than a new one per frame.
    pub fn error_keyed(&mut self, key: &'static str, title: impl Into<String>) {
        self.push(NoticeKind::Error, title.into(), Vec::new(), Some(key));
    }

    /// Push one notice carrying a body: a headline plus a detail line each for
    /// however many things the operation has to say. This is what keeps a
    /// multi-part result — an export's per-mesh notes, a run's warnings — to a
    /// single card instead of one per line.
    ///
    /// A report **with** lines is always sticky whatever its kind: the lines are
    /// there because the user has to read them, and a success big enough to have
    /// a body is no longer a routine confirmation. With no lines it behaves
    /// exactly like the matching one-line push.
    pub fn report(&mut self, kind: NoticeKind, title: impl Into<String>, lines: Vec<String>) {
        self.push(kind, title.into(), lines, None);
    }

    /// Show the current view mode (e.g. the material mode name), **replacing**
    /// any mode notice still on screen. Keyed to one slot, so rapid mode
    /// switching rewrites one card in place instead of stacking a card per
    /// switch.
    pub fn mode(&mut self, title: impl Into<String>) {
        self.push(NoticeKind::Info, title.into(), Vec::new(), Some(MODE_KEY));
    }

    /// Build a notice and either replace its keyed slot or append it.
    fn push(
        &mut self,
        kind: NoticeKind,
        title: String,
        lines: Vec<String>,
        key: Option<&'static str>,
    ) {
        // A body means the card waits to be read; otherwise the kind decides.
        let auto = lines.is_empty() && kind.auto_dismisses();
        let notice = Notice {
            id: self.next_id,
            kind,
            title,
            lines,
            dismiss: if auto {
                Dismiss::After {
                    remaining: motion::NOTIFICATION_EVENT,
                    deadline: None,
                }
            } else {
                Dismiss::Sticky
            },
            key,
        };
        self.next_id += 1;
        match key.and_then(|key| self.notices.iter().position(|n| n.key == Some(key))) {
            // In place, so the replacement doesn't jump to the end of the column
            // — the point of a slot is that the card stays where the user last
            // read it.
            Some(index) => self.notices[index] = notice,
            None => self.notices.push(notice),
        }
        // Drop the oldest past the cap here rather than hiding them at draw
        // time: a card that isn't drawn is one the user can't dismiss, and a
        // sticky one would then sit in the queue forever waiting for a click it
        // can never receive.
        let overflow = self
            .notices
            .len()
            .saturating_sub(size::NOTIFICATION_MAX_VISIBLE);
        self.notices.drain(..overflow);
    }

    /// Mark the start of a background job, raising the progress card on the
    /// first concurrent job. Pair every call with
    /// [`Notifications::end_activity`].
    ///
    /// `title` names the job; what it is *doing* goes to
    /// [`Notifications::update_activity`].
    ///
    /// Overlapping jobs share one card and the newest one names it, since it is
    /// also the one whose reports are arriving.
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
    /// This rewrites the card in place — nothing moves — so `app` may call it as
    /// often as it has something new to say. A no-op when nothing is running, so
    /// a report that arrives after its job ended can't raise a card.
    pub fn update_activity(&mut self, detail: impl Into<String>, fraction: Option<f32>) {
        if let Some(activity) = self.activity.as_mut() {
            activity.detail = detail.into();
            activity.fraction = fraction.map(|value| value.clamp(0.0, 1.0));
        }
    }

    /// Mark a background job finished; hides the progress card once all are done.
    pub fn end_activity(&mut self) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.activity = None;
        }
    }

    /// Draw the column. Call once per frame inside the egui pass.
    ///
    /// `insets` are the open side panels' widths, so the column centres on the
    /// free viewport rather than on the window — the same correction the stats
    /// cards make. It asks for exactly one delayed repaint, at the soonest
    /// deadline among the auto-dismissing cards, so the on-demand loop
    /// (invariant 6) never spins for a card that is merely *up*.
    pub fn show(&mut self, ctx: &Context, insets: ChromeInsets) {
        let now = ctx.input(|input| input.time);
        let elapsed = (now - self.last_now).max(0.0);
        self.last_now = now;

        if self.notices.is_empty() && self.activity.is_none() {
            return;
        }

        let mut dismissed: Option<u64> = None;
        let mut hovered: Option<u64> = None;
        // Anchored, so this Area is not movable; left interactable (the default)
        // so a click on a card — the ✕, or just a stray one — is consumed here
        // rather than falling through to the viewport behind it.
        egui::Area::new(Id::new("notifications"))
            .order(Order::Foreground)
            // Persistent chrome, not a transient popup: no fade, for the same
            // reason the option windows have none (invariant 6).
            .fade_in(false)
            .anchor(
                Align2::CENTER_BOTTOM,
                vec2(
                    (insets.left - insets.right) * 0.5,
                    -size::NOTIFICATION_MARGIN_Y,
                ),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = size::NOTIFICATION_CARD_GAP;
                if let Some(activity) = self.activity.as_ref() {
                    activity_card(ui, activity);
                }
                for notice in &self.notices {
                    let card = notice_card(ui, notice);
                    if card.dismissed {
                        dismissed = Some(notice.id);
                    }
                    if card.hovered {
                        hovered = Some(notice.id);
                    }
                }
            });

        if let Some(id) = dismissed {
            self.notices.retain(|notice| notice.id != id);
        }

        // Resolve pending deadlines, push the hovered card's forward, and drop
        // whatever has expired. Done after the draw so a card always gets at
        // least one frame on screen, however long the loop was idle before it.
        let mut soonest: Option<f64> = None;
        self.notices.retain_mut(|notice| {
            let Dismiss::After {
                remaining,
                deadline,
            } = &mut notice.dismiss
            else {
                return true;
            };
            let due = match deadline {
                Some(due) => {
                    if hovered == Some(notice.id) {
                        *due += elapsed;
                    }
                    *due
                }
                None => *deadline.insert(now + remaining.as_secs_f64()),
            };
            if due <= now {
                return false;
            }
            soonest = Some(soonest.map_or(due, |best: f64| best.min(due)));
            true
        });

        if let Some(due) = soonest {
            ctx.request_repaint_after(Duration::from_secs_f64((due - now).max(0.0)));
        }
    }
}

/// The slot every mode notice shares.
const MODE_KEY: &str = "mode";

/// A card's frame: an option window's, minus its shadow.
///
/// [`egui::Frame::window`] is the same builder `egui::Window` uses, so the fill,
/// stroke, corner radius and padding track whatever the option windows do and a
/// notice never drifts out of step with the chrome beside it. The shadow is the
/// one part that doesn't carry: egui's is offset 20pt *downward* with a 15pt
/// blur, which is right for a single window floating over the viewport and wrong
/// for a column — every card would cast onto the card below it, and the bottom
/// one onto the status bar 8pt under it. The fill and the stroke already
/// separate a card from the model behind it.
fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::window(ui.style()).shadow(egui::epaint::Shadow::NONE)
}

/// What a drawn card reports back to [`Notifications::show`].
struct CardResponse {
    /// The ✕ was clicked this frame.
    dismissed: bool,
    /// The pointer is over the card, so its countdown is paused.
    hovered: bool,
}

/// Draw one notice: the tinted title, its ✕, and the body lines under them.
fn notice_card(ui: &mut egui::Ui, notice: &Notice) -> CardResponse {
    let mut dismissed = false;
    let frame = card_frame(ui).show(ui, |ui| {
        ui.set_width(size::NOTIFICATION_WIDTH);
        ui.spacing_mut().item_spacing.y = size::NOTIFICATION_LINE_GAP;
        dismissed = title_row(ui, notice.kind, &notice.title, true);
        let shown = notice.lines.len().min(size::NOTIFICATION_MAX_LINES);
        for line in &notice.lines[..shown] {
            ui.label(egui::RichText::new(line).color(color::TEXT_BODY));
        }
        // The column grows upward from the status bar, so a card long enough to
        // run its own title off the top of the window says how much it is
        // holding back instead.
        let hidden = notice.lines.len() - shown;
        if hidden > 0 {
            ui.label(egui::RichText::new(format!("+ {hidden} more")).color(color::TEXT_MUTED));
        }
    });
    CardResponse {
        dismissed,
        // `contains_pointer` rather than `hovered`: egui reports nothing as
        // hovered while any widget is being dragged, and a scrub on the
        // transport below would otherwise un-pause every card on screen.
        hovered: frame.response.contains_pointer(),
    }
}

/// Draw the progress card: the job's title, the line under it, and a bar when
/// the job knows its own denominator. No ✕ — the job's own end takes it down.
fn activity_card(ui: &mut egui::Ui, activity: &Activity) {
    card_frame(ui).show(ui, |ui| {
        ui.set_width(size::NOTIFICATION_WIDTH);
        ui.spacing_mut().item_spacing.y = size::NOTIFICATION_LINE_GAP;
        title_row(ui, NoticeKind::Progress, &activity.title, false);
        if !activity.detail.is_empty() {
            ui.label(egui::RichText::new(&activity.detail).color(color::TEXT_MUTED));
        }
        if let Some(fraction) = activity.fraction {
            progress_bar(ui, fraction);
        }
    });
}

/// A card's header: the title in the heading style, tinted by kind, with the
/// close glyph pinned to the right when the card is dismissible. Returns whether
/// the ✕ was clicked.
///
/// Laid out right-to-left so the ✕ is placed first and the title takes whatever
/// is left — the title truncates rather than wraps, since a second line would
/// shift every body line under it.
fn title_row(ui: &mut egui::Ui, kind: NoticeKind, title: &str, closable: bool) -> bool {
    let mut dismissed = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if closable {
            dismissed = close_button(ui).clicked();
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
            ui.label(
                egui::RichText::new(title)
                    .text_style(egui::TextStyle::Heading)
                    .color(kind.color()),
            );
        });
    });
    dismissed
}

/// The card's ✕. Painted as egui's own window close button is — two line
/// segments in a square of `spacing.icon_width`, stroked with the interact
/// foreground so it brightens on hover — so the glyph matches the X on the
/// option windows beside it rather than approximating one with a text rune.
fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let size = egui::Vec2::splat(ui.spacing().icon_width);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let rect = rect.shrink(2.0).expand(visuals.expansion);
        let stroke = visuals.fg_stroke;
        let painter = ui.painter();
        painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
        painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
    }
    response.on_hover_text("Dismiss")
}

/// A slim filled track, the width of the card. Not `egui::ProgressBar`: that one
/// sizes itself to the available width *and* reserves a text row, which would
/// change the card's height the moment a fraction became known.
fn progress_bar(ui: &mut egui::Ui, fraction: f32) {
    let (track, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), size::NOTIFICATION_PROGRESS_HEIGHT),
        Sense::hover(),
    );
    let radius = egui::CornerRadius::same(size::NOTIFICATION_PROGRESS_RADIUS);
    ui.painter()
        .rect_filled(track, radius, color::NOTICE_PROGRESS_TRACK);
    let filled = Rect::from_min_size(
        track.min,
        vec2(track.width() * fraction.clamp(0.0, 1.0), track.height()),
    );
    ui.painter().rect_filled(filled, radius, color::ACCENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one headless egui pass at egui time `time`, showing the column.
    ///
    /// The clock is the whole point: a deadline resolves and expires against
    /// `RawInput::time`, so a test drives seconds of wall time in three calls
    /// without sleeping. Textures are cleared because the pass loads the font
    /// atlas and the delta is of no interest here.
    fn pass(ctx: &egui::Context, notifications: &mut Notifications, time: f64) {
        ctx.begin_pass(egui::RawInput {
            time: Some(time),
            ..Default::default()
        });
        notifications.show(ctx, ChromeInsets::default());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::init_style(&ctx);
        ctx
    }

    #[test]
    fn an_auto_notice_expires_and_a_sticky_one_does_not() {
        let ctx = context();
        let mut notifications = Notifications::new();
        notifications.success("Loaded thing.fbx");
        notifications.error("Couldn't load other.fbx");

        // The deadline resolves on the first frame the card is drawn, not when
        // it was pushed — so an idle loop can't spend a notice's life for it.
        pass(&ctx, &mut notifications, 0.0);
        assert_eq!(notifications.notices.len(), 2);
        pass(&ctx, &mut notifications, 0.5);
        assert_eq!(notifications.notices.len(), 2);

        // Past the success's deadline: it goes, the error stays put.
        pass(
            &ctx,
            &mut notifications,
            motion::NOTIFICATION_EVENT.as_secs_f64() + 0.5,
        );
        assert_eq!(notifications.notices.len(), 1);
        assert_eq!(notifications.notices[0].kind, NoticeKind::Error);

        pass(&ctx, &mut notifications, 1_000.0);
        assert_eq!(notifications.notices.len(), 1);
    }

    #[test]
    fn a_report_with_lines_is_one_sticky_card() {
        let ctx = context();
        let mut notifications = Notifications::new();
        notifications.report(
            NoticeKind::Success,
            "Exported dog.fbx (5139 triangles)",
            vec![
                "LOD 0: 'Body' was written as triangles.".to_owned(),
                "LOD 0: 'Eyes' was written as triangles.".to_owned(),
            ],
        );

        pass(&ctx, &mut notifications, 0.0);
        // One card, not one per line — which is the whole reason `report` exists.
        assert_eq!(notifications.notices.len(), 1);
        assert_eq!(notifications.notices[0].lines.len(), 2);

        // A success *with* a body waits to be read, whatever its kind's default.
        pass(&ctx, &mut notifications, 1_000.0);
        assert_eq!(notifications.notices.len(), 1);
    }

    #[test]
    fn a_keyed_push_replaces_in_place() {
        let ctx = context();
        let mut notifications = Notifications::new();
        notifications.info("first");
        notifications.mode("Shaded");
        notifications.mode("Unlit");
        notifications.mode("Wireframe");

        pass(&ctx, &mut notifications, 0.0);
        // The three mode pushes share one slot, and it stayed where it was
        // rather than jumping past the plain notice pushed before it.
        assert_eq!(notifications.notices.len(), 2);
        assert_eq!(notifications.notices[1].title, "Wireframe");
    }

    /// The column is capped, and the cap drops the *oldest* — the newest cards
    /// are the ones being read. Dropping happens on push, not at draw time: a
    /// card that isn't drawn has no ✕, so a sticky one hidden behind the cap
    /// would wait forever for a click it could never receive.
    #[test]
    fn the_column_keeps_only_the_newest_cards() {
        let ctx = context();
        let mut notifications = Notifications::new();
        for index in 0..size::NOTIFICATION_MAX_VISIBLE + 3 {
            notifications.error(format!("error {index}"));
        }
        pass(&ctx, &mut notifications, 0.0);

        assert_eq!(notifications.notices.len(), size::NOTIFICATION_MAX_VISIBLE);
        assert_eq!(
            notifications.notices.last().map(|n| n.title.as_str()),
            Some(format!("error {}", size::NOTIFICATION_MAX_VISIBLE + 2).as_str())
        );
        assert_eq!(
            notifications.notices.first().map(|n| n.title.as_str()),
            Some("error 3")
        );
    }

    #[test]
    fn overlapping_jobs_share_one_progress_card() {
        let ctx = context();
        let mut notifications = Notifications::new();
        notifications.begin_activity("Loading a.fbx");
        notifications.begin_activity("Decoding b.png");
        pass(&ctx, &mut notifications, 0.0);
        // The newest names it: it is the one whose reports are arriving.
        assert_eq!(
            notifications.activity.as_ref().map(|a| a.title.as_str()),
            Some("Decoding b.png")
        );

        notifications.end_activity();
        assert!(notifications.activity.is_some());
        notifications.end_activity();
        assert!(notifications.activity.is_none());

        // An update after the last job ended can't raise a card.
        notifications.update_activity("late", None);
        assert!(notifications.activity.is_none());
    }
}
