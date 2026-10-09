//! Review comments: read from the opened FBX on the import worker, mapped onto
//! the scene's nodes, given a world position every frame for their pins, and
//! "gone to" when the chrome asks.
//!
//! The comments are read by `review-annotate` straight from the file — the same
//! code the `review-comments` CLI runs — rather than through ufbx's property
//! capture, so what the viewer shows and what a headless tool reports cannot
//! drift apart. A binary file is streamed, so the read costs a small fraction of
//! the import it follows.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use glam::Vec3;
use review_annotate::comments::{file_note_host, read_file};
use review_annotate::fbx::Format;
use review_annotate::mapping::{ImportedNode, map_nodes};
use review_annotate::thread::{Anchor, SavedView, Topology};
use review_model::extras::Synthetic;
use review_model::{ModelData, SourceExtras, anim, closest_point_on_triangle};
use review_render::OrbitCamera;
use review_ui::{
    CommentEntry, CommentPin, DraftAnchor, NotWritable, ObjectRef, ViewProjectionMode,
    ViewportTool, WorkspaceMode,
};

use crate::App;
use crate::pick::MeshHit;

/// What a file looked like on disk when its comments were read: what a save
/// checks before it writes over the file, so an artist's re-export in the
/// meantime is noticed rather than silently replaced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

impl Fingerprint {
    pub(crate) fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Where the loaded comments came from, and what a save has to carry through
/// untouched.
#[derive(Debug, Clone)]
pub(crate) struct CommentSource {
    /// The file the comments are saved back into — the opened one, or the file a
    /// Save As last wrote.
    pub(crate) path: PathBuf,
    /// What `path` looked like when it was read or last saved.
    pub(crate) fingerprint: Fingerprint,
    pub(crate) format: Format,
    /// Per object, the threads that did not decode, written back as they were.
    pub(crate) opaque: HashMap<i64, Vec<serde_json::Value>>,
    /// Objects whose payload came from a newer format version: never rewritten.
    pub(crate) frozen: HashSet<i64>,
    /// Objects that carried comments in the file — a save removes the property
    /// from any of these that no longer has a thread.
    pub(crate) had_comments: HashSet<i64>,
}

/// A file's comments, read on the import worker.
#[derive(Debug)]
pub(crate) struct LoadedComments {
    pub(crate) entries: Vec<CommentEntry>,
    pub(crate) source: CommentSource,
    /// Per scene node, the FBX object it came from.
    pub(crate) node_objects: Vec<Option<ObjectRef>>,
    /// The object file-level notes go on, and its node.
    pub(crate) file_host: Option<(ObjectRef, Option<usize>)>,
    /// Objects whose comments could not all be read (kept as written, and
    /// reported once).
    pub(crate) unreadable: usize,
}

/// The comments of the import that produced generation `generation`, posted from
/// the import worker.
#[derive(Debug)]
pub(crate) struct CommentsReady {
    pub(crate) generation: u64,
    pub(crate) result: Result<LoadedComments, String>,
}

/// Read `path`'s comments and attach each to the scene node of `model` it is
/// stored on. `extras` says which of the importer's nodes it made up (the scene
/// root, helper nodes), which the mapping steps over; without it no node can be
/// told apart, and threads may end up attached to nothing (they are still
/// listed).
pub(crate) fn read_comments(
    path: &Path,
    model: &ModelData,
    extras: Option<&SourceExtras>,
) -> Result<LoadedComments, String> {
    let fingerprint = Fingerprint::of(path).map_err(|error| error.to_string())?;
    let (scan, comments) = read_file(path).map_err(|error| error.to_string())?;

    let nodes: Vec<ImportedNode<'_>> = model
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| ImportedNode {
            name: &node.name,
            parent: node.parent,
            real: extras
                .and_then(|extras| extras.nodes.get(index))
                .is_none_or(|extra| extra.synthetic == Synthetic::None),
        })
        .collect();
    let mapped = map_nodes(&scan.models, &nodes);
    let node_of_model: HashMap<usize, usize> = mapped
        .iter()
        .enumerate()
        .filter_map(|(node, model)| model.map(|model| (model, node)))
        .collect();

    let node_objects: Vec<Option<ObjectRef>> = mapped
        .iter()
        .map(|model| {
            model.map(|model| ObjectRef {
                id: scan.models[model].id,
                name: scan.models[model].name.clone(),
            })
        })
        .collect();
    let file_host = file_note_host(&scan).map(|model| {
        (
            ObjectRef {
                id: scan.models[model].id,
                name: scan.models[model].name.clone(),
            },
            node_of_model.get(&model).copied(),
        )
    });
    let mut opaque: HashMap<i64, Vec<serde_json::Value>> = HashMap::new();
    let mut frozen = HashSet::new();
    let mut had_comments = HashSet::new();
    for host in &comments.hosts {
        let id = scan.models[host.model].id;
        had_comments.insert(id);
        if host.payload.is_newer() {
            frozen.insert(id);
        }
        if !host.payload.opaque.is_empty() {
            opaque.insert(id, host.payload.opaque.clone());
        }
    }
    // An object whose property could not be read at all is left as it is.
    for warning in &comments.warnings {
        if matches!(warning.error, review_annotate::codec::CodecError::Json(_)) {
            let id = scan.models[warning.model].id;
            had_comments.insert(id);
            frozen.insert(id);
        }
    }

    let mut entries = Vec::with_capacity(comments.thread_count());
    for host in &comments.hosts {
        let object = &scan.models[host.model];
        for thread in &host.payload.threads {
            let mut thread = thread.clone();
            if thread.id.is_empty() {
                thread.id = review_annotate::thread::new_thread_id();
            }
            entries.push(CommentEntry {
                thread,
                node: node_of_model.get(&host.model).copied(),
                object: object.id,
                object_name: object.name.clone(),
                read_only: host.payload.is_newer(),
            });
        }
    }
    let mut unreadable: Vec<usize> = comments
        .warnings
        .iter()
        .map(|warning| warning.model)
        .collect();
    unreadable.dedup();
    Ok(LoadedComments {
        entries,
        source: CommentSource {
            path: path.to_path_buf(),
            fingerprint,
            format: scan.format,
            opaque,
            frozen,
            had_comments,
        },
        node_objects,
        file_host,
        unreadable: unreadable.len(),
    })
}

/// Per node that carries a surface pin, its polygons in the file's order, each as
/// (first triangle, triangle count) — how a pin's `face` / `tri` find their
/// triangle. Built once per model, for the nodes that need it.
#[derive(Default)]
pub(crate) struct PinGeometry {
    revision: Option<u64>,
    faces: HashMap<usize, Vec<(u32, u32)>>,
}

impl PinGeometry {
    fn faces_of(&mut self, model: &ModelData, revision: u64, node: usize) -> &[(u32, u32)] {
        if self.revision != Some(revision) {
            self.faces.clear();
            self.revision = Some(revision);
        }
        self.faces
            .entry(node)
            .or_insert_with(|| node_faces(model, node))
    }
}

/// `node`'s polygons, in order, as (first triangle, triangle count). Import lays
/// each polygon's fan out as consecutive triangles, so a polygon is a run of
/// triangles sharing one `to_face`.
fn node_faces(model: &ModelData, node: usize) -> Vec<(u32, u32)> {
    let triangles = &model.triangles;
    if triangles.node.len() != triangles.to_face.len() {
        return Vec::new();
    }
    let mut faces: Vec<(u32, u32)> = Vec::new();
    let mut last_face = None;
    for (triangle, (&owner, &face)) in triangles.node.iter().zip(&triangles.to_face).enumerate() {
        if owner as usize != node {
            continue;
        }
        match faces.last_mut() {
            Some(run) if last_face == Some(face) => run.1 += 1,
            _ => faces.push((triangle as u32, 1)),
        }
        last_face = Some(face);
    }
    faces
}

/// Where `entry`'s pin is, and whether that point is on the surface: a world
/// anchor as stored; a surface anchor on its triangle, through `pose` when one is
/// active, or — when the host mesh no longer has the polygon it was placed on —
/// at its stored local point under the host's rest transform. `None` for a thread
/// without a 3D anchor.
pub(crate) fn resolve_anchor(
    model: &ModelData,
    revision: u64,
    geometry: &mut PinGeometry,
    pose: Option<(&anim::AnimContext, &anim::DeformPose)>,
    entry: &CommentEntry,
) -> Option<(Vec3, bool)> {
    let anchor = entry.thread.anchor.as_ref()?.known()?;
    match anchor {
        Anchor::World { pos } => Some((Vec3::from(*pos), false)),
        Anchor::Surface {
            face,
            tri,
            bary,
            local,
            topo,
        } => {
            let node = entry.node?;
            let scene_node = model.nodes.get(node)?;
            let faces = geometry.faces_of(model, revision, node);
            let matches = faces.len() as u64 == topo.polys
                && scene_node.source_vertex_count as u64 == topo.verts;
            let on_surface = matches
                .then(|| faces.get(*face as usize))
                .flatten()
                .filter(|&&(_, count)| *tri < count)
                .and_then(|&(first, _)| {
                    let triangle = (first + tri) as usize;
                    let corners = model.indices.get(triangle * 3..triangle * 3 + 3)?;
                    let mut point = Vec3::ZERO;
                    for (&corner, &weight) in corners.iter().zip(bary) {
                        let position = match pose {
                            Some((context, deform)) => {
                                anim::deform_corner(model, context, deform, corner as usize).0
                            }
                            None => model.vertices.get(corner as usize)?.position,
                        };
                        point += position * weight;
                    }
                    Some(point)
                });
            Some(match on_surface {
                Some(point) => (point, true),
                None => (
                    scene_node.transform.transform_point3(Vec3::from(*local)),
                    false,
                ),
            })
        }
        // UV pins belong to the UV workspace.
        Anchor::Uv { .. } => None,
    }
}

impl App {
    /// Put the import worker's comments in front of the chrome.
    pub(crate) fn handle_comments_ready(&mut self, message: CommentsReady) {
        if message.generation != self.model_load_generation() {
            return;
        }
        match message.result {
            Ok(loaded) => {
                if loaded.unreadable > 0 {
                    self.notifications.warning(
                        crate::keys::app_notifications::comments_unreadable(
                            loaded.unreadable as f64,
                        ),
                    );
                }
                log::info!(
                    "{} review comment thread(s) in {} ({:?})",
                    loaded.entries.len(),
                    loaded.source.path.display(),
                    loaded.source.format
                );
                self.ui.comments.load(loaded.entries);
                self.ui.comments.node_objects = loaded.node_objects;
                self.ui.comments.file_host = loaded.file_host;
                self.ui.comments.not_writable = None;
                self.comment_source = Some(loaded.source);
            }
            Err(error) => {
                // A file ufbx opened but the comment reader cannot (an FBX 6 file,
                // say) has no comments to show, and none can be written to it.
                log::warn!("review comments could not be read: {error}");
                self.ui.comments.clear();
                self.ui.comments.not_writable = Some(NotWritable::Unreadable);
                self.comment_source = None;
            }
        }
        self.request_redraw();
    }

    /// Resolve every anchored thread to a world position for this frame's pins:
    /// a surface pin through the current pose, so it rides on a skinned or
    /// animated mesh, falling back to its stored local position when the mesh no
    /// longer has the polygon it was placed on.
    pub(crate) fn update_comment_pins(&mut self) {
        self.ui.comments.pins.clear();
        if self.ui.mode != WorkspaceMode::ThreeD
            || !self.ui.comments.show_pins
            || self.ui.comments.threads.is_empty()
        {
            return;
        }
        let model = self.scene_model.clone();
        let revision = self.scene_revision;
        let fps = model.frame_rate_or_default();
        // The frame on screen, to tell which pins are in their frame range.
        let current = self.ui.animation.selected_clip.and_then(|index| {
            let clip = model.animations.get(index)?;
            Some((
                clip.name.as_str(),
                clip.frame_at(self.ui.animation.time, fps),
            ))
        });
        let pose = self
            .animation
            .active
            .then(|| {
                self.animation
                    .context()
                    .map(|context| (context, &self.animation.deform))
            })
            .flatten();

        let mut pins = Vec::new();
        for (index, entry) in self.ui.comments.threads.iter().enumerate() {
            let resolved = resolve_anchor(&model, revision, &mut self.pin_geometry, pose, entry);
            let Some((world, on_surface)) = resolved else {
                continue;
            };
            let in_range = entry.thread.frames.as_ref().is_none_or(|frames| {
                current.is_some_and(|(clip, frame)| {
                    clip == frames.clip
                        && (frames.start as usize..=frames.end as usize).contains(&frame)
                })
            });
            pins.push(CommentPin {
                thread: index,
                world,
                on_surface,
                in_range,
            });
        }
        self.ui.comments.pins = pins;
    }

    /// Whether a viewport click leaves a review comment.
    pub(crate) fn commenting_enabled(&self) -> bool {
        self.ui.tool == ViewportTool::Comment && self.ui.mode == WorkspaceMode::ThreeD
    }

    /// The camera now, as a comment saves it.
    fn saved_view(&self) -> Option<SavedView> {
        let camera = self.renderer.as_ref()?.camera;
        Some(SavedView {
            target: camera.target.to_array(),
            yaw: camera.yaw,
            pitch: camera.pitch,
            distance: camera.distance,
            fov: camera.fov_y_radians,
            ortho: self.ui.projection_mode == ViewProjectionMode::Orthographic,
        })
    }

    /// A Comment-tool click at `position` (physical pixels): on the mesh, start a
    /// comment pinned to the spot — or, while a pin is being moved, move it there;
    /// on empty space, start a comment about the view.
    pub(crate) fn place_comment(&mut self, position: glam::Vec2) {
        let hit = self.cast_mesh(position);
        let model = self.scene_model.clone();
        let placed = hit.map(|hit| (hit, self.surface_anchor(&model, hit)));
        if let Some(index) = self.ui.comments.repin {
            if let Some((hit, (anchor, world))) = placed {
                self.ui
                    .comments
                    .repin_thread(index, hit.node, anchor, world);
            }
            self.request_redraw();
            return;
        }
        let Some(view) = self.saved_view() else {
            return;
        };
        let frames = self.ui.current_frames(&model);
        let scale = self.scale_factor();
        let screen = egui::pos2(position.x / scale, position.y / scale);
        let anchor = match placed {
            Some((hit, (anchor, world))) => DraftAnchor::Surface {
                node: hit.node,
                anchor,
                world,
            },
            None => DraftAnchor::View,
        };
        self.ui.comments.begin_draft(anchor, frames, view, screen);
        self.request_redraw();
    }

    /// The surface anchor for a click on the mesh: the polygon of the clicked
    /// node and triangle of its fan that was hit, the barycentrics of the point on
    /// that triangle as drawn (posed, if it is), the point in the node's local
    /// space, and the node's counts. A mesh with no polygon topology gets a scene
    /// point instead.
    fn surface_anchor(&mut self, model: &ModelData, hit: MeshHit) -> (Anchor, Vec3) {
        let world_point = (
            Anchor::World {
                pos: hit.world.to_array(),
            },
            hit.world,
        );
        let Some(node) = model.nodes.get(hit.node) else {
            return world_point;
        };
        let faces = self
            .pin_geometry
            .faces_of(model, self.scene_revision, hit.node);
        let polys = faces.len() as u64;
        let Some((face, first)) = faces
            .iter()
            .enumerate()
            .find_map(|(face, &(first, count))| {
                (first as usize..(first + count) as usize)
                    .contains(&hit.triangle)
                    .then_some((face, first))
            })
        else {
            return world_point;
        };
        let pose = (hit.posed && self.animation.active)
            .then(|| {
                self.animation
                    .context()
                    .map(|context| (context, &self.animation.deform))
            })
            .flatten();
        let Some(corners) = model.indices.get(hit.triangle * 3..hit.triangle * 3 + 3) else {
            return world_point;
        };
        let corner = |index: u32| match pose {
            Some((context, deform)) => {
                anim::deform_corner(model, context, deform, index as usize).0
            }
            None => model
                .vertices
                .get(index as usize)
                .map_or(Vec3::ZERO, |vertex| vertex.position),
        };
        let (_, bary) = closest_point_on_triangle(
            hit.world,
            corner(corners[0]),
            corner(corners[1]),
            corner(corners[2]),
        );
        (
            Anchor::Surface {
                face: face as u32,
                tri: (hit.triangle - first as usize) as u32,
                bary: bary.to_array(),
                local: node
                    .transform
                    .inverse()
                    .transform_point3(hit.world)
                    .to_array(),
                topo: Topology {
                    polys,
                    verts: node.source_vertex_count as u64,
                },
            },
            hit.world,
        )
    }

    /// Go to thread `index` the way it was written: fly to its saved view (or,
    /// without one, turn the camera to its pin), and jump to its clip and first
    /// frame.
    pub(crate) fn show_comment(&mut self, index: usize) {
        let Some(entry) = self.ui.comments.threads.get(index) else {
            return;
        };
        let thread = entry.thread.clone();
        let pin = self
            .ui
            .comments
            .pins
            .iter()
            .find(|pin| pin.thread == index)
            .map(|pin| pin.world);
        if let Some(renderer) = self.renderer.as_mut() {
            let current = renderer.camera;
            if let Some(view) = thread.view {
                renderer.animate_camera_to(OrbitCamera {
                    target: Vec3::from(view.target),
                    yaw: view.yaw,
                    pitch: view.pitch,
                    distance: view.distance,
                    fov_y_radians: view.fov,
                    ..current
                });
                self.ui.projection_mode = if view.ortho {
                    ViewProjectionMode::Orthographic
                } else {
                    ViewProjectionMode::Perspective
                };
            } else if let Some(target) = pin {
                renderer.animate_camera_to(OrbitCamera { target, ..current });
            }
        }
        if let Some(frames) = &thread.frames {
            let model = self.scene_model.clone();
            if let Some(clip_index) = model
                .animations
                .iter()
                .position(|clip| clip.name == frames.clip)
            {
                self.ui.select_clip(&model, Some(clip_index));
                let fps = model.frame_rate_or_default();
                self.ui.animation.time =
                    model.animations[clip_index].frame_time(frames.start as usize, fps);
            }
        }
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_annotate::codec::{PROPERTY, encode};
    use review_annotate::fbx::{self, Edit};
    use review_annotate::thread::{AnchorValue, Message, Scope, Status, Thread, Topology};

    fn fixture(name: &str) -> Option<PathBuf> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/test_models")
            .join(name);
        path.exists().then_some(path)
    }

    fn surface_thread(face: u32, topo: Topology, local: [f32; 3]) -> Thread {
        Thread {
            id: String::new(),
            status: Status::Open,
            scope: Scope::Object,
            anchor: Some(AnchorValue::Known(Anchor::Surface {
                face,
                tri: 0,
                bary: [1.0 / 3.0; 3],
                local,
                topo,
            })),
            frames: None,
            view: None,
            messages: vec![Message {
                author: "test".to_owned(),
                time: String::new(),
                text: "pin".to_owned(),
                extra: Default::default(),
            }],
            extra: Default::default(),
        }
    }

    /// A surface pin written into a real file comes back attached to the node it
    /// was written on, lands on its triangle, and falls back to its local point
    /// once the mesh's counts no longer match.
    #[test]
    fn a_surface_pin_round_trips_onto_its_triangle() {
        let Some(path) = fixture("SM_Ammo_Crate_01a.fbx") else {
            return;
        };
        let (model, extras) = review_import::load_model_full(&path).expect("imports");
        let extras = extras.expect("extras");
        let bytes = std::fs::read(&path).expect("reads");
        let scan = fbx::scan(&bytes, PROPERTY).expect("scans");
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
        // The first mesh node, and the object it came from.
        let (node, object) = model
            .nodes
            .iter()
            .enumerate()
            .find_map(|(index, scene_node)| {
                scene_node
                    .mesh_part
                    .and(mapped[index])
                    .map(|object| (index, object))
            })
            .expect("a mesh node");
        let faces = node_faces(&model, node);
        let topo = Topology {
            polys: faces.len() as u64,
            verts: model.nodes[node].source_vertex_count as u64,
        };
        let (first, _) = faces[0];
        let corners = &model.indices[first as usize * 3..first as usize * 3 + 3];
        let expected = corners
            .iter()
            .map(|&corner| model.vertices[corner as usize].position)
            .sum::<Vec3>()
            / 3.0;

        let threads = [
            surface_thread(0, topo, [0.0; 3]),
            surface_thread(
                0,
                Topology {
                    polys: topo.polys + 1,
                    verts: topo.verts,
                },
                [1.0, 2.0, 3.0],
            ),
        ];
        let patched = fbx::patch(
            &bytes,
            PROPERTY,
            &[Edit {
                model: scan.models[object].id,
                value: Some(encode(&threads, &[])),
                hidden: false,
            }],
        )
        .expect("patches");
        let temp = std::env::temp_dir().join(format!("review-pin-{}.fbx", std::process::id()));
        std::fs::write(&temp, patched).expect("writes");
        let loaded = read_comments(&temp, &model, Some(&extras)).expect("reads comments");
        let _ = std::fs::remove_file(&temp);

        assert_eq!(loaded.entries.len(), 2);
        assert!(loaded.entries.iter().all(|entry| entry.node == Some(node)));
        assert!(
            loaded
                .entries
                .iter()
                .all(|entry| entry.thread.id.len() == 16),
            "ids assigned"
        );

        let mut geometry = PinGeometry::default();
        let (point, on_surface) =
            resolve_anchor(&model, 1, &mut geometry, None, &loaded.entries[0]).expect("resolves");
        assert!(on_surface);
        assert!(point.distance(expected) < 1e-4, "{point} vs {expected}");

        let (point, on_surface) =
            resolve_anchor(&model, 1, &mut geometry, None, &loaded.entries[1]).expect("resolves");
        assert!(!on_surface, "an edited mesh falls back to the local point");
        let fallback = model.nodes[node]
            .transform
            .transform_point3(Vec3::new(1.0, 2.0, 3.0));
        assert!(point.distance(fallback) < 1e-4);
    }
}
