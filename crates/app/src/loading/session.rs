//! Putting a request's parsed files on screen: [`classify`]'s plan, applied.
//!
//! A new session replaces the model and attaches the rest of the batch to it; an
//! addition attaches to the model already there. Attaching moves a file's clips
//! into an [`AttachedAnimation`] re-pointed at the model and drops everything
//! else the file carried — its meshes, its source capture — so an attached file
//! costs only the keys that can actually play.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use review_import::StagedImport;
use review_model::{AttachedAnimation, Binding, ModelData};
use review_ui::{ActivityId, NoticeKind};

use super::classify::{FileShape, LeftOut, Matcher, Plan, Rig, Session, classify};
use super::measure::MeasureJob;
use super::{LoadRequest, OpenIntent, file_label, format_load_time, frame_camera_to_model};
use crate::{App, keys};

/// [`classify`]'s binding questions, answered over the parsed files and the
/// model on screen.
struct ModelMatcher<'a> {
    current: &'a ModelData,
    files: &'a [&'a ModelData],
}

impl Matcher for ModelMatcher<'_> {
    fn attaches(&self, file: usize, rig: Rig) -> bool {
        let rig = match rig {
            Rig::Current => self.current,
            Rig::File(index) => self.files[index],
        };
        attaches(rig, self.files[file])
    }

    fn current_attaches_to(&self, rig: usize) -> bool {
        attaches(self.files[rig], self.current)
    }
}

/// Whether `source` is animation for `rig`: its clips drive something of it,
/// and any mesh it carries is a copy of one of the rig's own.
fn attaches(rig: &ModelData, source: &ModelData) -> bool {
    let binding = Binding::new(rig, source);
    binding.binds(source) && binding.foreign_meshes(rig, source) == 0
}

/// What an attached file is called on screen: its name without the extension,
/// as the model's own file is.
fn file_stem(path: &Path) -> String {
    path.file_stem().map_or_else(
        || file_label(path),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

impl App {
    /// Classify `parsed` against what is on screen and apply the plan. Returns
    /// what is left to measure, if anything — the caller spawns it, and it is
    /// what brings the request's loading card down.
    pub(super) fn apply_parsed(
        &mut self,
        intent: OpenIntent,
        parsed: Vec<(PathBuf, StagedImport)>,
        started: Instant,
        activity: ActivityId,
    ) -> Option<MeasureJob> {
        if parsed.is_empty() {
            return None;
        }
        let shapes: Vec<FileShape> = parsed
            .iter()
            .map(|(_, staged)| FileShape {
                has_geometry: staged.model.has_geometry(),
                triangles: staged.model.stats.triangle_count,
                has_clips: !staged.model.animations.is_empty(),
            })
            .collect();
        let session = if self.scene_model.is_empty() {
            Session::Empty
        } else if self.scene_model.has_geometry() {
            Session::Rig
        } else {
            Session::StandIn
        };
        let plan = {
            let models: Vec<&ModelData> = parsed.iter().map(|(_, staged)| &staged.model).collect();
            let matcher = ModelMatcher {
                current: &self.scene_model,
                files: &models,
            };
            classify(intent, session, &shapes, &matcher)
        };
        log::debug!("opened {} files as {plan:?}", parsed.len());

        let mut files: Vec<Option<(PathBuf, StagedImport)>> =
            parsed.into_iter().map(Some).collect();
        let mut take = |index: usize| files.get_mut(index).and_then(Option::take);
        match plan {
            Plan::TakeOver { rig } => {
                // Every file of the session the rig takes over is opened again with
                // it, so each binds to the rig from its own file — tracks the
                // stand-in had no bone for included.
                let mut paths: Vec<PathBuf> = take(rig).map(|(path, _)| path).into_iter().collect();
                paths.extend(std::mem::take(&mut self.scene_files));
                paths.extend(
                    (0..shapes.len()).filter_map(|index| take(index).map(|(path, _)| path)),
                );
                log::info!(
                    "{} takes over from the skeleton standing in",
                    paths[0].display()
                );
                self.supersede_loads();
                self.start_request_now(LoadRequest {
                    paths,
                    intent: OpenIntent::NewSession,
                });
                None
            }
            Plan::NewSession {
                rig,
                attach,
                left_out,
            } => {
                let (rig_path, StagedImport { model, extras }) = take(rig)?;
                // A drop that turned out to replace the session supersedes what
                // the old one still had in flight; for an open it already has.
                let generation = self.supersede_loads();
                let model = Arc::new(model);
                self.apply_new_model(&rig_path, Arc::clone(&model), started.elapsed());
                let attached = self.attach_parsed(&model, attach.into_iter().filter_map(&mut take));
                // After its animation files, so the model is the newest entry in
                // Open Recent: reopening it from a fresh start opens the model, not
                // one of its animations on its own.
                self.remember_recent_file(&rig_path);
                let left_out = left_out
                    .into_iter()
                    .filter_map(|(index, why)| take(index).map(|(path, _)| (path, why)))
                    .collect();
                self.report_left_out(left_out);
                Some(MeasureJob {
                    generation,
                    activity,
                    model,
                    new_model: Some(extras),
                    attached,
                    started,
                })
            }
            Plan::Attach { attach, left_out } => {
                let model = Arc::clone(&self.scene_model);
                let attached = self.attach_parsed(&model, attach.into_iter().filter_map(&mut take));
                let left_out = left_out
                    .into_iter()
                    .filter_map(|(index, why)| take(index).map(|(path, _)| (path, why)))
                    .collect();
                self.report_left_out(left_out);
                (!attached.is_empty()).then(|| MeasureJob {
                    generation: self.model_load_generation(),
                    activity,
                    model,
                    new_model: None,
                    attached,
                    started,
                })
            }
        }
    }

    /// Show `model` as the session's model, and re-point every piece of
    /// dependent state (framing, stats, materials, undo) at it.
    fn apply_new_model(&mut self, path: &Path, model: Arc<ModelData>, elapsed: Duration) {
        // Loading into the empty viewport (first load, or after Ctrl+N) shows
        // the model already framed — a fly-in from the home view would only
        // delay it. Replacing an already-loaded model keeps the animated
        // re-frame so the view change reads as a transition.
        let animate_framing = !self.scene_model.is_empty();
        let stats = model.stats;
        log::info!(
            "loaded {} in {:.2} s: {} polygons, {} triangles, {} vertices, \
             {} materials, {} draws, {} UV sets, {} nodes, {} bones, {} clips, \
             1 unit = {} m",
            path.display(),
            elapsed.as_secs_f64(),
            stats.polygon_count,
            stats.triangle_count,
            stats.vertex_count,
            stats.material_count,
            stats.draw_count,
            stats.uv_set_count,
            model.nodes.len(),
            stats.bone_count,
            stats.clip_count,
            stats.source_unit_meters,
        );
        let (materials_snapshot, material_revision) = if let Some(renderer) = self.renderer.as_mut()
        {
            frame_camera_to_model(renderer, &model, animate_framing);
            renderer.reset_uv_camera();
            // Seed the editable material table from the import defaults.
            renderer.set_model_materials(&model.materials);
            (renderer.material_snapshot(), renderer.material_revision())
        } else {
            (Vec::new(), 0)
        };
        self.ui.materials_snapshot = materials_snapshot;
        self.ui.material_revision = material_revision;
        self.reset_ui_for_new_model(&model);
        self.scene_model = model;
        self.scene_files = vec![path.to_path_buf()];
        // The capture describing this model arrives on its own event a moment
        // later; the previous model's must not stand in for it.
        self.scene_extras = None;
        self.scene_revision = self.next_model_revision();
        // Any Opt result (and every per-object override) describes the previous
        // model, so drop both before the new one is drawn.
        self.reset_opt_for_new_model();
        self.reset_audit_for_new_model();
        let label = file_label(path);
        self.set_window_title(Some(&label));
        self.notifications.success(keys::app_notifications::loaded(
            label,
            format_load_time(elapsed),
        ));
        // A gate run starts measuring from here — the first present with the
        // model actually on screen (`gate.rs`); no-op otherwise.
        self.gate_model_ready(path);
    }

    /// Attach each of `files` to `model`, the session's model, and say so.
    /// Returns them by their file index, for their clips to be measured.
    fn attach_parsed(
        &mut self,
        model: &ModelData,
        files: impl Iterator<Item = (PathBuf, StagedImport)>,
    ) -> Vec<(usize, Arc<AttachedAnimation>)> {
        let mut attached = Vec::new();
        for (path, staged) in files {
            // The file's own source capture is never used: only the model's is
            // exported. Dropping it here frees the C side.
            let StagedImport { model: source, .. } = staged;
            let animation = Arc::new(AttachedAnimation::new(model, source, file_stem(&path)));
            let matched = animation.node_map.iter().flatten().count();
            log::info!(
                "attached {}: {} clips, {matched} of its {} nodes matched",
                path.display(),
                animation.clips.len(),
                animation.nodes.len()
            );
            self.ui.attach_animation(Arc::clone(&animation));
            self.scene_files.push(path.clone());
            self.remember_recent_file(&path);
            attached.push((self.ui.attached_animations.len(), animation));
        }
        let clips: usize = attached
            .iter()
            .map(|(_, animation)| animation.clips.len())
            .sum();
        match attached.as_slice() {
            [] => {}
            [(file, _)] => {
                let path = &self.scene_files[*file];
                self.notifications
                    .success(keys::app_notifications::attached(
                        clips as f64,
                        file_label(path),
                    ));
            }
            several => self
                .notifications
                .success(keys::app_notifications::attached_files(
                    clips as f64,
                    several.len() as f64,
                )),
        }
        attached
    }

    /// One card listing every file of a request that was not loaded, and why.
    fn report_left_out(&mut self, left_out: Vec<(PathBuf, LeftOut)>) {
        if left_out.is_empty() {
            return;
        }
        let model = self
            .scene_files
            .first()
            .map(|path| file_label(path))
            .unwrap_or_default();
        let lines = left_out
            .iter()
            .map(|(path, why)| {
                log::warn!("not loaded: {} ({why:?})", path.display());
                match why {
                    LeftOut::SecondModel => {
                        keys::app_notifications::not_loaded_second_model(file_label(path))
                    }
                    LeftOut::NoMatch => keys::app_notifications::not_loaded_no_match(
                        file_label(path),
                        model.clone(),
                    ),
                }
            })
            .collect();
        self.notifications.report(
            NoticeKind::Warning,
            keys::app_notifications::not_loaded(left_out.len() as f64),
            lines,
        );
    }
}
