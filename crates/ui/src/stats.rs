//! The model-stats overlay: a compact, fixed-width grid of measured counts.
//!
//! Every value shown is a real measured number carried through import in
//! [`review_model::ModelStats`] (invariant 5) — never a placeholder.

use review_model::ScopeStats;

use crate::state::{ScopedStats, TexturePoolEntry, UiState};
use crate::theme::{color, font, motion, size};
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
        _ => "-",
    }
}

/// Format a byte count as a compact human-readable size (B / KB / MB / GB), the
/// binary (1024) step the file managers use. `0` reads as "-" (size unknown).
fn human_size(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_owned();
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

/// The scope columns the model-stats card reports, left to right: their heading
/// and what each one covers.
///
/// They name the same three slices of the scene as the bounding box's own scope
/// dropdown ("All Meshes" / "Only Selection" / "Only Visible"), abbreviated to
/// fit a column heading.
const SCOPE_COLUMNS: [(&str, &str); 3] = [
    (
        "All",
        "Every mesh in the file - selected or not, hidden or not. These are the \
         counts the source file itself reports.",
    ),
    (
        "Sel",
        "Only what is selected in the Outliner: a node and everything under it, \
         or every triangle of a material slot. Empty when nothing is selected, \
         and a material has no authored vertex count to report.",
    ),
    (
        "Vis",
        concat!(
            "Only the meshes the Outliner is still showing. Identical to All ",
            "until you hide something - ",
            primary_key!(),
            "+click a row's eye to isolate one mesh."
        ),
    ),
];

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
    scoped_row(ui, "Draws", stats.draw_count, scoped, |scope| {
        Some(scope.draw_count)
    });
    scoped_row(ui, "Polys", stats.polygon_count, scoped, |scope| {
        Some(scope.polygon_count)
    });
    scoped_row(ui, "Tris", stats.triangle_count, scoped, |scope| {
        Some(scope.triangle_count)
    });
    scoped_row(ui, "Verts", stats.vertex_count, scoped, |scope| {
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
        scoped_row(ui, "GPU Verts", stats.gpu_vertex_count, scoped, |scope| {
            Some(scope.gpu_vertex_count)
        });
        splits_row(ui, stats.vertex_count, stats.gpu_vertex_count, scoped);
    }
    // The rest describe the file or the viewer, not a slice of geometry, so they
    // carry one value under the All column rather than repeating it three times.
    whole_model_row(ui, "UV Sets", &stats.uv_set_count.to_string());
    // Skeletal models only: an unrigged mesh shouldn't carry a permanent "0".
    if stats.bone_count > 0 {
        whole_model_row(ui, "Bones", &stats.bone_count.to_string());
    }
    // Animated files only, for the same reason.
    if stats.clip_count > 0 {
        whole_model_row(ui, "Clips", &stats.clip_count.to_string());
    }
    whole_model_row(ui, "Unit", &source_unit_label(stats.source_unit_meters));
    whole_model_row(ui, "FPS", &format!("{:.0}", state.fps));
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
            for (heading, tooltip) in SCOPE_COLUMNS.iter().rev() {
                let (rect, response) = value_cell(ui);
                paint_cell(ui, rect, heading, color::TEXT_MUTED);
                response.on_hover_text(*tooltip);
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
    label: &str,
    all: usize,
    scoped: ScopedStats,
    measure: impl Fn(&ScopeStats) -> Option<usize>,
) {
    cells_row(
        ui,
        label,
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
        "Vtx Splits",
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
fn whole_model_row(ui: &mut egui::Ui, label: &str, value: &str) {
    cells_row(
        ui,
        label,
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
fn cells_row(ui: &mut egui::Ui, label: &str, cells: [Cell; 3]) {
    // Only the columns that actually carry a value; a row with one is copied as
    // plainly as it reads on screen.
    let measured: Vec<String> = SCOPE_COLUMNS
        .iter()
        .zip(&cells)
        .filter_map(|((heading, _), cell)| match cell {
            Cell::Value(value) => Some(format!("{heading} {value}")),
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
            ui.label(mono_label(label, font::STATS, color::TEXT_MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed columns, no gap — see [`scope_heading_row`].
                ui.spacing_mut().item_spacing.x = 0.0;
                for cell in cells.iter().rev() {
                    let (rect, _) = value_cell(ui);
                    match cell {
                        Cell::Value(value) => paint_cell(ui, rect, value, color::TEXT_VALUE),
                        Cell::Unmeasured => paint_cell(ui, rect, "—", color::TEXT_MUTED),
                        Cell::NotScoped => {}
                    }
                }
            });
        })
        .response
        .rect;

    row_interaction(ui, rect, label, copy_text);
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
        (None, _) => "Source (nothing applied)".to_owned(),
        (Some(_), count) if count > 1 => format!("Processed · LOD {}", state.opt.active_lod),
        (Some(_), _) => "Processed".to_owned(),
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

    delta_row(ui, "Tris", stats.0, baseline.map(|source| source.triangles));
    // "Mesh Verts", matching the source card's row of that name: both count the
    // vertex buffer. Calling this one "Verts" put the same word on two different
    // measurements — the file's DCC count on one card and the buffer length on
    // the other — and the two cards read as contradicting each other.
    delta_row(
        ui,
        "Mesh Verts",
        stats.1,
        baseline.map(|source| source.vertices),
    );
    stat_row(ui, "Draws", &stats.2.to_string());

    ui.add_space(size::STATS_ROW_SPACING);
    // ACMR/ATVR describe vertex-cache behaviour, overdraw the pixel cost, and
    // overfetch the vertex-buffer read pattern. They are what makes the reorder
    // operations — which change nothing visible — measurable, so they carry their
    // change too: a reorder that moved nothing is a reorder worth removing.
    let compare = level.map(|_| source_metrics);
    metric_row(ui, "ACMR", metrics.acmr, compare.map(|m| m.acmr));
    metric_row(ui, "ATVR", metrics.atvr, compare.map(|m| m.atvr));
    metric_row(
        ui,
        "Overdraw",
        metrics.overdraw,
        compare.map(|m| m.overdraw),
    );
    metric_row(
        ui,
        "Overfetch",
        metrics.overfetch,
        compare.map(|m| m.overfetch),
    );

    // The simplifier's achieved error, shown only for a level that ran one — it
    // is meaningless (and always zero) for the unsimplified level 0. It has no
    // source counterpart to change against.
    if state.opt.active_lod > 0 && level.is_some() {
        stat_row(ui, "Error", &format!("{:.4}", metrics.simplify_error));
    }
}

/// A count row, with its change against the source when there is one to show.
fn delta_row(ui: &mut egui::Ui, label: &str, value: usize, source: Option<usize>) {
    value_row(
        ui,
        label,
        &value.to_string(),
        change(value as f32, source.map(|source| source as f32)),
    );
}

/// A measured ratio (ACMR / ATVR / overdraw / overfetch) and its change.
fn metric_row(ui: &mut egui::Ui, label: &str, value: f32, source: Option<f32>) {
    value_row(ui, label, &format!("{value:.2}"), change(value, source));
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

/// What each stats row means, in the artist's terms rather than the renderer's.
///
/// Keyed by the row's own label so the source and processed cards cannot end up
/// describing the same figure differently — every row whose label appears here
/// gets its tooltip automatically, on either card.
fn stat_tooltip(label: &str) -> Option<&'static str> {
    Some(match label {
        "Draws" => {
            "Draw calls this mesh costs - one per distinct material. Each is a \
             separate command to the GPU, so fewer is cheaper; merging materials \
             is what brings it down."
        }
        "Polys" => {
            "Polygons as authored in the source file: quads and n-gons counted \
             once each, before triangulation."
        }
        "Tris" => {
            "Triangles after triangulation - what the GPU actually rasterizes, \
             and what an engine's triangle budget counts."
        }
        "Verts" => {
            "Vertices as the source file counts them (control points) - the \
             number your DCC's stats show. It ignores the extra vertices that \
             hard edges and UV seams force the GPU to store."
        }
        "GPU Verts" => {
            "Vertices an engine would actually upload: one per unique combination \
             of position, normal, UVs and colour, per material. A vertex on a hard \
             edge or a UV seam is stored once per side."
        }
        "Vtx Splits" => {
            "How much larger the GPU vertex count is than the authored one - the \
             price of this asset's hard edges and UV seams. A few percent is \
             normal; hundreds of percent means per-face normals or heavily \
             fragmented UVs."
        }
        "UV Sets" => {
            "UV channels the mesh carries. A second set is usually a lightmap or \
             a detail-texture layout."
        }
        "Bones" => "Joints in the skeleton this mesh is bound to. Rigged meshes only.",
        "Clips" => {
            "Animation clips (FBX animation stacks) the file carries. Listed in the \
             Outliner's Animations tab."
        }
        "Unit" => {
            "The world unit the source file declared (centimetres, inches…). \
             Import normalises every model to metres; this is what the file itself \
             claimed, which is where scale mismatches come from."
        }
        "FPS" => {
            "Frames per second this preview is drawing at - a property of the \
             viewer and your GPU, not of the asset."
        }
        "ACMR" => {
            "Average Cache Miss Ratio: vertex-shader runs per triangle, simulated \
             against a 16-entry GPU vertex cache. 3.0 means no vertex is ever \
             reused; about 0.5 is the best a closed mesh can reach. Lower is \
             cheaper - Optimize Vertex Cache is the operation that moves it."
        }
        "ATVR" => {
            "Average Transformed Vertex Ratio: how many times the average vertex \
             gets shaded. 1.0 means each is shaded exactly once; 2.0 means the \
             mesh is transformed twice over. Unlike ACMR it does not shift with \
             the triangle count, so it is the fairer figure for judging one \
             mesh's ordering against itself."
        }
        "Overdraw" => {
            "Pixels shaded divided by pixels covered, measured from viewpoints \
             around the mesh. 1.0 means nothing is drawn over anything; higher \
             means the GPU shades pixels a later triangle then hides. Optimize \
             Overdraw trades cache behaviour for this."
        }
        "Overfetch" => {
            "Vertex-buffer bytes read divided by the buffer's size. 1.0 means each \
             byte is fetched once; higher means the index order jumps around and \
             the GPU re-reads the same memory. Optimize Vertex Fetch reorders the \
             buffer to bring it down."
        }
        "Error" => {
            "How far this LOD deviates from the mesh it was simplified from, as \
             the simplifier measured it - a fraction of the model's overall size \
             (or world units, under the absolute-error flag)."
        }
        _ => return None,
    })
}

/// Attach a row's explanation, when it has one.
fn with_tooltip(response: egui::Response, label: &str) {
    if let Some(text) = stat_tooltip(label) {
        response.on_hover_text(text);
    }
}

/// One stats row with an optional tinted change column to the right of the value.
///
/// The whole strip is one click target that copies the row to the clipboard, and
/// the widget that carries the row's explanation.
fn value_row(ui: &mut egui::Ui, label: &str, value: &str, delta: Option<(String, egui::Color32)>) {
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

    row_interaction(ui, rect, label, copy_text);
}

/// Turn a laid-out row into the card's one interactive element: click to copy it,
/// hover for its explanation. Shared by both cards' rows, single- and
/// multi-column, so a row behaves the same wherever it is drawn.
fn row_interaction(ui: &mut egui::Ui, rect: egui::Rect, label: &str, copy_text: String) {
    // Claimed *after* the labels so it sits above them in hit order — the row,
    // not a word in it, is what the pointer finds.
    let id = ui.id().with(("stat_row", label));
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
            response.on_hover_text("Copied to clipboard");
        }
        _ => with_tooltip(response, label),
    }
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
        None => "-".to_owned(),
    }
}

/// One stats row: label hugs the left edge, value right-aligns against the
/// panel's right edge so the numeric column reads as a tidy block.
fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    value_row(ui, label, value, None);
}
