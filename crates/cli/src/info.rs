//! `review-cli info`: what each model holds, as the stats card shows it.
//!
//! The source's own counts (invariant 5): polygons, triangles and vertices as
//! the DCC counts them, and GPU vertices as an engine would build them - never
//! the viewer's corner-split buffer. A single model's figures are also the
//! report's top-level totals, so a tool reads `triangles` straight off it.

use review_labels::units::{match_known_unit, meters_value};
use review_localization::tr;
use serde_json::json;

use crate::args::{CommandName, Common};
use crate::console::{self, Bar};
use crate::inputs;
use crate::keys::cli as k;
use crate::load;
use crate::report::{Entry, Report, Status, figure};

pub(crate) fn run(common: &Common) -> bool {
    let bar = Bar::new(common.progress_bar);
    let mut report = Report::start(common.report.clone(), CommandName::Info);
    work(common, &bar, &mut report);
    bar.clear();
    let ok = !report.has_problem() && report.failed() == 0;
    report.finish(ok) && ok
}

fn work(common: &Common, bar: &Bar, report: &mut Report) {
    let models = inputs::gather(common, report);
    let count = models.len();
    let mut done = 0usize;
    let mut totals = [0usize; 4];
    for (index, model) in models.iter().enumerate() {
        console::say(&format!("[{}/{count}] {}", index + 1, model.label()));
        let progress = |fraction: f32| bar.set((index as f32 + fraction) / count as f32);
        let mut entry = Entry::new(&model.label());
        let loaded = match load::load(&model.path, &progress) {
            Ok(loaded) => loaded,
            Err(problem) => {
                console::warn(&format!(
                    "  {}",
                    k::info_failed(model.label(), problem.clone())
                ));
                report.add(Status::Failed, entry, Some(problem));
                continue;
            }
        };
        for warning in &loaded.warnings {
            console::warn(&format!("  {}", k::warning(warning.clone())));
            entry.notice(warning.clone());
        }
        let model_data = &loaded.model;
        let stats = model_data.stats;
        console::say(&format!(
            "  {}",
            k::info_counts(
                stats.gpu_vertex_count as f64,
                stats.polygon_count as f64,
                stats.triangle_count as f64,
                stats.vertex_count as f64,
            )
        ));
        console::say(&format!(
            "  {}",
            k::info_parts(
                stats.bone_count as f64,
                stats.clip_count as f64,
                stats.draw_count as f64,
                stats.material_count as f64,
                stats.uv_set_count as f64,
            )
        ));
        // The stats card's Unit row: the file's authored unit, by its symbol
        // when it is a known one. A file that declares none has no row.
        let meters = stats.source_unit_meters;
        if meters > 0.0 {
            let unit = match match_known_unit(meters) {
                Some(unit) => tr(unit.symbol).into_owned(),
                None => meters_value(format!("{meters:.4}")),
            };
            console::say(&format!("  {}", k::info_unit(unit)));
        }

        entry.set(
            "counts",
            json!({
                "polygons": stats.polygon_count,
                "triangles": stats.triangle_count,
                "vertices": stats.vertex_count,
                "gpu_vertices": stats.gpu_vertex_count,
                "draws": stats.draw_count,
                "materials": stats.material_count,
                "uv_sets": stats.uv_set_count,
                "bones": stats.bone_count,
                "clips": stats.clip_count,
                "nodes": model_data.nodes.len(),
            }),
        );
        entry.set("unit_meters", figure(meters));
        entry.set(
            "up_axis",
            json!(
                loaded
                    .extras
                    .as_ref()
                    .map(|extras| format!("{:?}", extras.scene.axes[1]))
            ),
        );
        report.add(Status::Done, entry, None);
        done += 1;
        for (total, value) in totals.iter_mut().zip([
            stats.polygon_count,
            stats.triangle_count,
            stats.vertex_count,
            stats.gpu_vertex_count,
        ]) {
            *total += value;
        }
        progress(1.0);
    }
    if count > 0 {
        console::say(&k::info_done(done as f64, report.failed() as f64));
    }
    report.total("polygons", json!(totals[0]));
    report.total("triangles", json!(totals[1]));
    report.total("vertices", json!(totals[2]));
    report.total("gpu_vertices", json!(totals[3]));
}
