//! `review-cli optimize`: the Opt workspace's operation stack over every model,
//! written out as FBX.
//!
//! The stack is a preset saved from the viewer, with `--lods`, `--reduce` and
//! the export options laid over it - or, with no preset, only those. Each model
//! then goes through exactly what the viewer's Export does: the preset's
//! per-object settings rebound to this model's objects by name, the run, and
//! `export_fbx`. What it writes, and where, is settled before the model is even
//! loaded, so a model whose files are already there costs nothing to skip and a
//! batch can never write one model over another - or over a model it is
//! reading.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use review_labels::opt::{
    explain_export_note, explain_opt_error, explain_opt_warning, rebind_notes,
};
use review_localization::tr;
use review_model::ENGINE_VERTEX_BYTES;
use review_optimize::{
    LodLevel, LodParams, OpKind, OptProgress, OptStack, OptStage, ProcessInput, ProcessedResult,
    ReduceParams, export::planned_files, export_fbx, preset, process_with_progress,
};
use serde_json::{Value, json};

use crate::args::{CommandName, Common, IfExists, Lods, OptimizeArgs};
use crate::console::{self, Bar};
use crate::inputs::{self, Model};
use crate::keys::cli as k;
use crate::load;
use crate::report::{Entry, Report, Status, figure};

/// How a model's share of the progress bar divides between its steps.
const LOADED: f32 = 0.2;
const PROCESSED: f32 = 0.9;

pub(crate) fn run(common: &Common, args: &OptimizeArgs) -> bool {
    let bar = Bar::new(common.progress_bar);
    let mut report = Report::start(common.report.clone(), CommandName::Optimize);
    work(common, args, &bar, &mut report);
    bar.clear();
    let ok = !report.has_problem() && report.failed() == 0;
    report.finish(ok) && ok
}

fn work(common: &Common, args: &OptimizeArgs, bar: &Bar, report: &mut Report) {
    let stack = match build_stack(args) {
        Ok(stack) => stack,
        Err(problem) => {
            console::warn(&problem);
            report.problem(problem);
            return;
        }
    };

    let models = inputs::gather(common, report);
    if models.is_empty() {
        return;
    }

    let count = models.len();
    console::say(&k::optimize_start(
        count as f64,
        args.out.display().to_string(),
    ));
    let mut batch = Batch {
        stack: &stack,
        args,
        sources: models
            .iter()
            .map(|model| inputs::canonical(&model.path))
            .collect(),
        claimed: HashMap::new(),
    };
    let (mut done, mut skipped) = (0usize, 0usize);
    let mut outputs: Vec<String> = Vec::new();
    for (index, model) in models.iter().enumerate() {
        console::say(&format!("[{}/{count}] {}", index + 1, model.label()));
        let progress = |fraction: f32| bar.set((index as f32 + fraction) / count as f32);
        let (status, entry, problem, written) = batch.optimize(model, &progress);
        match status {
            Status::Done => done += 1,
            Status::Skipped => skipped += 1,
            Status::Failed => {}
        }
        if let Some(problem) = &problem {
            match status {
                Status::Skipped => console::say(&format!("  {problem}")),
                _ => console::warn(&format!(
                    "  {}",
                    k::optimize_failed(model.label(), problem.clone())
                )),
            }
        }
        outputs.extend(written);
        report.add(status, entry, problem);
    }
    let failed = report.failed() as f64;
    console::say(&k::optimize_done(done as f64, failed, skipped as f64));
    report.total("written", json!(outputs.len()));
    report.total("outputs", json!(outputs));
}

/// The stack every model runs: the preset, if any, with the command line's
/// options laid over it.
fn build_stack(args: &OptimizeArgs) -> Result<OptStack, String> {
    let mut stack = match &args.preset {
        Some(path) => {
            let unreadable =
                |reason: String| k::optimize_preset_unreadable(path.display().to_string(), reason);
            let json =
                std::fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
            preset::from_json(&json).map_err(|error| unreadable(explain_opt_error(&error)))?
        }
        None => OptStack::default(),
    };
    match &args.lods {
        None => {}
        Some(Lods::None) => {
            for op in &mut stack.ops {
                if matches!(op.kind, OpKind::SimplifyLod(_)) {
                    op.enabled = false;
                }
            }
        }
        Some(Lods::Ratios(ratios)) => set_lods(&mut stack, ratios),
    }
    if let Some(ratio) = args.reduce {
        set_reduce(&mut stack, ratio);
    }
    if let Some(packaging) = args.packaging {
        stack.export.packaging = packaging;
    }
    if let Some(hierarchy) = args.hierarchy {
        stack.export.hierarchy = hierarchy;
    }
    if let Some(format) = args.format {
        stack.export.format = format;
    }
    stack.sanitize();
    if !stack.ops.iter().any(|op| op.enabled) {
        return Err(tr(k::OPTIMIZE_NOTHING_ENABLED).into_owned());
    }
    Ok(stack)
}

/// `--lods`: the stack's LOD chain becomes these levels. A level the preset
/// already has keeps its error limit; a new one gets the one before it, which
/// is what the Inspector's "add level" does.
fn set_lods(stack: &mut OptStack, ratios: &[f32]) {
    let chain = |existing: &[LodLevel]| {
        let mut levels: Vec<LodLevel> = Vec::with_capacity(ratios.len());
        for (index, &target_ratio) in ratios.iter().enumerate() {
            let target_error = existing.get(index).map_or_else(
                || {
                    levels
                        .last()
                        .map_or(LodLevel::default().target_error, |previous| {
                            previous.target_error
                        })
                },
                |level| level.target_error,
            );
            levels.push(LodLevel {
                target_ratio,
                target_error,
            });
        }
        levels
    };
    if let Some(op) = stack
        .ops
        .iter_mut()
        .find(|op| matches!(op.kind, OpKind::SimplifyLod(_)))
    {
        op.enabled = true;
        if let OpKind::SimplifyLod(params) = &mut op.kind {
            params.levels = chain(&params.levels);
        }
        return;
    }
    let defaults = LodParams::default();
    let params = LodParams {
        levels: chain(&defaults.levels),
        ..defaults
    };
    insert(stack, OpKind::SimplifyLod(params), false);
}

/// `--reduce`: the stack's Reduce takes this target, or one is added.
fn set_reduce(stack: &mut OptStack, ratio: f32) {
    if let Some(op) = stack
        .ops
        .iter_mut()
        .find(|op| matches!(op.kind, OpKind::Reduce(_)))
    {
        op.enabled = true;
        if let OpKind::Reduce(params) = &mut op.kind {
            params.target.target_ratio = ratio;
        }
        return;
    }
    let mut params = ReduceParams::default();
    params.target.target_ratio = ratio;
    insert(stack, OpKind::Reduce(params), true);
}

/// Add an operation the command line asked for where it does what it says: in
/// front of the reorders (vertex cache, overdraw, fetch), which only pay off
/// when nothing changes the triangles after them - and, for a Reduce, in front
/// of the LOD chain too, so every level is made from the reduced model.
fn insert(stack: &mut OptStack, kind: OpKind, before_lods: bool) {
    stack.push_op(kind);
    let added = stack.ops.len() - 1;
    let target = stack.ops[..added].iter().position(|op| {
        matches!(
            op.kind,
            OpKind::VertexCache | OpKind::Overdraw { .. } | OpKind::VertexFetch
        ) || (before_lods && matches!(op.kind, OpKind::SimplifyLod(_)))
    });
    if let Some(target) = target {
        stack.reorder(added, target as isize - added as isize);
    }
}

/// How many levels a run of `stack` produces: level 0, plus the LOD chain's.
fn level_count(stack: &OptStack) -> usize {
    stack
        .lod_params()
        .map_or(1, |params| 1 + params.levels.len())
}

/// What a whole run shares between its models.
struct Batch<'a> {
    stack: &'a OptStack,
    args: &'a OptimizeArgs,
    /// Every model being read, resolved: none of them may be written over.
    sources: HashSet<PathBuf>,
    /// Every file a model of this run is to write, resolved, with that model.
    claimed: HashMap<PathBuf, String>,
}

impl Batch<'_> {
    /// Optimize one model: its status, its entry, what went wrong, and the
    /// files it wrote.
    fn optimize(
        &mut self,
        model: &Model,
        progress: &dyn Fn(f32),
    ) -> (Status, Entry, Option<String>, Vec<String>) {
        let mut entry = Entry::new(&model.label());
        let destination = self.destination(model);
        let planned = planned_files(0..level_count(self.stack), &destination, &self.stack.export);
        if let Err((status, problem)) = self.claim(model, &planned) {
            return (status, entry, Some(problem), Vec::new());
        }

        let loaded = match load::load(&model.path, &|fraction| progress(fraction * LOADED)) {
            Ok(loaded) => loaded,
            Err(problem) => return (Status::Failed, entry, Some(problem), Vec::new()),
        };
        for warning in &loaded.warnings {
            warn(&mut entry, warning.clone());
        }

        // The preset's per-object settings name objects of the model it was
        // saved from; rebind them to this one's by name, as Load preset does.
        let mut stack = self.stack.clone();
        let rebind = stack.rebind_to_model(&loaded.model.nodes);
        if !self.stack.overrides.is_empty() {
            entry.set(
                "overrides",
                json!({
                    "rebound": rebind.rebound,
                    "by_position": rebind.by_position,
                    "dropped": rebind.dropped,
                }),
            );
            for note in rebind_notes(rebind) {
                warn(&mut entry, note);
            }
        }

        let enabled: Vec<&OpKind> = stack
            .ops
            .iter()
            .filter(|op| op.enabled)
            .map(|op| &op.kind)
            .collect();
        let on_progress = |update: OptProgress<'_>| {
            progress(LOADED + (PROCESSED - LOADED) * run_fraction(&enabled, update));
        };
        let result = process_with_progress(
            ProcessInput {
                model: &loaded.model,
                stack: &stack,
                render_vertex_size: ENGINE_VERTEX_BYTES,
                // Nothing is hidden: there is no Outliner. A bake that should
                // leave an object out is a preset with that object excluded.
                hidden_nodes: &[],
                extras: loaded.extras.as_ref(),
            },
            &on_progress,
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                return (
                    Status::Failed,
                    entry,
                    Some(explain_opt_error(&error)),
                    Vec::new(),
                );
            }
        };
        for warning in &result.warnings {
            warn(&mut entry, explain_opt_warning(warning));
        }
        record_levels(&mut entry, &result);

        // The chain the run made is the one the plan was for; were it ever
        // not, its files are claimed and checked like the planned ones.
        let files = planned_files(
            result.lods.iter().map(|lod| lod.level),
            &destination,
            &stack.export,
        );
        if files != planned
            && let Err((status, problem)) = self.claim(model, &files)
        {
            return (status, entry, Some(problem), Vec::new());
        }

        if let Some(folder) = destination.parent()
            && let Err(error) = std::fs::create_dir_all(folder)
        {
            return (Status::Failed, entry, Some(error.to_string()), Vec::new());
        }
        let exported = export_fbx(
            &result.lods,
            &loaded.model,
            loaded.extras.as_ref(),
            &destination,
            &stack.export,
        );
        let exported = match exported {
            Ok(exported) => exported,
            Err(error) => {
                return (
                    Status::Failed,
                    entry,
                    Some(explain_opt_error(&error)),
                    Vec::new(),
                );
            }
        };
        let written: Vec<String> = exported
            .files
            .iter()
            .map(|file| file.display().to_string())
            .collect();
        for file in &written {
            console::say(&format!("  {}", k::optimize_wrote(file.clone())));
        }
        for note in &exported.notes {
            let text = explain_export_note(note);
            console::say(&format!("  {}", k::note(text.clone())));
            entry.notice(text);
        }
        entry.set("outputs", json!(written));
        progress(1.0);
        (Status::Done, entry, None, written)
    }

    /// Where `model` is written: its place under `--out`, its name with the
    /// suffix, and the `.fbx` extension whatever the source's case.
    fn destination(&self, model: &Model) -> PathBuf {
        let stem = model
            .relative
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{stem}{}.fbx", self.args.suffix);
        self.args.out.join(model.relative.with_file_name(name))
    }

    /// Claim `files` for `model`, or say why it cannot have them: one is a
    /// model of this run, another model of this run is to write one, or one is
    /// already there and `--if-exists` does not say to replace it.
    fn claim(&mut self, model: &Model, files: &[PathBuf]) -> Result<(), (Status, String)> {
        let resolved: Vec<PathBuf> = files
            .iter()
            .map(|file| inputs::canonical_destination(file))
            .collect();
        for (file, resolved) in files.iter().zip(&resolved) {
            let shown = file.display().to_string();
            if self.sources.contains(resolved) {
                return Err((Status::Failed, k::optimize_is_a_model(shown)));
            }
            if let Some(other) = self.claimed.get(resolved)
                && *other != model.label()
            {
                return Err((
                    Status::Failed,
                    k::optimize_written_twice(shown, other.clone()),
                ));
            }
        }
        for resolved in resolved {
            self.claimed.insert(resolved, model.label());
        }
        let Some(existing) = files.iter().find(|file| file.exists()) else {
            return Ok(());
        };
        let shown = existing.display().to_string();
        match self.args.if_exists {
            IfExists::Replace => Ok(()),
            IfExists::Skip => Err((Status::Skipped, k::optimize_skipped(shown))),
            IfExists::Fail => Err((Status::Failed, k::optimize_exists(shown))),
        }
    }
}

/// Say a warning under the model's name, and keep it in its entry.
fn warn(entry: &mut Entry, text: String) {
    console::warn(&format!("  {}", k::warning(text.clone())));
    entry.notice(text);
}

/// How far through the stack a run is, `0.0..=1.0`, pieced together from the
/// stage it reports and which of the enabled operations it is in.
fn run_fraction(enabled: &[&OpKind], update: OptProgress<'_>) -> f32 {
    let within = update.fraction().unwrap_or(0.0);
    match update.stage {
        OptStage::Preparing => 0.0,
        OptStage::Measuring => 0.05,
        OptStage::Operation => {
            let index = update.op.and_then(|op| {
                enabled
                    .iter()
                    .position(|kind| std::ptr::eq(*kind, op))
                    .or_else(|| enabled.iter().position(|kind| *kind == op))
            });
            let share = 0.85 / enabled.len().max(1) as f32;
            0.1 + share * (index.unwrap_or(0) as f32 + within)
        }
        OptStage::Assembling => 0.95 + 0.05 * within,
    }
}

/// Each level's counts and measured figures, against the source's: what the
/// processed stats card shows, and the lines under the model's name.
fn record_levels(entry: &mut Entry, result: &ProcessedResult) {
    entry.set(
        "source",
        json!({
            "triangles": result.source.triangles,
            "vertices": result.source.vertices,
            "acmr": figure(result.source_metrics.acmr),
            "atvr": figure(result.source_metrics.atvr),
            "overdraw": figure(result.source_metrics.overdraw),
            "overfetch": figure(result.source_metrics.overfetch),
        }),
    );
    let mut levels = Vec::with_capacity(result.lods.len());
    for lod in &result.lods {
        let stats = lod.model.stats;
        console::say(&format!(
            "  {}",
            k::optimize_level(
                f64::from(lod.level as u32),
                stats.triangle_count as f64,
                stats.vertex_count as f64,
            )
        ));
        levels.push(json!({
            "level": lod.level,
            "polygons": stats.polygon_count,
            "triangles": stats.triangle_count,
            "vertices": stats.vertex_count,
            "acmr": figure(lod.metrics.acmr),
            "atvr": figure(lod.metrics.atvr),
            "overdraw": figure(lod.metrics.overdraw),
            "overfetch": figure(lod.metrics.overfetch),
            "simplify_error": if lod.metrics.simplified {
                figure(lod.metrics.simplify_error)
            } else {
                Value::Null
            },
        }));
    }
    entry.set("levels", json!(levels));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(lods: Option<Lods>, reduce: Option<f32>) -> OptimizeArgs {
        OptimizeArgs {
            out: PathBuf::from("out"),
            preset: None,
            lods,
            reduce,
            packaging: None,
            hierarchy: None,
            format: None,
            suffix: String::new(),
            if_exists: IfExists::Fail,
        }
    }

    fn kinds(stack: &OptStack) -> Vec<&'static str> {
        stack.ops.iter().map(|op| op.kind.label()).collect()
    }

    #[test]
    fn lods_alone_make_a_chain_with_one_error_limit() {
        let stack = build_stack(&args(Some(Lods::Ratios(vec![0.5, 0.25, 0.1, 0.05])), None))
            .expect("a stack");
        let params = stack.lod_params().expect("a LOD op");
        let levels: Vec<(f32, f32)> = params
            .levels
            .iter()
            .map(|level| (level.target_ratio, level.target_error))
            .collect();
        // The first three keep the defaults' limits; the fourth takes the third's.
        assert_eq!(
            levels,
            [(0.5, 0.01), (0.25, 0.01), (0.1, 0.01), (0.05, 0.01)]
        );
        assert_eq!(level_count(&stack), 5);
    }

    #[test]
    fn an_added_reduce_goes_before_the_chain_and_an_added_chain_before_the_reorders() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::SimplifyLod(LodParams::default()));
        stack.push_op(OpKind::VertexCache);
        set_reduce(&mut stack, 0.5);
        assert_eq!(
            kinds(&stack)[0],
            OpKind::Reduce(ReduceParams::default()).label()
        );

        let mut stack = OptStack::default();
        stack.push_op(OpKind::VertexCache);
        set_lods(&mut stack, &[0.5]);
        assert_eq!(
            kinds(&stack),
            [
                OpKind::SimplifyLod(LodParams::default()).label(),
                OpKind::VertexCache.label()
            ]
        );
    }

    #[test]
    fn no_lods_switches_the_presets_chain_off() {
        let mut stack = OptStack::default();
        stack.push_op(OpKind::SimplifyLod(LodParams::default()));
        stack.push_op(OpKind::VertexCache);
        let json = preset::to_json(&stack).expect("serializes");
        let path =
            std::env::temp_dir().join(format!("review-cli-preset-{}.json", std::process::id()));
        std::fs::write(&path, json).expect("written");
        let mut with_preset = args(Some(Lods::None), None);
        with_preset.preset = Some(path.clone());
        let stack = build_stack(&with_preset).expect("a stack");
        assert!(stack.lod_params().is_none());
        assert_eq!(level_count(&stack), 1);
        let _ = std::fs::remove_file(path);
    }
}
