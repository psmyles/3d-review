//! The model-stats overlay: a compact, fixed-width grid of measured counts.
//!
//! Every value shown is a real measured number carried through import in
//! [`review_model::ModelStats`] (invariant 5) — never a placeholder.

use review_localization::Key;
use review_model::ScopeStats;

use crate::docs::Page;
use crate::keys;
use crate::state::{ScopedStats, TexturePoolEntry, UiState};
use crate::theme::{color, font, motion, size};
use crate::widgets::{Tip, mono_label, tip};

/// A row of the model-stats card, or of the Opt workspace's second card.
///
/// The rows used to be identified by their English label, with the explanations
/// looked up in a `match` on that string. That made every label a lookup key, so
/// translating one silently dropped its explanation — and it put the same word,
/// spelled twice, between a row and the thing it describes. The label is now one
/// of the row's properties rather than its name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatRow {
    Draws,
    Polys,
    Tris,
    Verts,
    GpuVerts,
    VtxSplits,
    UvSets,
    Bones,
    Clips,
    Unit,
    Fps,
    MeshVerts,
    Acmr,
    Atvr,
    Overdraw,
    Overfetch,
    Error,
}

impl StatRow {
    /// A stable id for egui's per-row state (the click-to-copy feedback) and for
    /// the row's hit rectangle. Deliberately **not** the label: a localized id
    /// would move a row's stored state when the language changed.
    fn id(self) -> &'static str {
        match self {
            StatRow::Draws => "draws",
            StatRow::Polys => "polys",
            StatRow::Tris => "tris",
            StatRow::Verts => "verts",
            StatRow::GpuVerts => "gpu_verts",
            StatRow::VtxSplits => "vtx_splits",
            StatRow::UvSets => "uv_sets",
            StatRow::Bones => "bones",
            StatRow::Clips => "clips",
            StatRow::Unit => "unit",
            StatRow::Fps => "fps",
            StatRow::MeshVerts => "mesh_verts",
            StatRow::Acmr => "acmr",
            StatRow::Atvr => "atvr",
            StatRow::Overdraw => "overdraw",
            StatRow::Overfetch => "overfetch",
            StatRow::Error => "error",
        }
    }

    fn label(self) -> Key {
        match self {
            StatRow::Draws => keys::ui_stats::DRAWS,
            StatRow::Polys => keys::ui_stats::POLYS,
            StatRow::Tris => keys::ui_stats::TRIS,
            StatRow::Verts => keys::ui_stats::VERTS,
            StatRow::GpuVerts => keys::ui_stats::GPU_VERTS,
            StatRow::VtxSplits => keys::ui_stats::VTX_SPLITS,
            StatRow::UvSets => keys::ui_stats::UV_SETS,
            StatRow::Bones => keys::ui_stats::BONES,
            StatRow::Clips => keys::ui_stats::CLIPS,
            StatRow::Unit => keys::ui_stats::UNIT,
            StatRow::Fps => keys::ui_stats::FPS,
            StatRow::MeshVerts => keys::ui_stats::MESH_VERTS,
            StatRow::Acmr => keys::ui_stats::ACMR,
            StatRow::Atvr => keys::ui_stats::ATVR,
            StatRow::Overdraw => keys::ui_stats::OVERDRAW,
            StatRow::Overfetch => keys::ui_stats::OVERFETCH,
            StatRow::Error => keys::ui_stats::ERROR,
        }
    }

    fn description(self) -> Key {
        match self {
            StatRow::Draws => keys::ui_stats::DRAWS_DESCRIPTION,
            StatRow::Polys => keys::ui_stats::POLYS_DESCRIPTION,
            StatRow::Tris => keys::ui_stats::TRIS_DESCRIPTION,
            StatRow::Verts => keys::ui_stats::VERTS_DESCRIPTION,
            StatRow::GpuVerts => keys::ui_stats::GPU_VERTS_DESCRIPTION,
            StatRow::VtxSplits => keys::ui_stats::VTX_SPLITS_DESCRIPTION,
            StatRow::UvSets => keys::ui_stats::UV_SETS_DESCRIPTION,
            StatRow::Bones => keys::ui_stats::BONES_DESCRIPTION,
            StatRow::Clips => keys::ui_stats::CLIPS_DESCRIPTION,
            StatRow::Unit => keys::ui_stats::UNIT_DESCRIPTION,
            StatRow::Fps => keys::ui_stats::FPS_DESCRIPTION,
            StatRow::MeshVerts => keys::ui_stats::MESH_VERTS_DESCRIPTION,
            StatRow::Acmr => keys::ui_stats::ACMR_DESCRIPTION,
            StatRow::Atvr => keys::ui_stats::ATVR_DESCRIPTION,
            StatRow::Overdraw => keys::ui_stats::OVERDRAW_DESCRIPTION,
            StatRow::Overfetch => keys::ui_stats::OVERFETCH_DESCRIPTION,
            StatRow::Error => keys::ui_stats::ERROR_DESCRIPTION,
        }
    }

    /// Where a row's full explanation lives. The counts and the file's own
    /// figures are the stats page; the four GPU-behaviour ratios and the
    /// simplifier's error belong to the Opt comparison, which is the only place
    /// they are produced.
    fn page(self) -> Page {
        match self {
            StatRow::MeshVerts
            | StatRow::Acmr
            | StatRow::Atvr
            | StatRow::Overdraw
            | StatRow::Overfetch
            | StatRow::Error => Page::OptComparison,
            _ => Page::Stats,
        }
    }
}

/// One of the model-stats card's three scope columns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StatScope {
    All,
    Sel,
    Vis,
}

impl StatScope {
    /// Left to right, the order the card lays them out in.
    const ALL: [StatScope; 3] = [StatScope::All, StatScope::Sel, StatScope::Vis];

    fn heading(self) -> Key {
        match self {
            StatScope::All => keys::ui_stats::SCOPE_ALL,
            StatScope::Sel => keys::ui_stats::SCOPE_SEL,
            StatScope::Vis => keys::ui_stats::SCOPE_VIS,
        }
    }

    /// The column's own explanation. `Vis` names the platform's modifier, so it
    /// is formatted rather than looked up — which is exactly what the old
    /// `concat!`-with-a-literal could not do in another language.
    fn description(self) -> egui::WidgetText {
        match self {
            StatScope::All => keys::ui_stats::SCOPE_ALL_DESCRIPTION.into(),
            StatScope::Sel => keys::ui_stats::SCOPE_SEL_DESCRIPTION.into(),
            StatScope::Vis => {
                keys::ui_stats::scope_vis_description(crate::primary_modifier().into_owned()).into()
            }
        }
    }
}

/// A row of the Tex viewport's card.
#[derive(Clone, Copy)]
enum TexStatRow {
    Format,
    Dimension,
    Channels,
    BitDepth,
    FileSize,
}

impl TexStatRow {
    fn id(self) -> &'static str {
        match self {
            TexStatRow::Format => "tex_format",
            TexStatRow::Dimension => "tex_dimension",
            TexStatRow::Channels => "tex_channels",
            TexStatRow::BitDepth => "tex_bit_depth",
            TexStatRow::FileSize => "tex_file_size",
        }
    }

    fn label(self) -> Key {
        match self {
            TexStatRow::Format => keys::ui_stats::TEX_FORMAT,
            TexStatRow::Dimension => keys::ui_stats::TEX_DIMENSION,
            TexStatRow::Channels => keys::ui_stats::TEX_CHANNELS,
            TexStatRow::BitDepth => keys::ui_stats::TEX_BIT_DEPTH,
            TexStatRow::FileSize => keys::ui_stats::TEX_FILE_SIZE,
        }
    }

    fn description(self) -> Key {
        match self {
            TexStatRow::Format => keys::ui_stats::TEX_FORMAT_DESCRIPTION,
            TexStatRow::Dimension => keys::ui_stats::TEX_DIMENSION_DESCRIPTION,
            TexStatRow::Channels => keys::ui_stats::TEX_CHANNELS_DESCRIPTION,
            TexStatRow::BitDepth => keys::ui_stats::TEX_BIT_DEPTH_DESCRIPTION,
            TexStatRow::FileSize => keys::ui_stats::TEX_FILE_SIZE_DESCRIPTION,
        }
    }
}

/// The Tex viewport's stats panel: the viewed texture's source format, pixel
/// dimensions, channel layout, bit depth and on-disk file size. Every value is a
/// real measured property of the file (invariant 5) — the `source_*` counts come
/// straight from the decoder, the size from `app`'s `fs::metadata`.
pub(crate) fn texture_stats_grid(ui: &mut egui::Ui, entry: &TexturePoolEntry) {
    let image = &entry.image;
    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;
    tex_row(ui, TexStatRow::Format, &entry.format_label());
    tex_row(
        ui,
        TexStatRow::Dimension,
        &keys::ui_stats::tex_dimension_value(f64::from(image.width), f64::from(image.height)),
    );
    tex_row(
        ui,
        TexStatRow::Channels,
        &channel_label(image.source_channels),
    );
    // Bit depth is shown as total bits per pixel (per-channel depth × channels),
    // e.g. 8-bit RGBA → 32, matching the convention texture tools display.
    let bits = image.source_bit_depth as u32 * image.source_channels.max(1) as u32;
    tex_row(ui, TexStatRow::BitDepth, &bits.to_string());
    tex_row(ui, TexStatRow::FileSize, &human_size(entry.file_size));
}

/// Short channel-layout label for a source channel count.
fn channel_label(channels: u8) -> String {
    let key = match channels {
        1 => keys::ui_stats::CHANNELS_GREY,
        2 => keys::ui_stats::CHANNELS_GREY_ALPHA,
        3 => keys::ui_stats::CHANNELS_RGB,
        4 => keys::ui_stats::CHANNELS_RGBA,
        _ => keys::ui_stats::UNMEASURED,
    };
    review_localization::tr(key).into_owned()
}

/// Format a byte count as a compact human-readable size (B / KB / MB / GB), the
/// binary (1024) step the file managers use. `0` reads as "-" (size unknown).
fn human_size(bytes: u64) -> String {
    if bytes == 0 {
        return review_localization::tr(keys::ui_stats::UNMEASURED).into_owned();
    }
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    let scaled = |unit: u64| format!("{:.1}", bytes as f64 / unit as f64);
    if bytes >= GB {
        keys::ui_units::gigabytes_value(scaled(GB))
    } else if bytes >= MB {
        keys::ui_units::megabytes_value(scaled(MB))
    } else if bytes >= KB {
        keys::ui_units::kilobytes_value(scaled(KB))
    } else {
        keys::ui_units::bytes_value(bytes.to_string())
    }
}

/// What one scope column has to say about one row.
enum Cell {
    /// A measured value.
    Value(String),
    /// This scope was measured and has nothing to report here — nothing is
    /// selected, or a material slot has no authored vertex count. Shown as an em
    /// dash, never a zero (invariant 5).
    Unmeasured,
    /// The row isn't about a slice of geometry at all (the UV-set count, the
    /// file's unit, the viewer's frame rate), so the column stays empty rather
    /// than claiming the scope was measured and came up short.
    NotScoped,
}

impl Cell {
    fn measured(value: Option<usize>) -> Self {
        Self::from_text(value.map(|value| value.to_string()))
    }

    fn from_text(value: Option<String>) -> Self {
        value.map_or(Cell::Unmeasured, Cell::Value)
    }
}

/// The model-stats overlay: every measured count, reported three ways — for the
/// whole file, for the selection, and for what is still visible.
///
/// The All column is the source file's own figures as import carried them
/// (invariant 5); the other two are measured off the mesh by
/// [`UiState::scoped_stats`], summed from the per-draw-group table so no scan
/// runs on the redraw path. A row with nothing to say for a scope shows an em
/// dash, never a zero.
pub(crate) fn stats_grid(ui: &mut egui::Ui, state: &UiState, scoped: ScopedStats) {
    let stats = &state.stats;
    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;

    scope_heading_row(ui);
    scoped_row(ui, StatRow::Draws, stats.draw_count, scoped, |scope| {
        Some(scope.draw_count)
    });
    scoped_row(ui, StatRow::Polys, stats.polygon_count, scoped, |scope| {
        Some(scope.polygon_count)
    });
    scoped_row(ui, StatRow::Tris, stats.triangle_count, scoped, |scope| {
        Some(scope.triangle_count)
    });
    scoped_row(ui, StatRow::Verts, stats.vertex_count, scoped, |scope| {
        scope.vertex_count
    });
    // `Verts` above is the count the artist's DCC reports; `GPU Verts` is what
    // the asset costs an engine — unique vertices per draw group, measured at
    // import. (This viewer's own corner-split upload is an internal layout and
    // deliberately not a stat.) The gap between the two is the split overhead:
    // extra vertices the asset's hard edges and UV seams cost, which is exactly
    // the kind of figure an audit tool exists to flag — a healthy game asset
    // reads a few percent, a scan with per-face normals reads +500%.
    if stats.gpu_vertex_count > 0 {
        scoped_row(
            ui,
            StatRow::GpuVerts,
            stats.gpu_vertex_count,
            scoped,
            |scope| Some(scope.gpu_vertex_count),
        );
        splits_row(ui, stats.vertex_count, stats.gpu_vertex_count, scoped);
    }
    // The rest describe the file or the viewer, not a slice of geometry, so they
    // carry one value under the All column rather than repeating it three times.
    whole_model_row(ui, StatRow::UvSets, &stats.uv_set_count.to_string());
    // Skeletal models only: an unrigged mesh shouldn't carry a permanent "0".
    if stats.bone_count > 0 {
        whole_model_row(ui, StatRow::Bones, &stats.bone_count.to_string());
    }
    // Animated files only, for the same reason.
    if stats.clip_count > 0 {
        whole_model_row(ui, StatRow::Clips, &stats.clip_count.to_string());
    }
    whole_model_row(
        ui,
        StatRow::Unit,
        &source_unit_label(stats.source_unit_meters),
    );
    whole_model_row(ui, StatRow::Fps, &format!("{:.0}", state.fps));
}

/// The column headings, each carrying its own explanation of what it covers.
fn scope_heading_row(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.style_mut().interaction.selectable_labels = false;
        // Empty label cell: the headings sit over the value columns, and the
        // leftmost column holds row names, which need no heading.
        ui.label(mono_label("", font::STATS, color::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The columns carry their own fixed width; egui's default gap between
            // them would push the leftmost one into the row labels.
            ui.spacing_mut().item_spacing.x = 0.0;
            for scope in StatScope::ALL.iter().rev() {
                let (rect, response) = value_cell(ui);
                paint_cell(
                    ui,
                    rect,
                    &review_localization::tr(scope.heading()),
                    color::TEXT_MUTED,
                );
                tip(
                    response,
                    Tip::new(scope.heading())
                        .describe(scope.description())
                        .page(Page::Stats),
                );
            }
        });
    });
    ui.separator();
}

/// A measured count in each of the three scopes. `all` is the whole-model figure
/// as the source file reported it; `measure` pulls the same count out of a
/// measured scope.
fn scoped_row(
    ui: &mut egui::Ui,
    row: StatRow,
    all: usize,
    scoped: ScopedStats,
    measure: impl Fn(&ScopeStats) -> Option<usize>,
) {
    cells_row(
        ui,
        row,
        [
            Cell::Value(all.to_string()),
            Cell::measured(scoped.selected.as_ref().and_then(&measure)),
            Cell::measured(measure(&scoped.visible)),
        ],
    );
}

/// The vertex-split overhead, recomputed per scope from that scope's own two
/// vertex counts rather than scaled off the whole model's — a selection of
/// hard-edged props splits far more than the file's average.
fn splits_row(ui: &mut egui::Ui, vertices: usize, gpu_vertices: usize, scoped: ScopedStats) {
    let overhead = |vertices: Option<usize>, gpu_vertices: usize| {
        let vertices = vertices.filter(|&count| count > 0)?;
        let percent = (gpu_vertices as f32 - vertices as f32) / vertices as f32 * 100.0;
        Some(format!("{percent:+.0}%"))
    };
    cells_row(
        ui,
        StatRow::VtxSplits,
        [
            Cell::from_text(overhead(Some(vertices), gpu_vertices)),
            Cell::from_text(
                scoped
                    .selected
                    .and_then(|scope| overhead(scope.vertex_count, scope.gpu_vertex_count)),
            ),
            Cell::from_text(overhead(
                scoped.visible.vertex_count,
                scoped.visible.gpu_vertex_count,
            )),
        ],
    );
}

/// A row whose value describes the file or the viewer as a whole, so it sits
/// under the All column and leaves the scoped ones blank.
fn whole_model_row(ui: &mut egui::Ui, row: StatRow, value: &str) {
    cells_row(
        ui,
        row,
        [
            Cell::Value(value.to_owned()),
            Cell::NotScoped,
            Cell::NotScoped,
        ],
    );
}

/// One row of the model-stats card: its name, then a fixed-width cell per scope.
///
/// The whole strip is one click target that copies the row — every column of it
/// — to the clipboard, and the widget that carries the row's explanation.
fn cells_row(ui: &mut egui::Ui, row: StatRow, cells: [Cell; 3]) {
    let label = review_localization::tr(row.label());
    // Only the columns that actually carry a value; a row with one is copied as
    // plainly as it reads on screen.
    let measured: Vec<String> = StatScope::ALL
        .iter()
        .zip(&cells)
        .filter_map(|(scope, cell)| match cell {
            Cell::Value(value) => Some(format!(
                "{} {value}",
                review_localization::tr(scope.heading())
            )),
            Cell::Unmeasured | Cell::NotScoped => None,
        })
        .collect();
    let copy_text = format!("{label}: {}", measured.join(", "));

    let rect = ui
        .horizontal(|ui| {
            // The stats are display-only. egui's labels are selectable by
            // default, which puts a text cursor over the row and — because a
            // selectable label senses drags — makes it an interactive widget
            // that swallows the row's own hover, so the explanations below never
            // reached the screen.
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(mono_label(&label, font::STATS, color::TEXT_MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed columns, no gap — see [`scope_heading_row`].
                ui.spacing_mut().item_spacing.x = 0.0;
                for cell in cells.iter().rev() {
                    let (rect, _) = value_cell(ui);
                    match cell {
                        Cell::Value(value) => paint_cell(ui, rect, value, color::TEXT_VALUE),
                        // A hyphen, never a zero (invariant 5). Not a message:
                        // it is punctuation, and it is the same mark in every
                        // language the bundled fonts cover.
                        Cell::Unmeasured => paint_cell(ui, rect, "-", color::TEXT_MUTED),
                        Cell::NotScoped => {}
                    }
                }
            });
        })
        .response
        .rect;

    row_interaction(ui, rect, row.id(), Some(row), copy_text);
}

/// Claim one fixed-width scope column. Fixed rather than content-sized so the
/// three columns stay in line down the card whatever lands in them.
fn value_cell(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    ui.allocate_exact_size(
        egui::vec2(size::STATS_VALUE_COLUMN, height),
        egui::Sense::hover(),
    )
}

/// Paint one column's text against the right edge of its cell, so the digits
/// form a single column even as the numbers change width.
fn paint_cell(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    ui.painter().text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        text,
        egui::FontId::new(font::STATS, egui::FontFamily::Monospace),
        color,
    );
}

/// The Opt workspace's second stats card: the measured counts and GPU-behaviour
/// figures of whatever the right-hand view is showing, each with its change
/// against the source.
///
/// Every number here is measured off the mesh in hand — the counts by walking its
/// buffers, the ratios by meshoptimizer's own analyzers (invariant 5). The
/// changes are computed from two measured values, never estimated.
///
/// The card is up whenever the workspace has run at all, and a run with nothing
/// enabled still measures the source: the point of the reorder operations is to
/// move ACMR and overfetch, and choosing whether to add one means reading where
/// they already stand.
pub(crate) fn processed_stats_grid(ui: &mut egui::Ui, state: &UiState) {
    let Some(result) = state.opt.result.as_ref() else {
        return;
    };
    // Measured off the source mesh by the same run, *after* its lossless index
    // pass — the buffer an engine importer would build. Quoting against the
    // DCC count or the viewer's corner-split upload made honest operations read
    // as inventing or deleting most of the mesh.
    let source = result.source;
    let source_metrics = result.source_metrics;
    let level = state.opt.active_level();

    ui.spacing_mut().item_spacing.y = size::STATS_ROW_SPACING;

    // With no level there is nothing processed to name, and the figures below are
    // the source's own. Otherwise name the level whenever there is more than one
    // to be on: "Processed" alone leaves the reader to guess whether they are
    // looking at the simplified mesh or the level it was simplified from.
    let heading = match (level, result.levels.len()) {
        (None, _) => review_localization::tr(keys::ui_stats::SOURCE_NOTHING_APPLIED).into_owned(),
        (Some(_), count) if count > 1 => keys::ui_stats::processed_lod(state.opt.active_lod as f64),
        (Some(_), _) => review_localization::tr(keys::ui_stats::PROCESSED).into_owned(),
    };
    ui.label(mono_label(&heading, font::STATS, color::TEXT_PRIMARY));
    ui.add_space(size::STATS_ROW_SPACING);

    // Against itself, the source's every change is zero, so the baseline card
    // shows plain values; a column of "0%" would be noise.
    let baseline = level.map(|_| source);
    let stats = level.map_or_else(
        || (source.triangles, source.vertices, state.stats.draw_count),
        |level| {
            (
                level.stats.triangle_count,
                level.stats.vertex_count,
                level.stats.draw_count,
            )
        },
    );
    let metrics = level.map_or(source_metrics, |level| level.metrics);

    delta_row(
        ui,
        StatRow::Tris,
        stats.0,
        baseline.map(|source| source.triangles),
    );
    // "Mesh Verts", matching the source card's row of that name: both count the
    // vertex buffer. Calling this one "Verts" put the same word on two different
    // measurements — the file's DCC count on one card and the buffer length on
    // the other — and the two cards read as contradicting each other.
    delta_row(
        ui,
        StatRow::MeshVerts,
        stats.1,
        baseline.map(|source| source.vertices),
    );
    stat_row(ui, StatRow::Draws, &stats.2.to_string());

    ui.add_space(size::STATS_ROW_SPACING);
    // ACMR/ATVR describe vertex-cache behaviour, overdraw the pixel cost, and
    // overfetch the vertex-buffer read pattern. They are what makes the reorder
    // operations — which change nothing visible — measurable, so they carry their
    // change too: a reorder that moved nothing is a reorder worth removing.
    let compare = level.map(|_| source_metrics);
    metric_row(ui, StatRow::Acmr, metrics.acmr, compare.map(|m| m.acmr));
    metric_row(ui, StatRow::Atvr, metrics.atvr, compare.map(|m| m.atvr));
    metric_row(
        ui,
        StatRow::Overdraw,
        metrics.overdraw,
        compare.map(|m| m.overdraw),
    );
    metric_row(
        ui,
        StatRow::Overfetch,
        metrics.overfetch,
        compare.map(|m| m.overfetch),
    );

    // The simplifier's achieved error, shown only for a level that ran one — a
    // LOD level, or level 0 once a Reduce rewrote it. It is meaningless for an
    // unsimplified level, and has no source counterpart to change against.
    if level.is_some() && metrics.simplified {
        stat_row(
            ui,
            StatRow::Error,
            &format!("{:.4}", metrics.simplify_error),
        );
    }
}

/// A count row, with its change against the source when there is one to show.
fn delta_row(ui: &mut egui::Ui, row: StatRow, value: usize, source: Option<usize>) {
    value_row(
        ui,
        row.id(),
        Some(row),
        &review_localization::tr(row.label()),
        &value.to_string(),
        change(value as f32, source.map(|source| source as f32)),
    );
}

/// A measured ratio (ACMR / ATVR / overdraw / overfetch) and its change.
fn metric_row(ui: &mut egui::Ui, row: StatRow, value: f32, source: Option<f32>) {
    value_row(
        ui,
        row.id(),
        Some(row),
        &review_localization::tr(row.label()),
        &format!("{value:.2}"),
        change(value, source),
    );
}

/// The percentage change from `source` to `value`, with the colour that says
/// whether it was an improvement.
///
/// Every figure on this card is one where **lower is better** — fewer triangles
/// and vertices, fewer cache misses, less overdraw, fewer bytes fetched — so one
/// rule covers them all: down is green, up is red.
fn change(value: f32, source: Option<f32>) -> Option<(String, egui::Color32)> {
    let source = source?;
    if source == 0.0 || !source.is_finite() || !value.is_finite() {
        return None;
    }
    let percent = (value - source) / source * 100.0;
    // Below a twentieth of a percent the rounded figure would read "-0%", which
    // looks like a bug rather than "unchanged".
    if percent.abs() < 0.05 {
        return Some(("  0%".to_owned(), color::TEXT_MUTED));
    }
    let tint = if percent < 0.0 {
        color::STATS_DELTA_BETTER
    } else {
        color::STATS_DELTA_WORSE
    };
    Some((format!("{percent:+.0}%"), tint))
}

/// One stats row with an optional tinted change column to the right of the value.
///
/// The whole strip is one click target that copies the row to the clipboard, and
/// the widget that carries the row's explanation.
fn value_row(
    ui: &mut egui::Ui,
    id: &str,
    row: Option<StatRow>,
    label: &str,
    value: &str,
    delta: Option<(String, egui::Color32)>,
) {
    let copy_text = match &delta {
        Some((change, _)) => format!("{label}: {value} ({})", change.trim()),
        None => format!("{label}: {value}"),
    };

    let rect = ui
        .horizontal(|ui| {
            // The stats are display-only. egui's labels are selectable by
            // default, which puts a text cursor over the row and — because a
            // selectable label senses drags — makes it an interactive widget
            // that swallows the row's own hover, so the explanations below never
            // reached the screen.
            ui.style_mut().interaction.selectable_labels = false;
            ui.label(mono_label(label, font::STATS, color::TEXT_MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some((text, tint)) = delta {
                    ui.label(mono_label(&text, font::STATS, tint));
                }
                ui.label(mono_label(value, font::STATS, color::TEXT_VALUE));
            });
        })
        .response
        .rect;

    row_interaction(ui, rect, id, row, copy_text);
}

/// Turn a laid-out row into the card's one interactive element: click to copy it,
/// hover for its explanation. Shared by both cards' rows, single- and
/// multi-column, so a row behaves the same wherever it is drawn.
fn row_interaction(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    row_id: &str,
    row: Option<StatRow>,
    copy_text: String,
) {
    // Claimed *after* the labels so it sits above them in hit order — the row,
    // not a word in it, is what the pointer finds. Keyed on the row's stable id
    // rather than its label, so switching language does not move the row's
    // copied-feedback state to a different row.
    let id = ui.id().with(("stat_row", row_id));
    let response = ui.interact(rect, id, egui::Sense::click());
    let now = ui.input(|input| input.time);
    if response.clicked() {
        ui.ctx().copy_text(copy_text);
        ui.ctx().data_mut(|data| data.insert_temp(id, now));
    }

    // A copy leaves nothing on screen to show it happened, so the row says so
    // in place of its explanation for a moment. The pending repaint is what
    // clears it: the redraw loop is on-demand, and a pointer resting on the row
    // produces no further events (invariant 6).
    let copied_at: Option<f64> = ui.ctx().data(|data| data.get_temp(id));
    match copied_at.map(|at| now - at) {
        Some(elapsed) if elapsed < motion::STATS_COPIED_FEEDBACK_SECS => {
            ui.ctx()
                .request_repaint_after_secs((motion::STATS_COPIED_FEEDBACK_SECS - elapsed) as f32);
            response.on_hover_text(keys::common::COPIED);
        }
        _ => {
            if let Some(row) = row {
                tip(
                    response,
                    Tip::new(row.label())
                        .describe(row.description())
                        .page(row.page()),
                );
            }
        }
    }
}

/// Render the file's authored world unit (meters per source unit) as a short
/// label. Snaps the common DCC units to their names and falls back to the raw
/// factor for anything else; `0.0`/non-finite means the file declared no unit.
fn source_unit_label(meters_per_unit: f32) -> String {
    match crate::units::match_known_unit(meters_per_unit) {
        Some(unit) => review_localization::tr(unit.symbol).into_owned(),
        None if meters_per_unit.is_finite() && meters_per_unit > 0.0 => {
            crate::units::meters_value(format!("{meters_per_unit:.4}"))
        }
        None => review_localization::tr(keys::ui_stats::UNMEASURED).into_owned(),
    }
}

/// One stats row: label hugs the left edge, value right-aligns against the
/// panel's right edge so the numeric column reads as a tidy block.
fn stat_row(ui: &mut egui::Ui, row: StatRow, value: &str) {
    value_row(
        ui,
        row.id(),
        Some(row),
        &review_localization::tr(row.label()),
        value,
        None,
    );
}

/// One row of the Tex viewport's card. Its rows have no scope columns and no
/// change against a source, so they share `value_row` rather than `cells_row`.
fn tex_row(ui: &mut egui::Ui, row: TexStatRow, value: &str) {
    let label = review_localization::tr(row.label());
    value_row(ui, row.id(), None, &label, value, None);
    // `value_row` attaches a `StatRow`'s tooltip; a Tex row's is its own, and
    // pointing at the texture-workspace page rather than the stats one.
    let id = ui.id().with(("stat_row", row.id()));
    if let Some(response) = ui.ctx().read_response(id) {
        tip(
            response,
            Tip::new(row.label())
                .describe(row.description())
                .page(Page::Tex),
        );
    }
}
