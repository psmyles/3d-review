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
//! **A card is built out of egui's own window styling, not a look of our own.**
//! The frame is [`egui::Frame::window`], the same builder `egui::Window` uses
//! (minus its shadow and its padding — see [`card_frame`]); the header and body
//! pad themselves with `spacing.window_margin`, exactly as that window's title
//! bar and content do; the header row is one `TextStyle::Heading` tall, which is
//! what egui allocates for its own title bar; the divider takes both its colour
//! and its thickness from `window_stroke`, the border it meets at either end;
//! the ✕ is egui's own two-stroke glyph; the body text is `text_color` and
//! `weak_text_color`; the progress bar is [`egui::ProgressBar`], slimmed. The
//! kind tints are egui's `error_fg_color`, `warn_fg_color`, `hyperlink_color`
//! and `strong_text_color` — every one but success, because egui's palette has
//! no green. So a notice follows any restyle of the chrome for free, and there
//! is almost nothing here for the two to drift apart on.
//!
//! **A card is a header over a body.** The header says what kind of thing this
//! is and nothing more — `Warning`, `Success`, `Error`, `Info`, or `Working`
//! while a job runs — in that kind's colour, with egui's own two-stroke ✕ at its
//! right; then a hairline edge to edge, and under it the message, wrapped across
//! as many lines as it takes. The message used to *be* the header, truncated to
//! one line, which is the one thing a notification cannot afford to lose: a card
//! reading `Couldn't load pedestal.fbx: unexpected en…` has cut exactly the half
//! that says what went wrong. A kind is a word, so it always fits; a message is a
//! sentence, so it gets room. The **mode** notice is the one exception and stays
//! a single bare line — see [`Notice::compact`].
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

use egui::{Align, Align2, Color32, Context, Id, Layout, Order, Sense, TextWrapMode, vec2};

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
    /// What a card's header calls this kind. The header says only this, so the
    /// word has to carry the whole classification on its own — which is why a
    /// running job reads "Working" rather than "Progress": the header names what
    /// is happening, not what the variant is called.
    fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Progress => review_localization::tr(crate::keys::ui_notices::WORKING),
            Self::Info => review_localization::tr(crate::keys::ui_notices::INFO),
            Self::Success => review_localization::tr(crate::keys::ui_notices::SUCCESS),
            Self::Warning => review_localization::tr(crate::keys::ui_notices::WARNING),
            Self::Error => review_localization::tr(crate::keys::ui_notices::ERROR),
        }
    }

    /// The header tint for this kind, taken from egui's own visuals wherever it
    /// has a word for the thing.
    ///
    /// `error_fg_color` and `warn_fg_color` exist precisely for this and are
    /// what egui tints its own error and warning text with, so a notice matches
    /// the rest of the chrome for free and follows any restyle of it.
    /// `hyperlink_color` is the one blue in the palette meant to be *read* as
    /// text rather than filled behind it, which is what a running job wants,
    /// and `strong_text_color` is the plain emphasis egui already has.
    ///
    /// Success is the single exception: egui's palette carries no green, so
    /// that one word is ours ([`color::NOTICE_SUCCESS`]).
    fn color(self, visuals: &egui::Visuals) -> Color32 {
        match self {
            Self::Progress => visuals.hyperlink_color,
            Self::Info => visuals.strong_text_color(),
            Self::Success => color::NOTICE_SUCCESS,
            Self::Warning => visuals.warn_fg_color,
            Self::Error => visuals.error_fg_color,
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

/// The target every notice is logged under, so its line is tagged `[notice]`.
const NOTICE_TARGET: &str = "review_notice";

/// Write a card to the log as well, so the log is a history of what the viewer
/// *said* as well as what it did. In the words the card shows - the catalog's,
/// not English - because those are what a user reporting it will quote; the
/// line `app` logs beside most of them carries the diagnostic detail. A mode
/// notice is Debug: it names a view the user just switched to themselves.
fn log_notice(notice: &Notice) {
    let mut text = notice.message.clone();
    for line in &notice.lines {
        text.push('\n');
        text.push_str(line);
    }
    let level = if notice.compact {
        log::Level::Debug
    } else {
        match notice.kind {
            NoticeKind::Error => log::Level::Error,
            NoticeKind::Warning => log::Level::Warn,
            NoticeKind::Progress | NoticeKind::Info | NoticeKind::Success => log::Level::Info,
        }
    };
    log::log!(target: NOTICE_TARGET, level, "{text}");
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
    /// What the notice actually says, wrapped across as many lines as it needs.
    /// A card's header carries only the kind, so this is the whole of the
    /// message and none of it may be cut.
    message: String,
    /// Further detail under the message, one wrapped line each — an export's
    /// per-mesh notes, a run's warnings. Empty for a plain notice.
    lines: Vec<String>,
    /// Draw as a single line with no header and no divider.
    ///
    /// Only [`Notifications::mode`] sets this. A mode notice is one word naming
    /// the view you just switched to, it is gone in two seconds, and you asked
    /// for it by pressing the key — a `Info` header over a divider over the word
    /// `Unlit` is three times the furniture for none of the information. Every
    /// other notice reports something you did not ask for and has to say what
    /// kind of thing it is.
    compact: bool,
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
    pub fn success(&mut self, message: impl Into<String>) {
        self.push(NoticeKind::Success, message.into(), Vec::new(), None);
    }

    /// Push a transient info notice. Expires on its own.
    pub fn info(&mut self, message: impl Into<String>) {
        self.push(NoticeKind::Info, message.into(), Vec::new(), None);
    }

    /// Push a warning: the work went through, but not entirely as asked. Stays
    /// until dismissed — the user is meant to read it, and it is usually the
    /// only sign that something silently differs from what they expect.
    pub fn warning(&mut self, message: impl Into<String>) {
        self.push(NoticeKind::Warning, message.into(), Vec::new(), None);
    }

    /// Push an error notice. Stays until dismissed.
    pub fn error(&mut self, message: impl Into<String>) {
        self.push(NoticeKind::Error, message.into(), Vec::new(), None);
    }

    /// Push an error into a named slot, replacing whatever error already holds
    /// it. For a failure that *repeats* — a wedged device reports itself every
    /// frame — so the viewport gets one card rather than a new one per frame.
    pub fn error_keyed(&mut self, key: &'static str, message: impl Into<String>) {
        self.push(NoticeKind::Error, message.into(), Vec::new(), Some(key));
    }

    /// Push one notice carrying extra detail: the message, plus a line each for
    /// however many things the operation has to say. This is what keeps a
    /// multi-part result — an export's per-mesh notes, a run's warnings — to a
    /// single card instead of one per line.
    ///
    /// A report **with** lines is always sticky whatever its kind: the lines are
    /// there because the user has to read them, and a success big enough to have
    /// a body is no longer a routine confirmation. With no lines it behaves
    /// exactly like the matching one-line push.
    pub fn report(&mut self, kind: NoticeKind, message: impl Into<String>, lines: Vec<String>) {
        self.push(kind, message.into(), lines, None);
    }

    /// Show the current view mode (e.g. the material mode name), **replacing**
    /// any mode notice still on screen. Keyed to one slot, so rapid mode
    /// switching rewrites one card in place instead of stacking a card per
    /// switch.
    pub fn mode(&mut self, message: impl Into<String>) {
        let notice = self.build(NoticeKind::Info, message.into(), Vec::new(), Some(MODE_KEY));
        self.place(Notice {
            compact: true,
            ..notice
        });
    }

    /// Build a notice and either replace its keyed slot or append it.
    fn push(
        &mut self,
        kind: NoticeKind,
        message: String,
        lines: Vec<String>,
        key: Option<&'static str>,
    ) {
        let notice = self.build(kind, message, lines, key);
        self.place(notice);
    }

    /// Assemble a notice and claim its id, without showing it yet.
    fn build(
        &mut self,
        kind: NoticeKind,
        message: String,
        lines: Vec<String>,
        key: Option<&'static str>,
    ) -> Notice {
        // A body means the card waits to be read; otherwise the kind decides.
        let auto = lines.is_empty() && kind.auto_dismisses();
        let notice = Notice {
            id: self.next_id,
            kind,
            message,
            lines,
            compact: false,
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
        notice
    }

    /// Put a built notice in the column, replacing its keyed slot if it has one.
    fn place(&mut self, notice: Notice) {
        let key = notice.key;
        let slot = key.and_then(|key| self.notices.iter().position(|n| n.key == Some(key)));
        // A slot rewritten with the words it already shows is the same notice
        // again - a fault repeating every frame - and is logged the first time.
        if slot.is_none_or(|index| self.notices[index].message != notice.message) {
            log_notice(&notice);
        }
        match slot {
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
        let title = title.into();
        log::debug!(target: NOTICE_TARGET, "working: {title}");
        self.activity = Some(Activity {
            title,
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

        // A card wants `NOTIFICATION_WIDTH` for its text and settles for what
        // the free viewport can give: the window, less the docked side panels,
        // less the margin the column keeps from their edges, less the card's own
        // padding. Measured here rather than inside the card because every card
        // in the column must come out the same width.
        let padding = ctx.global_style().spacing.window_margin.sum().x;
        let free = ctx.content_rect().width()
            - insets.left
            - insets.right
            - size::OVERLAY_MARGIN * 2.0
            - padding;
        let text_width = size::NOTIFICATION_WIDTH
            .min(free)
            .max(size::NOTIFICATION_MIN_WIDTH);

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
            // Pin the cards to the top of the Area's rect. An `Area` defaults to
            // `Layout::default()`, which is `top_down(Align::LEFT)` and so
            // carries `main_align: Center` — the column would be centred
            // *vertically* inside a rect that is its own previous measured size.
            // That closes a loop between what the Area measures and where it
            // then lays out, which is what let one oversized card (see
            // [`title_row`]) stay oversized for the rest of the session. With
            // `Align::Min` the measured size is just the cards.
            .layout(Layout::top_down(Align::Min).with_main_align(Align::Min))
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
                    activity_card(ui, activity, text_width);
                }
                for notice in &self.notices {
                    let card = notice_card(ui, notice, text_width);
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

/// A card's frame: an option window's, minus its shadow and its padding.
///
/// [`egui::Frame::window`] is the same builder `egui::Window` uses, so the fill,
/// stroke and corner radius track whatever the option windows do and a notice
/// never drifts out of step with the chrome beside it. Two parts don't carry:
///
/// The **shadow**, because egui's is offset 20pt downward with a 15pt blur —
/// right for a single window floating over the viewport, wrong for a column,
/// where every card would cast onto the card below it and the bottom one onto
/// the status bar 8pt under it. The fill and the stroke already separate a card
/// from the model behind it.
///
/// The **inner margin**, because the divider under the header runs edge to edge.
/// A frame that padded its content would inset the divider with it; instead the
/// padding is applied to the header and the body separately ([`card_pad`]), so
/// the line between them spans the full card.
fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::window(ui.style())
        .shadow(egui::epaint::Shadow::NONE)
        .inner_margin(0)
}

/// The padding the header and the body each carry, which is the padding
/// `Frame::window` would have applied to the card as a whole — taken from the
/// same style value, so a card is inset exactly like an option window even
/// though its own frame has no margin.
fn card_pad(ui: &egui::Ui) -> egui::Margin {
    ui.style().spacing.window_margin
}

/// Set a card's outer width from its text column, and butt its blocks together.
///
/// The zero spacing is the whole reason this is a helper. The column's `Area`
/// sets `item_spacing.y` to the gap *between cards*, and a `Frame`'s content
/// inherits the style it was shown into — so that gap was also being inserted
/// between the header, the divider and the body, putting two card-gaps of dead
/// space above every message. Each block already carries its own padding
/// ([`card_pad`]), exactly as `egui::Window`'s title bar and body do, so the
/// spacing between them must be none.
fn begin_card(ui: &mut egui::Ui, text_width: f32) {
    ui.set_width(text_width + card_pad(ui).sum().x);
    ui.spacing_mut().item_spacing.y = 0.0;
}

/// What a drawn card reports back to [`Notifications::show`].
struct CardResponse {
    /// The ✕ was clicked this frame.
    dismissed: bool,
    /// The pointer is over the card, so its countdown is paused.
    hovered: bool,
}

/// Draw one notice: the kind in the header, then the message and any detail
/// lines in the body under it.
fn notice_card(ui: &mut egui::Ui, notice: &Notice, text_width: f32) -> CardResponse {
    let mut dismissed = false;
    let frame = card_frame(ui).show(ui, |ui| {
        begin_card(ui, text_width);
        if notice.compact {
            dismissed = compact_row(ui, notice);
            return;
        }
        dismissed = header(ui, notice.kind, true);
        body(ui, |ui| {
            // The message itself, wrapped. This is the part the user is here to
            // read, so it gets the room to be read in. It was the card's
            // *header* once, truncated to a single line, which showed half of
            // "Couldn't load pedestal.fbx: unexpected end of file" and cut the
            // half that says what went wrong.
            ui.label(&notice.message);
            let shown = notice.lines.len().min(size::NOTIFICATION_MAX_LINES);
            for line in &notice.lines[..shown] {
                ui.label(line);
            }
            // The column grows upward from the status bar, so a card long enough
            // to run its own header off the top of the window says how much it
            // is holding back instead.
            let hidden = notice.lines.len() - shown;
            if hidden > 0 {
                let weak = ui.visuals().weak_text_color();
                ui.label(
                    egui::RichText::new(crate::keys::ui_notices::more(hidden as f64)).color(weak),
                );
            }
        });
    });
    CardResponse {
        dismissed,
        // `contains_pointer` rather than `hovered`: egui reports nothing as
        // hovered while any widget is being dragged, and a scrub on the
        // transport below would otherwise un-pause every card on screen.
        hovered: frame.response.contains_pointer(),
    }
}

/// Draw the progress card: the job in the body, the stage line under it, and a
/// bar when the job knows its own denominator. No ✕ — the job's own end takes it
/// down.
fn activity_card(ui: &mut egui::Ui, activity: &Activity, text_width: f32) {
    card_frame(ui).show(ui, |ui| {
        begin_card(ui, text_width);
        header(ui, NoticeKind::Progress, false);
        body(ui, |ui| {
            ui.label(&activity.title);
            if !activity.detail.is_empty() {
                let weak = ui.visuals().weak_text_color();
                ui.label(egui::RichText::new(&activity.detail).color(weak));
            }
            if let Some(fraction) = activity.fraction {
                // egui's own progress bar, slimmed: its default height is a full
                // interactive row, which is a lot of furniture inside a card. It
                // brings its own fill and track from the visuals.
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_height(size::NOTIFICATION_PROGRESS_HEIGHT),
                );
            }
        });
    });
}

/// A card's header: what kind of thing this is, in that kind's colour, with the
/// close glyph pinned to the right when the card is dismissible — then a
/// hairline across the full width of the card. Returns whether the ✕ was
/// clicked.
///
/// The header names the *kind* and nothing else. It used to carry the message,
/// truncated to a single line, which is the one thing a notification cannot
/// afford to lose; the message moved to [`body`], where it wraps.
///
/// **The row is allocated at an explicit size, and that is what keeps the card
/// the size of its content.** A bare `with_layout` inherits the parent's whole
/// available rect — `Ui::horizontal` exists precisely because it does *not*,
/// capping the row to one interactive height first — and with the cross
/// alignment centred, the row then grew to fill every point the Area had to
/// give. The card frame grew with it, so a one-line notice was painted as a
/// strip the height of the window. Worse, it latched: an `egui::Area` lays each
/// frame out inside its *previous* measured size, so once a card had claimed
/// the height it kept claiming it, and every load pumped it further.
fn header(ui: &mut egui::Ui, kind: NoticeKind, closable: bool) -> bool {
    let mut dismissed = false;
    egui::Frame::NONE.inner_margin(card_pad(ui)).show(ui, |ui| {
        // One heading row, which is exactly what `egui::Window` allocates for
        // its own title bar — it sizes the collapse and close buttons to the
        // heading's row height and paints the smaller glyph inside. Taking the
        // interact height instead (which is larger) made the header visibly
        // deeper than the option window's beside it.
        let row = vec2(
            ui.available_width(),
            ui.text_style_height(&egui::TextStyle::Heading),
        );
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            if closable {
                dismissed = close_button(ui).clicked();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let tint = kind.color(ui.visuals());
                ui.label(
                    egui::RichText::new(kind.label())
                        .text_style(egui::TextStyle::Heading)
                        .color(tint),
                );
            });
        });
    });
    // Edge to edge, which is why the card's own frame carries no margin. Both
    // the colour *and* the thickness come from the frame's own stroke — the same
    // `window_stroke` that draws an option window's border — so the divider
    // reads as part of the border it meets at either end rather than as a rule
    // someone added on top of it.
    let stroke = ui.visuals().window_stroke();
    let (rule, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), stroke.width), Sense::hover());
    ui.painter().rect_filled(rule, 0.0, stroke.color);
    dismissed
}

/// A mode notice: one padded line, the message and its ✕, with no header and no
/// divider over it. Returns whether the ✕ was clicked.
///
/// This is the shape every card used to have, kept for the one case it suits.
/// A mode notice names the view you just switched into, in a word, because you
/// pressed the key that switched it — so it has nothing to classify and nothing
/// to explain, and a header saying `Info` over a rule over the word `Unlit`
/// would be furniture around a label. The message truncates here rather than
/// wrapping, which is safe for exactly this reason: the text is a mode name, not
/// a sentence, and `app` is the only caller.
fn compact_row(ui: &mut egui::Ui, notice: &Notice) -> bool {
    let mut dismissed = false;
    egui::Frame::NONE.inner_margin(card_pad(ui)).show(ui, |ui| {
        let height = ui
            .text_style_height(&egui::TextStyle::Body)
            .max(ui.spacing().interact_size.y);
        let row = vec2(ui.available_width(), height);
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            dismissed = close_button(ui).clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                let tint = notice.kind.color(ui.visuals());
                ui.label(egui::RichText::new(&notice.message).color(tint));
            });
        });
    });
    dismissed
}

/// A card's body: the padded area under the divider that holds the message.
fn body(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.inner_margin(card_pad(ui)).show(ui, |ui| {
        // Restore the style's own row spacing, which `begin_card` zeroed for the
        // card's outer stack. Taken from the style rather than from a token of
        // our own, so a notice's lines breathe exactly like an option panel's
        // rows do.
        ui.spacing_mut().item_spacing.y = ui.ctx().global_style().spacing.item_spacing.y;
        // Labels wrap by default in a width-constrained `Ui`; pin it anyway, so
        // a future style that flips the default can't silently start truncating
        // the one text the user is here to read.
        ui.style_mut().wrap_mode = Some(TextWrapMode::Wrap);
        add_contents(ui);
    });
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
    response.on_hover_text(crate::keys::ui_notices::DISMISS)
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
            // A real window, not egui's headless default: the layout bugs this
            // module has had were all "the card took the size of the screen",
            // which a default-sized context cannot show.
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
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
    fn the_column_keeps_only_the_newest_cards() {
        let ctx = context();
        let mut notifications = Notifications::new();
        for index in 0..size::NOTIFICATION_MAX_VISIBLE + 3 {
            notifications.error(format!("error {index}"));
        }
        pass(&ctx, &mut notifications, 0.0);

        assert_eq!(notifications.notices.len(), size::NOTIFICATION_MAX_VISIBLE);
        assert_eq!(
            notifications.notices.last().map(|n| n.message.as_str()),
            Some(format!("error {}", size::NOTIFICATION_MAX_VISIBLE + 2).as_str())
        );
        assert_eq!(
            notifications.notices.first().map(|n| n.message.as_str()),
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
