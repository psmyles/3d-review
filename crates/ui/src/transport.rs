//! The animation transport: the centre group of the status bar, shown only while
//! a clip is selected in the 3D workspace.
//!
//! Go-to-start · step back · play/pause · step forward · a frame scrubber · the
//! "frame / total   seconds" readout · loop · speed. Everything it edits is the
//! plain [`crate::state::AnimationUiState`] the Outliner's Animations tab also
//! writes; `app` runs the clock off it (invariant 6) and paces the redraw loop
//! while it plays.
//!
//! **It used to float over the viewport**, on the same centred overlay card as
//! the Opt legend, at a fixed 610pt that was never clamped to the space
//! available — so on a narrow window it ran straight under the model-stats card
//! beside it, and it sat on top of the very frames it was scrubbing. In the bar
//! it occupies the free span between the two mirrored icon groups (the same slot
//! the Opt workspace puts its comparison controls in, which is why the two are
//! exclusive by workspace) and degrades into it rather than over its neighbours.
//!
//! [`status_bar`]: crate::status_bar

use review_model::AnimationClip;

use crate::assets::{self, AppIcon};
use crate::docs::Page;
use crate::keys;
use crate::state::{PlaybackSpeed, UiState};
use crate::theme::{color, font, size};
use crate::widgets;
use crate::widgets::Tip;

/// How the row's width is spent: what the scrubber gets, and whether the frame
/// readout survives.
struct RailBudget {
    /// The width the scrubber rail ends up with.
    rail: f32,
    /// Whether the `frame / total   seconds` readout is drawn.
    readout_shown: bool,
}

/// Split a row's available width between the scrubber and the frame readout.
///
/// **What gives first when the bar is narrow**: the rail shrinks to
/// [`size::ANIM_SCRUB_MIN_WIDTH`], and past that the readout is dropped and its
/// block handed to the rail. The buttons, the loop toggle and the speed picker
/// are never dropped — they are the controls with no on-screen equivalent, and a
/// scrubber with little handle room is still draggable while a missing play
/// button is simply gone. The readout is the one part the transport can lose and
/// still be the transport; the scrubber's tooltip carries the frame number while
/// it is away.
///
/// `controls` is every fixed item but the readout; the row has seven gaps with
/// the readout in it and six without.
fn rail_budget(available: f32, controls: f32, readout: f32, gap: f32) -> RailBudget {
    let rail = available - (controls + readout + gap * 7.0);
    if rail >= size::ANIM_SCRUB_MIN_WIDTH {
        return RailBudget {
            rail,
            readout_shown: true,
        };
    }
    RailBudget {
        rail: (available - (controls + gap * 6.0)).max(size::ANIM_SCRUB_MIN_WIDTH),
        readout_shown: false,
    }
}

/// The transport's single row of controls: the four transport buttons pinned
/// left, the loop toggle + speed dropdown pinned right, and the scrubber
/// stretched across everything between them.
///
/// The widths of both end groups are known up front, so the rail is sized by
/// subtraction rather than by a token — the slider is whatever the row has left
/// ([`rail_budget`]). The readout between them is allocated at the width of the
/// *widest* string this clip can produce (plus a trailing pad), so a frame
/// counter or an elapsed time gaining a digit can't shove the right-hand
/// controls sideways.
pub(crate) fn transport_row(
    ui: &mut egui::Ui,
    state: &mut UiState,
    clip: &AnimationClip,
    fps: f64,
) {
    let anim = &mut state.animation;
    let last_frame = clip.frame_count(fps).saturating_sub(1);
    // Square tiles the height of the speed dropdown — `compact_combo` sizes its
    // button from `PANEL_ROW_H`, so every control in the row reads as one band.
    let tile = egui::Vec2::splat(size::PANEL_ROW_H);
    let button = |ui: &mut egui::Ui, icon: &AppIcon, selected: bool, tooltip: Tip| {
        widgets::icon_tile_button(ui, icon, selected, tooltip, tile, false).clicked()
    };

    ui.horizontal(|ui| {
        let gap = size::ANIM_TRANSPORT_GAP;
        ui.spacing_mut().item_spacing.x = gap;

        // The numerals are formatted here - the frame padded to a constant width,
        // the seconds to two places - and handed to the message as values, so the
        // words and their order around them are the catalog's.
        let digits = last_frame.to_string().len();
        let readout = keys::ui_transport::readout(
            format!("{:>digits$}", clip.frame_at(anim.time, fps)),
            last_frame as f64,
            format!("{:.2}", (anim.time - clip.time_begin).max(0.0)),
        );
        // The frame field is space-padded to a constant width already; the elapsed
        // seconds are not, so the block is measured at the clip's full duration.
        let widest = keys::ui_transport::readout(
            last_frame.to_string(),
            last_frame as f64,
            format!("{:.2}", clip.duration()),
        );
        let readout_width = mono_width(ui, &widest) + size::ANIM_TRANSPORT_GROUP_GAP;

        // Five tiles (four transport + loop) and the combo, whose `width` is its
        // inner content so its frame padding counts too. Whatever is left over
        // goes to the rail and the readout.
        let combo_width = size::ANIM_SPEED_COMBO_WIDTH;
        let controls = tile.x * 5.0 + combo_width + ui.spacing().button_padding.x * 2.0;
        let RailBudget {
            rail,
            readout_shown,
        } = rail_budget(ui.available_width(), controls, readout_width, gap);

        if button(
            ui,
            &assets::ICON_ANIM_GO_TO_START,
            false,
            Tip::new(keys::ui_transport::FIRST_FRAME).page(Page::Animation),
        ) {
            anim.time = clip.time_begin;
        }
        if button(
            ui,
            &assets::ICON_ANIM_STEP_BACK,
            false,
            Tip::new(keys::ui_transport::PREVIOUS_FRAME)
                .describe(keys::ui_transport::PREVIOUS_FRAME_DESCRIPTION)
                .page(Page::Animation),
        ) {
            anim.time = clip.step_frame_time(anim.time, -1, fps);
            anim.playing = false;
        }
        let play_icon = if anim.playing {
            &assets::ICON_ANIM_PAUSE
        } else {
            &assets::ICON_ANIM_PLAY
        };
        if button(
            ui,
            play_icon,
            anim.playing,
            Tip::new(keys::ui_transport::PLAY_PAUSE)
                .describe(keys::ui_transport::PLAY_PAUSE_DESCRIPTION)
                .page(Page::Animation),
        ) {
            anim.toggle_playback(clip);
        }
        if button(
            ui,
            &assets::ICON_ANIM_STEP_FORWARD,
            false,
            Tip::new(keys::ui_transport::NEXT_FRAME)
                .describe(keys::ui_transport::NEXT_FRAME_DESCRIPTION)
                .page(Page::Animation),
        ) {
            anim.time = clip.step_frame_time(anim.time, 1, fps);
            anim.playing = false;
        }

        // The scrubber works in whole frames; dragging it pauses playback so the
        // frame under the pointer is the one shown.
        let mut frame = clip.frame_at(anim.time, fps) as f64;
        ui.spacing_mut().slider_width = rail;
        let scrub = ui.add(
            egui::Slider::new(&mut frame, 0.0..=last_frame as f64)
                .integer()
                .show_value(false),
        );
        if scrub.changed() {
            anim.time = clip.frame_time(frame.round().max(0.0) as usize, fps);
            anim.playing = false;
        }
        if !readout_shown {
            // The bar is too narrow to show the frame number, so the scrubber
            // holds it: dropping the readout must not mean losing the figure.
            scrub.on_hover_text(&readout);
        }

        // Painted into a fixed-width block rather than laid out as a label, so
        // its own width never depends on the value it is showing.
        if readout_shown {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(readout_width, tile.y), egui::Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                readout,
                egui::FontId::monospace(font::STATS),
                color::TEXT_VALUE,
            );
        }

        if button(
            ui,
            &assets::ICON_ANIM_LOOP,
            anim.looping,
            Tip::new(keys::ui_transport::LOOP)
                .describe(keys::ui_transport::LOOP_DESCRIPTION)
                .page(Page::Animation),
        ) {
            anim.looping = !anim.looping;
        }

        widgets::compact_combo(
            ui,
            "animation_speed",
            combo_width,
            anim.speed.label(),
            |ui| {
                for speed in PlaybackSpeed::ALL {
                    ui.selectable_value(&mut anim.speed, speed, speed.label());
                }
            },
        );
    });
}

/// Width of `text` in the transport's monospace readout font.
fn mono_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::monospace(font::STATS),
            color::TEXT_VALUE,
        )
        .rect
        .width()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything in the row except the rail and the readout, at the real
    /// tokens: five tiles, the speed combo and its frame padding.
    fn controls() -> f32 {
        size::PANEL_ROW_H * 5.0 + size::ANIM_SPEED_COMBO_WIDTH + 8.0
    }

    /// At its preferred width the row shows everything; squeezed, the readout is
    /// what goes, and the rail never falls below its floor.
    ///
    /// This is the behaviour the old floating card could not have: it was a
    /// fixed 610pt that simply ran under whatever sat beside it.
    #[test]
    fn the_readout_is_what_gives_when_the_row_is_narrow() {
        let gap = size::ANIM_TRANSPORT_GAP;
        let readout = 120.0;

        let roomy = rail_budget(size::ANIM_TRANSPORT_WIDTH, controls(), readout, gap);
        assert!(roomy.readout_shown, "a full-width row should show it all");
        assert!(roomy.rail >= size::ANIM_SCRUB_MIN_WIDTH);

        let tight = rail_budget(size::ANIM_TRANSPORT_MIN_WIDTH, controls(), readout, gap);
        assert!(
            !tight.readout_shown,
            "a narrow row must drop the readout, not overlap its neighbours"
        );
        assert!(
            tight.rail >= size::ANIM_SCRUB_MIN_WIDTH,
            "the rail kept {}pt, below its {}pt floor",
            tight.rail,
            size::ANIM_SCRUB_MIN_WIDTH
        );
    }

    /// Dropping the readout must actually buy the rail room — the whole point of
    /// dropping it. Pinned because the two budgets count gaps differently, and
    /// getting that wrong would shed the readout for nothing.
    #[test]
    fn dropping_the_readout_widens_the_rail() {
        let gap = size::ANIM_TRANSPORT_GAP;
        let readout = 120.0;
        // A width just under the point where the readout stops fitting.
        let threshold = controls() + readout + gap * 7.0 + size::ANIM_SCRUB_MIN_WIDTH;
        let just_under = rail_budget(threshold - 1.0, controls(), readout, gap);
        assert!(!just_under.readout_shown);
        assert!(
            just_under.rail > size::ANIM_SCRUB_MIN_WIDTH,
            "the freed readout block should have gone to the rail"
        );
    }

    /// A row narrower than its own fixed controls can't produce a negative rail
    /// — the bar clamps the transport away before this, but the floor is what
    /// makes the arithmetic safe if that guard ever moves.
    #[test]
    fn an_impossible_width_still_clamps_to_the_floor() {
        let budget = rail_budget(10.0, controls(), 120.0, size::ANIM_TRANSPORT_GAP);
        assert!(!budget.readout_shown);
        assert_eq!(budget.rail, size::ANIM_SCRUB_MIN_WIDTH);
    }
}
