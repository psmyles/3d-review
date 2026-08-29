//! The model-stats overlay: a compact, fixed-width grid of measured counts.
//!
//! Every value shown is a real measured number carried through import in
//! [`review_model::ModelStats`] (invariant 5) — never a placeholder.

use crate::state::{TexturePoolEntry, UiState};
use crate::theme::{color, font, size};
use crate::widgets::mono_label;

/// The Tex viewport's stats panel: the viewed texture's source format, pixel
/// dimensions, channel layout, bit depth and on-disk file size. Every value is a
/// real measured property of the file (invariant 5) — the `source_*` counts come
/// straight from the decoder, the size from `app`'s `fs::metadata`.
pub(crate) fn texture_stats_grid(ui: &mut egui::Ui, entry: &TexturePoolEntry) {
    let image = &entry.image;
    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
    stat_row(ui, "Format", &entry.format_label());
    stat_row(
        ui,
        "Dimension",
        &format!("{} × {}", image.width, image.height),
    );
    stat_row(ui, "Channels", channel_label(image.source_channels));
    // Bit depth is shown as total bits per pixel (per-channel depth × channels),
    // e.g. 8-bit RGBA → 32, matching the convention texture tools display.
    let bits = image.source_bit_depth as u32 * image.source_channels.max(1) as u32;
    stat_row(ui, "Bit depth", &bits.to_string());
    stat_row(ui, "File size", &human_size(entry.file_size));
}

/// Short channel-layout label for a source channel count.
fn channel_label(channels: u8) -> &'static str {
    match channels {
        1 => "Grey",
        2 => "Grey+A",
        3 => "RGB",
        4 => "RGBA",
        _ => "—",
    }
}

/// Format a byte count as a compact human-readable size (B / KB / MB / GB), the
/// binary (1024) step the file managers use. `0` reads as "—" (size unknown).
fn human_size(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_owned();
    }
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub(crate) fn stats_grid(ui: &mut egui::Ui, state: &UiState) {
    let stats = &state.stats;
    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
    stat_row(ui, "Draws", &stats.draw_count.to_string());
    stat_row(ui, "Polys", &stats.polygon_count.to_string());
    stat_row(ui, "Tris", &stats.triangle_count.to_string());
    stat_row(ui, "Verts", &stats.vertex_count.to_string());
    stat_row(ui, "UV Sets", &stats.uv_set_count.to_string());
    // Skeletal models only: an unrigged mesh shouldn't carry a permanent "0".
    if stats.bone_count > 0 {
        stat_row(ui, "Bones", &stats.bone_count.to_string());
    }
    stat_row(ui, "Unit", &source_unit_label(stats.source_unit_meters));
    stat_row(ui, "FPS", &format!("{:.0}", state.fps));
}

/// The Opt workspace's second stats card: the *processed* mesh's own measured
/// counts, each with its change against the source, plus the GPU-behaviour
/// figures meshoptimizer measured for it.
///
/// Every number here is measured off the processed mesh in hand — the counts by
/// walking its buffers, the ratios by meshoptimizer's own analyzers (invariant 5).
/// The deltas are computed from the two measured counts, not estimated.
pub(crate) fn processed_stats_grid(ui: &mut egui::Ui, state: &UiState) {
    let Some(level) = state.opt.active_level() else {
        return;
    };
    let source = &state.stats;
    let stats = &level.stats;
    let metrics = &level.metrics;

    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;

    let heading = if state.opt.active_lod == 0 {
        "Processed".to_owned()
    } else {
        format!("Processed · LOD {}", state.opt.active_lod)
    };
    ui.label(mono_label(&heading, font::STATS, color::TEXT_PRIMARY));
    ui.add_space(size::STATS_ROW_SPACING);

    delta_row(ui, "Tris", stats.triangle_count, source.triangle_count);
    delta_row(ui, "Verts", stats.vertex_count, source.vertex_count);
    stat_row(ui, "Draws", &stats.draw_count.to_string());

    ui.add_space(size::STATS_ROW_SPACING);
    // ACMR/ATVR describe vertex-cache behaviour, overdraw the pixel cost, and
    // overfetch the vertex-buffer read pattern. They are what makes the reorder
    // operations — which change nothing visible — measurable.
    stat_row(ui, "ACMR", &format!("{:.2}", metrics.acmr));
    stat_row(ui, "ATVR", &format!("{:.2}", metrics.atvr));
    stat_row(ui, "Overdraw", &format!("{:.2}", metrics.overdraw));
    stat_row(ui, "Overfetch", &format!("{:.2}", metrics.overfetch));

    // The simplifier's achieved error, shown only for a level that ran one — it
    // is meaningless (and always zero) for the unsimplified level 0.
    if state.opt.active_lod > 0 {
        stat_row(ui, "Error", &format!("{:.4}", metrics.simplify_error));
    }
}

/// A count row carrying its percentage change against the source. A reduction
/// reads as a negative percentage, which is the direction that matters here.
fn delta_row(ui: &mut egui::Ui, label: &str, value: usize, source: usize) {
    let delta = if source == 0 {
        String::new()
    } else {
        let change = (value as f32 - source as f32) / source as f32 * 100.0;
        // Below a tenth of a percent, the rounded figure would read "-0.0%",
        // which looks like a bug rather than "essentially unchanged".
        if change.abs() < 0.05 {
            "  0%".to_owned()
        } else {
            format!("{change:+.0}%")
        }
    };
    ui.horizontal(|ui| {
        ui.label(mono_label(label, font::STATS, color::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !delta.is_empty() {
                ui.label(mono_label(&delta, font::STATS, color::TEXT_MUTED));
            }
            ui.label(mono_label(
                &value.to_string(),
                font::STATS,
                color::TEXT_VALUE,
            ));
        });
    });
}

/// Render the file's authored world unit (meters per source unit) as a short
/// label. Snaps the common DCC units to their names and falls back to the raw
/// factor for anything else; `0.0`/non-finite means the file declared no unit.
fn source_unit_label(meters_per_unit: f32) -> String {
    match crate::units::match_known_unit(meters_per_unit) {
        Some((_, label)) => label.to_owned(),
        None if meters_per_unit.is_finite() && meters_per_unit > 0.0 => {
            format!("{meters_per_unit:.4} m")
        }
        None => "—".to_owned(),
    }
}

/// One stats row: label hugs the left edge, value right-aligns against the
/// panel's right edge so the numeric column reads as a tidy block.
fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(mono_label(label, font::STATS, color::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(mono_label(value, font::STATS, color::TEXT_VALUE));
        });
    });
}
