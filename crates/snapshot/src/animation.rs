//! Posing a subject on one frame of one of its clips, with the skeleton and the
//! animation curves over it: what the viewer's transport, its curve toggle and
//! an Outliner selection draw, for looking at them without a window.

use anyhow::{Result, bail};
use review_model::{AnimContext, Bounds, DeformPose, ModelData, NodeKind, NodePath, Pose, anim};

use crate::cli::ClipSpec;

/// A subject's model posed on one frame, and what the overlays draw over it.
pub struct Posed {
    pub deform: DeformPose,
    /// The clip's whole motion — what the viewer frames on while a clip is
    /// selected, so the views do too.
    pub envelope: Option<Bounds>,
    /// The clip's motion paths; empty without `--curves`.
    pub paths: Vec<NodePath>,
    /// Which of `paths` the selection leaves showing, by the viewer's own rule.
    pub shown: Vec<u32>,
    /// The `--select` names, split as an Outliner click would split them.
    pub selected_bones: Vec<u32>,
    pub selected_nodes: Vec<u32>,
}

/// Pose `model` on `clip`, sample its curves when asked, and resolve `select`
/// to node indices. `hidden` are the subject's hidden nodes, which take their
/// curves with them as they do in the viewer.
pub fn pose(
    model: &ModelData,
    clip: ClipSpec,
    curves: bool,
    select: &[String],
    hidden: &[u32],
) -> Result<Posed> {
    let Some(animation) = model.animations.get(clip.index) else {
        bail!(
            "--clip {}: the model has {} clips",
            clip.index,
            model.animations.len()
        );
    };
    let ctx = AnimContext::new(model);
    let frame = clip
        .frame
        .unwrap_or(0)
        .min(animation.frame_count().saturating_sub(1));
    let mut pose = Pose::new(model);
    anim::evaluate_pose(
        model,
        &ctx,
        Some(animation),
        animation.frame_time(frame),
        &mut pose,
    );
    let mut deform = DeformPose::default();
    anim::build_palette(model, &ctx, &pose, &mut deform);
    println!(
        "posed on clip {} '{}', frame {frame} of {}",
        clip.index,
        animation.name,
        animation.frame_count()
    );

    let mut selected_bones = Vec::new();
    let mut selected_nodes = Vec::new();
    for name in select {
        let mut found = false;
        for (index, node) in model.nodes.iter().enumerate() {
            if &node.name == name {
                found = true;
                if node.kind == NodeKind::Bone {
                    selected_bones.push(index as u32);
                } else {
                    selected_nodes.push(index as u32);
                }
            }
        }
        if !found {
            bail!("--select: no node is named '{name}'");
        }
    }
    // The viewer never holds both at once — a click on either kind clears the
    // other — so a mixed list keeps the bones.
    if !selected_bones.is_empty() {
        selected_nodes.clear();
    }
    selected_bones.sort_unstable();
    selected_bones.dedup();
    selected_nodes.sort_unstable();
    selected_nodes.dedup();

    let paths = if curves {
        anim::clip_paths(model, &ctx, animation)
    } else {
        Vec::new()
    };
    let shown = anim::selected_paths(model, &paths, &selected_bones, &selected_nodes, hidden);
    if curves {
        println!("{} curves, {} shown", paths.len(), shown.len());
    }

    Ok(Posed {
        deform,
        envelope: anim::clip_bounds(model, &ctx, animation),
        paths,
        shown,
        selected_bones,
        selected_nodes,
    })
}
