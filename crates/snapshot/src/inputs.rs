//! Turning `INPUT` arguments into subjects: a model to draw and the nodes of it
//! to leave out.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use review_model::{Bounds, ModelData, naming::node_lod};
use review_optimize::{LodLevel, LodParams, OpKind, OptStack, ProcessInput};

use crate::cli::{InputSpec, LevelSpec, StackSpec};

/// One row of a sheet: a model, and the mesh nodes of it hidden from view.
pub struct Subject {
    pub label: String,
    pub model: Arc<ModelData>,
    /// Sorted node indices, exactly what the viewer's Outliner eye toggles.
    pub hidden: Vec<u32>,
}

impl Subject {
    pub fn bounds(&self) -> Option<Bounds> {
        self.model.visible_bounds(&self.hidden)
    }

    /// Triangles and GPU vertices (unique per draw group) of what is shown.
    pub fn counts(&self) -> (usize, usize) {
        self.model
            .mesh_group_stats()
            .iter()
            .filter(|group| self.hidden.binary_search(&group.node).is_err())
            .fold((0, 0), |(triangles, vertices), group| {
                (
                    triangles + group.triangle_count,
                    vertices + group.gpu_vertex_count,
                )
            })
    }
}

/// Load (and process) one input into the subjects it names.
pub fn load(spec: &InputSpec) -> Result<Vec<Subject>> {
    let file = spec.path.file_name().map_or_else(
        || spec.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let model = review_import::load_model(&spec.path)
        .with_context(|| format!("loading {}", spec.path.display()))?;

    let Some(stack_spec) = &spec.stack else {
        return Ok(split_by_lod_suffix(&file, Arc::new(model), spec.level));
    };

    let stack = match stack_spec {
        StackSpec::Preset(path) => {
            let json = std::fs::read_to_string(path)
                .with_context(|| format!("reading preset {}", path.display()))?;
            review_optimize::preset::from_json(&json)?
        }
        StackSpec::Lods(ratios, error) => {
            let mut stack = OptStack::default();
            stack.push_op(OpKind::SimplifyLod(LodParams {
                levels: ratios
                    .iter()
                    .map(|&target_ratio| LodLevel {
                        target_ratio,
                        target_error: error.unwrap_or(LodLevel::default().target_error),
                    })
                    .collect(),
                ..LodParams::default()
            }));
            stack
        }
    };
    let started = std::time::Instant::now();
    let result = review_optimize::process(ProcessInput {
        model: &model,
        stack: &stack,
        render_vertex_size: review_render::scene_vertex_size(),
        hidden_nodes: &[],
        extras: None,
    })
    .with_context(|| format!("processing {file}"))?;
    println!(
        "processed {file} in {:.2} s: {} level(s)",
        started.elapsed().as_secs_f32(),
        result.lods.len()
    );
    for warning in &result.warnings {
        println!("  warning: {warning:?}");
    }

    let stack_name = match stack_spec {
        StackSpec::Preset(path) => path.file_stem().map_or_else(
            || "preset".into(),
            |stem| stem.to_string_lossy().into_owned(),
        ),
        StackSpec::Lods(..) => "lods".into(),
    };
    let wanted = |level: usize| match spec.level {
        LevelSpec::Default | LevelSpec::All => true,
        LevelSpec::One(one) => level == one as usize,
    };
    let subjects: Vec<Subject> = result
        .lods
        .into_iter()
        .filter(|lod| wanted(lod.level))
        .map(|lod| Subject {
            label: format!("{file} @{stack_name} LOD{}", lod.level),
            model: Arc::new(lod.model),
            hidden: Vec::new(),
        })
        .collect();
    if subjects.is_empty() {
        bail!("{file}: the stack produced no level matching the selector");
    }
    Ok(subjects)
}

/// A file's subjects by its `_LOD<n>` node suffixes: the selected level, every
/// level, or — with no selector — the whole file at once.
fn split_by_lod_suffix(file: &str, model: Arc<ModelData>, level: LevelSpec) -> Vec<Subject> {
    let mesh_nodes: Vec<u32> = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.mesh_part.is_some())
        .map(|(index, _)| index as u32)
        .collect();
    let levels: BTreeSet<u32> = mesh_nodes
        .iter()
        .filter_map(|&node| node_lod(&model, node))
        .collect();
    let only = |lod: u32| Subject {
        label: format!("{file} LOD{lod}"),
        model: model.clone(),
        hidden: mesh_nodes
            .iter()
            .copied()
            .filter(|&node| node_lod(&model, node) != Some(lod))
            .collect(),
    };
    match level {
        LevelSpec::One(lod) => vec![only(lod)],
        LevelSpec::All if !levels.is_empty() => levels.into_iter().map(only).collect(),
        LevelSpec::All | LevelSpec::Default => vec![Subject {
            label: file.to_owned(),
            model,
            hidden: Vec::new(),
        }],
    }
}
