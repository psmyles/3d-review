//! The orchestrator: one `SceneData` out of a LOD chain and its capture.
//!
//! Call order is load-bearing. Every property must be written before any curve
//! binds to it, so nodes, attributes, materials, textures, channels and layers
//! are built first and the animation last.

use std::collections::HashMap;

use review_model::{ModelData, SourceExtras};

use crate::OptError;
use crate::process::ProcessedLod;
use crate::stack::{ExportOptions, HierarchyMode};

use super::*;

/// Build the payload for one output file.
pub(crate) fn build_scene(
    lods: &[ProcessedLod],
    source: &ModelData,
    extras: Option<&SourceExtras>,
    options: &ExportOptions,
    report: &mut ExportReport,
) -> Result<SceneData, OptError> {
    let mut scene = SceneData::new(UnitScale::from_source(source.stats.source_unit_meters));

    if let Some(extras) = extras {
        build_settings(&mut scene, extras);
        let texture_map = build_textures(&mut scene, extras, report);
        build_materials_from_extras(&mut scene, source, extras, &texture_map);
        scene.texture_map = texture_map;
    } else {
        build_materials_from_viewer(&mut scene, source);
    }

    // Whether more than one level shares this file decides whether meshes need a
    // `_LOD<n>` suffix to stay distinguishable.
    let suffix_levels = lods.len() > 1;

    // Source node index → scene node index for every node already emitted into
    // this file, so meshes sharing ancestors share one chain (see `place_node`).
    let mut placed = HashMap::new();

    // With the capture, the whole authored graph goes out first — every node,
    // mesh-bearing or not — so lights, cameras, empties and bones survive.
    if let Some(extras) = extras
        && options.hierarchy == HierarchyMode::Rebuild
    {
        place_graph(&mut scene, &mut placed, source, extras);
        build_poses(&mut scene, &mut placed, source, extras, report);
    }
    let deforms = source.skin.is_some() || source.morph.is_some();
    if deforms && options.hierarchy == HierarchyMode::FlatBaked {
        report.notes.push(
            "Skinning and blend shapes were not written: a flat hierarchy bakes the geometry \
             into world space, which has no bones to bind to."
                .to_owned(),
        );
    }

    for lod in lods {
        let groups = group_by_node(&lod.model);
        for group in groups {
            let node_index = match (extras, options.hierarchy) {
                (Some(extras), HierarchyMode::Rebuild) => {
                    place_level_node(&mut scene, &placed, source, extras, &group, lod.level)
                }
                _ => {
                    let (node_index, notes) = place_node(
                        &mut scene,
                        &mut placed,
                        source,
                        &group,
                        options.hierarchy,
                        lod.level,
                    );
                    report.notes.extend(notes);
                    node_index
                }
            };

            let (mut mesh, notes) = build_mesh(
                &lod.model,
                &lod.carry,
                &group,
                node_index,
                suffix_levels.then_some(lod.level),
                options.hierarchy,
                source,
                extras,
                scene.unit,
            );
            if let Some(part) = group
                .source_node
                .zip(extras)
                .and_then(|(index, extras)| extras.mesh_of_node(index as u32))
            {
                mesh.props = scene.push_props(&part.props);
            }
            let mesh_name = mesh.name.to_string_lossy().into_owned();
            if notes.polygons_lost {
                report.notes.push(format!(
                    "LOD {}: '{mesh_name}' was written as triangles — the stack rebuilt its geometry.",
                    lod.level
                ));
            } else if notes.triangles_rebuilt > 0 {
                report.notes.push(format!(
                    "LOD {}: {} triangle(s) of '{mesh_name}' were written as triangles — the stack \
                     rebuilt them.",
                    lod.level, notes.triangles_rebuilt
                ));
            }
            if options.hierarchy == HierarchyMode::Rebuild {
                build_deform(
                    &mut scene,
                    &mut mesh,
                    &mut placed,
                    lod,
                    &group,
                    source,
                    extras,
                    report,
                );
            }
            report.triangle_count += mesh.triangle_count;
            scene.meshes.push(mesh);
        }
    }

    if let Some(extras) = extras
        && options.hierarchy == HierarchyMode::Rebuild
    {
        build_display_layers(&mut scene, &placed, extras);
        build_selection_sets(&mut scene, &placed, source, extras, report);
        build_animation(&mut scene, &placed, extras, report);
    } else if extras.is_some_and(|extras| !extras.animations.is_empty()) {
        report.notes.push(
            "Animation was not written: a flat hierarchy has no nodes for the curves to drive."
                .to_owned(),
        );
    }

    scene.mesh_count_into(report);
    if scene.meshes.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    Ok(scene)
}
