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
                // stand-in had no bone for included. Copied, not taken: until the
                // re-parse lands the stand-in is still the session on screen, and
                // if it fails it stays so, with its attached files still indexed
                // by these paths.
                let mut paths: Vec<PathBuf> = take(rig).map(|(path, _)| path).into_iter().collect();
                paths.extend(self.scene_files.iter().cloned());
                paths.extend(
                    (0..shapes.len()).filter_map(|index| take(index).map(|(path, _)| path)),
                );
                log::info!(
                    "{} takes over from the skeleton standing in",
                    paths[0].display()
                );
                // Supersedes parses only: the stand-in goes on being measured,
                // since it is what stays on screen should the re-parse fail.
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
                let repairs = extras.warnings().to_vec();
                let model = Arc::new(model);
                // Replacing the model is what supersedes the old one's measuring,
                // here rather than when the request was made.
                let generation =
                    self.apply_new_model(&rig_path, Arc::clone(&model), started.elapsed());
                self.report_repairs(&rig_path, &repairs);
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
                    generation: self.measure_generation(),
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
    /// dependent state (framing, stats, materials, undo) at it. Returns the
    /// generation it is measured under.
    fn apply_new_model(&mut self, path: &Path, model: Arc<ModelData>, elapsed: Duration) -> u64 {
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
        let generation = self.replace_scene_model(model, vec![path.to_path_buf()]);
        let label = file_label(path);
        self.set_window_title(Some(&label));
        self.notifications.success(keys::app_notifications::loaded(
            label,
            format_load_time(elapsed),
        ));
        // A gate run starts measuring from here — the first present with the
        // model actually on screen (`gate.rs`); no-op otherwise.
        self.gate_model_ready(path);
        generation
    }

    /// Attach each of `files` to `model`, the session's model, and say so.
    /// Returns them by their file index, for their clips to be measured.
    fn attach_parsed(
        &mut self,
        model: &ModelData,
        files: impl Iterator<Item = (PathBuf, StagedImport)>,
    ) -> Vec<(usize, Arc<AttachedAnimation>)> {
        let mut attached = Vec::new();
        // What the notice names when one file was attached. Kept from the loop
        // rather than looked up again by file index, which assumes `scene_files`
        // lines up with the attached list one for one.
        let mut last_label = String::new();
        for (path, staged) in files {
            // The file's own source capture is never used: only the model's is
            // exported. Dropping it here frees the C side - after saying what the
            // import had to leave out of the file's clips.
            let StagedImport {
                model: source,
                extras,
            } = staged;
            self.report_repairs(&path, extras.warnings());
            drop(extras);
            let animation = Arc::new(AttachedAnimation::new(model, source, file_stem(&path)));
            let matched = animation.node_map.iter().flatten().count();
            let report = &animation.report;
            log::info!(
                "attached {}: {} clips, {matched} of its {} nodes matched; animates {} of {} \
                 bones and {} of {} blend-shape channels; {} unmatched bones, {} other \
                 unmatched nodes",
                path.display(),
                animation.clips.len(),
                animation.nodes.len(),
                report.bones.animated,
                report.bones.total,
                report.channels.animated,
                report.channels.total,
                report.unmatched_bones.len(),
                report.unmatched_nodes.len(),
            );
            self.warn_mismatch(model, &path, &animation);
            self.ui.attach_animation(Arc::clone(&animation));
            self.scene_files.push(path.clone());
            self.remember_recent_file(&path);
            last_label = file_label(&path);
            attached.push((self.ui.attached_animations.len(), animation));
        }
        let clips: usize = attached
            .iter()
            .map(|(_, animation)| animation.clips.len())
            .sum();
        match attached.as_slice() {
            [] => {}
            [_] => {
                self.notifications
                    .success(keys::app_notifications::attached(clips as f64, last_label));
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

    /// Say up front what does not line up between an attached file and the
    /// model — before anything is played, so a clip that looks wrong is never
    /// the first sign of it. One card per file; nothing when it all lines up.
    fn warn_mismatch(&mut self, model: &ModelData, path: &Path, animation: &AttachedAnimation) {
        let lines = review_ui::binding_warnings(model, animation);
        if lines.is_empty() {
            return;
        }
        for line in &lines {
            log::warn!("{}: {line}", path.display());
        }
        let model_label = self
            .scene_files
            .first()
            .map(|path| file_label(path))
            .unwrap_or_default();
        self.notifications.report(
            NoticeKind::Warning,
            keys::app_notifications::attached_mismatch(file_label(path), model_label),
            lines,
        );
    }

    /// One card saying what the import had to leave out of `path` to load it at
    /// all: a malformed animation key, blend shape or skin binding is dropped on
    /// its own rather than failing the file (`review_import`'s funnel repair). The
    /// lines are the import's own diagnostics, passed through as a message variable
    /// like any other error detail.
    fn report_repairs(&mut self, path: &Path, repairs: &[String]) {
        if repairs.is_empty() {
            return;
        }
        for line in repairs {
            log::warn!("{}: {line}", path.display());
        }
        let lines = repairs
            .iter()
            .map(|line| keys::app_notifications::import_repair_line(line.clone()))
            .collect();
        self.notifications.report(
            NoticeKind::Warning,
            keys::app_notifications::loaded_with_repairs(file_label(path)),
            lines,
        );
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
