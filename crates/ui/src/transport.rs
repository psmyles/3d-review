//! The animation transport: a floating card at the bottom centre of the viewport,
//! shown only while a clip is selected in the 3D workspace.
//!
//! Go-to-start · step back · play/pause · step forward · a frame scrubber · the
//! "frame / total   seconds" readout · loop · speed. Everything it edits is the
//! plain [`crate::state::AnimationUiState`] the Outliner's Animations tab also
//! writes; `app` runs the clock off it (invariant 6) and paces the redraw loop
//! while it plays. Built on the same centred overlay card as the Opt legend.

use review_model::{AnimationClip, ModelData};

use crate::assets::{self, AppIcon};
use crate::state::{PlaybackSpeed, UiState, WorkspaceMode};
use crate::theme::{self, color, font, size};
use crate::widgets::{self, StatsCardSide};

/// Draw the transport for the selected clip, if any. `left_inset` /
/// `right_inset` are the docked side panels' widths, so the card stays centred
/// in the free viewport rather than the window.
pub(crate) fn draw_transport(
    ctx: &egui::Context,
    state: &mut UiState,
    model: &ModelData,
    status_bar_height: f32,
    left_inset: f32,
    right_inset: f32,
) {
    if state.mode != WorkspaceMode::ThreeD {
        return;
    }
    let Some(clip) = state
        .animation
        .selected_clip
        .and_then(|index| model.animations.get(index))
    else {
        return;
    };
    let fps = model.frame_rate_or_default();

    let offset = (left_inset - right_inset) * 0.5;
    widgets::stats_overlay_card_at(
        ctx,
        "animation_transport",
        StatsCardSide::Center,
        offset,
        status_bar_height,
        size::ANIM_TRANSPORT_WIDTH,
        |ui| transport_row(ui, ctx, state, clip, fps),
    );
}

/// The card's single row of controls.
fn transport_row(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut UiState,
    clip: &AnimationClip,
    fps: f64,
) {
    let anim = &mut state.animation;
    let last_frame = clip.frame_count(fps).saturating_sub(1);
    let tile = egui::Vec2::splat(theme::px(ctx, size::ANIM_TRANSPORT_BUTTON));
    let button = |ui: &mut egui::Ui, icon: &AppIcon, selected: bool, tooltip: &str| {
        widgets::icon_tile_button(ui, ctx, icon, selected, tooltip, tile, false).clicked()
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::px(ctx, size::ANIM_TRANSPORT_GAP);

        if button(
            ui,
            &assets::ICON_ANIM_GO_TO_START,
            false,
            "Go to first frame",
        ) {
            anim.time = clip.time_begin;
        }
        if button(
            ui,
            &assets::ICON_ANIM_STEP_BACK,
            false,
            "Previous frame (,)",
        ) {
            anim.time = clip.step_frame_time(anim.time, -1, fps);
            anim.playing = false;
        }
        let play_icon = if anim.playing {
            &assets::ICON_ANIM_PAUSE
        } else {
            &assets::ICON_ANIM_PLAY
        };
        if button(ui, play_icon, anim.playing, "Play / pause (Space)") {
            // Playing again from the end of a non-looping clip starts over.
            if !anim.playing && !anim.looping && anim.time >= clip.time_end {
                anim.time = clip.time_begin;
            }
            anim.playing = !anim.playing;
        }
        if button(ui, &assets::ICON_ANIM_STEP_FORWARD, false, "Next frame (.)") {
            anim.time = clip.step_frame_time(anim.time, 1, fps);
            anim.playing = false;
        }

        // The scrubber works in whole frames; dragging it pauses playback so the
        // frame under the pointer is the one shown.
        let mut frame = clip.frame_at(anim.time, fps) as f64;
        ui.spacing_mut().slider_width = theme::px(ctx, size::ANIM_SCRUB_WIDTH);
        let scrub = ui.add(
            egui::Slider::new(&mut frame, 0.0..=last_frame as f64)
                .integer()
                .show_value(false),
        );
        if scrub.changed() {
            anim.time = clip.frame_time(frame.round().max(0.0) as usize, fps);
            anim.playing = false;
        }

        let elapsed = (anim.time - clip.time_begin).max(0.0);
        let digits = last_frame.to_string().len();
        ui.label(widgets::mono_label(
            &format!(
                "{:>digits$} / {last_frame}   {elapsed:.2} s",
                clip.frame_at(anim.time, fps)
            ),
            font::STATS,
            color::TEXT_VALUE,
        ));

        if button(ui, &assets::ICON_ANIM_LOOP, anim.looping, "Loop playback") {
            anim.looping = !anim.looping;
        }

        widgets::compact_combo(
            ui,
            "animation_speed",
            theme::px(ctx, size::ANIM_SPEED_COMBO_WIDTH),
            anim.speed.label(),
            |ui| {
                for speed in PlaybackSpeed::ALL {
                    ui.selectable_value(&mut anim.speed, speed, speed.label());
                }
            },
        );
    });
}
