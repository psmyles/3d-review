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
    stat_row(ui, "Unit", &source_unit_label(stats.source_unit_meters));
    stat_row(ui, "FPS", &format!("{:.0}", state.fps));
}

/// Render the file's authored world unit (meters per source unit) as a short
/// label. Snaps the common DCC units to their names and falls back to the raw
/// factor for anything else; `0.0`/non-finite means the file declared no unit.
fn source_unit_label(meters_per_unit: f32) -> String {
    if !meters_per_unit.is_finite() || meters_per_unit <= 0.0 {
        return "—".to_owned();
    }

    // (meters-per-unit, label) for the units a DCC commonly writes. Matched with
    // a relative tolerance so float drift in the file's factor still resolves.
    const KNOWN: [(f32, &str); 6] = [
        (1.0, "m"),
        (0.01, "cm"),
        (0.001, "mm"),
        (0.0254, "in"),
        (0.3048, "ft"),
        (0.9144, "yd"),
    ];
    for (factor, label) in KNOWN {
        if (meters_per_unit - factor).abs() <= factor * 0.001 {
            return label.to_owned();
        }
    }

    format!("{meters_per_unit:.4} m")
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
