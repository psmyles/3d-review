//! Stamp sample review comments into an FBX — the files used to try the viewer's
//! comment display and to check other applications against.
//!
//!   cargo run -p review-annotate --example stamp -- <in.fbx> <out.fbx> [--hidden]
//!
//! Every mesh object gets a thread pinned to the middle polygon of its mesh; the
//! first also gets a reply, the second is resolved, and on an animated file the
//! third is about the first clip's opening frames. A point in the scene above the
//! model gets a pin of its own, and the root-most object a note about the whole
//! file.

use std::collections::HashMap;

use glam::Vec3;
use review_annotate::codec::{PROPERTY, encode};
use review_annotate::fbx::{self, Edit};
use review_annotate::mapping::{ImportedNode, map_nodes};
use review_annotate::thread::{
    Anchor, AnchorValue, FrameRange, Message, Scope, Status, Thread, Topology,
};
use review_model::extras::Synthetic;

fn message(author: &str, time: &str, text: &str) -> Message {
    Message {
        author: author.to_owned(),
        time: time.to_owned(),
        text: text.to_owned(),
        extra: Default::default(),
    }
}

fn thread(id: usize, scope: Scope, anchor: Option<Anchor>, messages: Vec<Message>) -> Thread {
    Thread {
        id: format!("{:016x}", 0x5a3e_0000_0000_0000_u64 + id as u64),
        status: Status::Open,
        scope,
        anchor: anchor.map(AnchorValue::Known),
        frames: None,
        view: None,
        messages,
        extra: Default::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        return Err("usage: stamp <in.fbx> <out.fbx> [--hidden]".into());
    };
    let hidden = rest.iter().any(|arg| arg == "--hidden");
    let bytes = std::fs::read(input)?;
    let scan = fbx::scan(&bytes, PROPERTY)?;
    let (model, extras) = review_import::load_model_full(input)?;
    let extras = extras.ok_or("no source properties captured")?;
    let nodes: Vec<ImportedNode<'_>> = model
        .nodes
        .iter()
        .zip(&extras.nodes)
        .map(|(node, extra)| ImportedNode {
            name: &node.name,
            parent: node.parent,
            real: extra.synthetic == Synthetic::None,
        })
        .collect();
    let mapped = map_nodes(&scan.models, &nodes);

    let mut by_object: HashMap<usize, Vec<Thread>> = HashMap::new();
    let mut next = 0;
    let mut meshes = 0;
    for (index, node) in model.nodes.iter().enumerate() {
        let (Some(_), Some(object)) = (node.mesh_part, mapped[index]) else {
            continue;
        };
        // The node's polygons as runs of triangles, in file order.
        let mut faces: Vec<(usize, usize)> = Vec::new();
        let mut last = None;
        for (triangle, (&owner, &face)) in model
            .triangles
            .node
            .iter()
            .zip(&model.triangles.to_face)
            .enumerate()
        {
            if owner as usize != index {
                continue;
            }
            match faces.last_mut() {
                Some(run) if last == Some(face) => run.1 += 1,
                _ => faces.push((triangle, 1)),
            }
            last = Some(face);
        }
        let Some(&(first, _)) = faces.get(faces.len() / 2) else {
            continue;
        };
        let corners = &model.indices[first * 3..first * 3 + 3];
        let centroid = corners
            .iter()
            .map(|&corner| model.vertices[corner as usize].position)
            .sum::<Vec3>()
            / 3.0;
        let local = node.transform.inverse().transform_point3(centroid);
        let mut messages = vec![message(
            "Ana",
            "2026-10-08T12:00:00Z",
            &format!(
                "Check the shading on {} here — \"seam\" & stretch ✓",
                node.name
            ),
        )];
        if meshes == 0 {
            messages.push(message(
                "Ben",
                "2026-10-09T09:30:00Z",
                "Moved the seam under the strap.",
            ));
        }
        let mut entry = thread(
            next,
            Scope::Object,
            Some(Anchor::Surface {
                face: (faces.len() / 2) as u32,
                tri: 0,
                bary: [1.0 / 3.0; 3],
                local: local.to_array(),
                topo: Topology {
                    polys: faces.len() as u64,
                    verts: node.source_vertex_count as u64,
                },
            }),
            messages,
        );
        if meshes == 1 {
            entry.status = Status::Resolved;
        }
        if meshes == 2
            && let Some(clip) = model.animations.first()
        {
            entry.frames = Some(FrameRange {
                clip: clip.name.clone(),
                start: 0,
                end: 10,
            });
        }
        by_object.entry(object).or_default().push(entry);
        next += 1;
        meshes += 1;
    }

    if let Some(root) = review_annotate::comments::file_note_host(&scan) {
        let above = model.bounds.map_or(Vec3::Y, |bounds| {
            Vec3::new(bounds.center().x, bounds.max.y * 1.2, bounds.center().z)
        });
        let notes = by_object.entry(root).or_default();
        notes.push(thread(
            next,
            Scope::Object,
            Some(Anchor::World {
                pos: above.to_array(),
            }),
            vec![message(
                "Ana",
                "2026-10-08T12:05:00Z",
                "Too much empty space above the model.",
            )],
        ));
        notes.push(thread(
            next + 1,
            Scope::File,
            None,
            vec![message(
                "Ana",
                "2026-10-08T12:10:00Z",
                "Overall: close; a few seams to fix.",
            )],
        ));
    }

    let edits: Vec<Edit> = by_object
        .iter()
        .map(|(&object, threads)| Edit {
            model: scan.models[object].id,
            value: Some(encode(threads, &[])),
            hidden,
        })
        .collect();
    std::fs::write(output, fbx::patch(&bytes, PROPERTY, &edits)?)?;
    let count: usize = by_object.values().map(Vec::len).sum();
    println!(
        "{count} threads on {} objects ({})",
        edits.len(),
        if hidden { "UH" } else { "U" }
    );
    Ok(())
}
