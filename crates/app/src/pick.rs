//! The viewport pick: turning a pointer position into a selection.
//!
//! Everything here runs on the main thread between events, so it is written to
//! do as little as possible per pointer move: one ray, one BVH query, and a
//! redraw requested only when the answer actually changes.
//!
//! The pieces it is assembled from live where they belong (invariant 2 — this is
//! the coordinator, not a fourth copy of the maths): the screen-to-world ray is
//! `OrbitCamera::screen_ray`, the spatial query is `review_model`'s
//! `SceneBvh`/`PosedScene`, the bone shapes come from the same builder the
//! skeleton overlay draws with, and what a click *does* to the selection is
//! `review_ui`'s shared rule, the one the Outliner's rows obey too.

use glam::Vec2;
use review_model::{ModelData, PosedScene, SceneBvh};
use review_render::{CameraProjection, OrbitCamera};
use review_ui::{HoverTarget, ViewportTool, WorkspaceMode};

use crate::App;

/// What a pick resolved to, before it is applied to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickTarget {
    /// A mesh or group node.
    Node(usize),
    /// A bone, while the skeleton overlay has made bones the pick target.
    Bone(usize),
}

impl PickTarget {
    /// The same target as a hover, which is the same thing seen a moment before
    /// the click.
    fn as_hover(self) -> HoverTarget {
        match self {
            PickTarget::Node(node) => HoverTarget::Node(node),
            PickTarget::Bone(bone) => HoverTarget::Bone(bone),
        }
    }
}

/// One viewport a ray can be cast into: the camera that draws it, the mesh it
/// draws, and the rectangle of the backbuffer it occupies.
///
/// A rectangle rather than the whole window because the Opt split is two of
/// these side by side, each with its own camera, its own mesh and its own
/// aspect ratio — which is the one thing that cannot be taken from the app's
/// camera, since `app` sets that from the whole window and the renderer
/// overrides it per half.
struct PickView<'a> {
    camera: OrbitCamera,
    model: &'a ModelData,
    bvh: &'a SceneBvh,
    /// Top-left corner and size of this view in physical pixels.
    origin: Vec2,
    size: Vec2,
    /// Whether this view draws the model posed. The Opt workspace deliberately
    /// shows the bind pose, so a pick there must use the bind pose too.
    posed: bool,
}

/// Whether a press and release that far apart still counts as a click rather
/// than a drag.
///
/// The whole of what lets one button both turn the camera and select: past the
/// threshold the gesture was a drag and belongs to the camera, and nothing is
/// selected when the button comes up.
pub(crate) fn is_click(press: Vec2, release: Vec2) -> bool {
    press.distance(release) <= crate::input::CLICK_MAX_DISTANCE_PX
}

impl App {
    /// Whether clicking the viewport picks anything right now.
    ///
    /// The Select tool has to be on, and there has to be a scene to pick in —
    /// the UV and Texture workspaces are 2D image viewers with nothing to hit.
    pub(crate) fn picking_enabled(&self) -> bool {
        self.ui.tool == ViewportTool::Select && self.ui.mode.is_scene()
    }

    /// Whether a pick targets *bones* rather than mesh parts.
    ///
    /// Having the skeleton drawn is itself the statement that the rig is what is
    /// being looked at, and the mesh is in front of it — so the overlay being up
    /// is what switches the target, with no second control to find.
    fn picking_bones(&self) -> bool {
        self.ui.debug.show_skeleton
    }

    /// The view a pointer position falls in, or `None` when there is nothing to
    /// pick: no index built yet, no mesh, or a workspace without a scene.
    fn pick_view(&self, position: Vec2) -> Option<PickView<'_>> {
        if !self.ui.mode.is_scene() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let (width, height) = self.gpu.as_ref()?.size();
        if width == 0 || height == 0 {
            return None;
        }
        let full = Vec2::new(width as f32, height as f32);

        // The Opt split is the only layout with more than one view. Which half
        // the pointer is in is decided by the *rectangle*, not by
        // `in_opt_right_view` — that answers "which camera does a drag move",
        // which is deliberately false while the cameras are synced, and a pick
        // still has to target the processed mesh on that side.
        if let Some([left, right]) = self.opt_split_halves() {
            let right_half = position.x >= right.0.x;
            let (origin, size) = if right_half { right } else { left };
            // Each half is drawn through a camera whose aspect ratio is its own,
            // not the window's; projecting through the uncorrected one puts every
            // ray a little sideways of the pixel it came from.
            let mut camera = if right_half && !self.ui.opt.camera_sync {
                renderer.opt_camera
            } else {
                renderer.camera
            };
            camera.aspect_ratio = size.x / size.y.max(1.0);
            let processed = right_half.then(|| self.opt_level_pick()).flatten();
            let (model, bvh) = match processed {
                Some(level) => level,
                // Before a run lands the renderer draws the source into both
                // halves, so a pick on the right must target it too.
                None => (self.scene_model.as_ref(), self.scene_bvh.as_deref()?),
            };
            return Some(PickView {
                camera,
                model,
                bvh,
                origin,
                size,
                posed: false,
            });
        }

        // The Aud split draws the model in its left half only; the right half is a
        // UV layout, with nothing to pick.
        if let Some([left, right]) = self.aud_split_halves() {
            if position.x >= right.0.x {
                return None;
            }
            let (origin, size) = left;
            let mut camera = renderer.camera;
            camera.aspect_ratio = size.x / size.y.max(1.0);
            return Some(PickView {
                camera,
                model: self.scene_model.as_ref(),
                bvh: self.scene_bvh.as_deref()?,
                origin,
                size,
                posed: self.animation.active,
            });
        }

        // Every other case draws one view over the whole backbuffer, with the
        // chrome painted on top of it.
        let (model, bvh) = match self.ui.mode {
            // The Opt overlay draws both meshes in one view; the solid one is
            // what a click is aiming at.
            WorkspaceMode::Opt => match self.opt_overlay_pick() {
                Some(level) => level,
                None => (self.scene_model.as_ref(), self.scene_bvh.as_deref()?),
            },
            _ => (self.scene_model.as_ref(), self.scene_bvh.as_deref()?),
        };
        Some(PickView {
            camera: renderer.camera,
            model,
            bvh,
            origin: Vec2::ZERO,
            size: full,
            // Opt always shows the bind pose, by design.
            posed: self.ui.mode.shows_pose() && self.animation.active,
        })
    }

    /// The window's device pixel ratio, for turning a size in egui points into
    /// the physical pixels a pointer position is measured in.
    fn scale_factor(&self) -> f32 {
        self.window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor() as f32)
    }

    /// The processed level currently drawn, with its own index — what the split's
    /// right half and the overlay's solid mesh are.
    fn opt_level_pick(&self) -> Option<(&ModelData, &SceneBvh)> {
        let opt = self.opt.as_ref()?;
        let level = self.ui.opt.active_lod;
        let model = &opt.processed.as_ref()?.lod(level)?.model;
        Some((model, opt.level_bvhs.get(level)?))
    }

    /// The mesh drawn solid in the Opt overlay — the processed level, unless the
    /// A/B swap has put the source in front.
    fn opt_overlay_pick(&self) -> Option<(&ModelData, &SceneBvh)> {
        if self.ui.opt.layout != review_ui::OptLayout::Overlay {
            return None;
        }
        match self.ui.opt.side {
            review_ui::ComparisonSide::Processed => self.opt_level_pick(),
            review_ui::ComparisonSide::Source => None,
        }
    }

    /// Cast a ray through `position` (physical pixels, window-relative) and
    /// return what it hits.
    fn cast(&mut self, position: Vec2) -> Option<PickTarget> {
        self.cast_hit(position).map(|(target, _)| target)
    }

    /// [`Self::cast`], plus the triangle the ray hit when it hit the mesh — what
    /// tells a click on a highlighted offender from one beside it.
    fn cast_hit(&mut self, position: Vec2) -> Option<(PickTarget, Option<u32>)> {
        let bones = self.picking_bones();
        let hidden = self.ui.hidden_mesh_nodes();
        let solo = self.ui.solo.then(|| self.ui.selected_node_set());

        // Rebuilt before the borrow below, since it needs `&mut self`.
        if !bones {
            self.sync_posed_pick();
        }

        let view = self.pick_view(position)?;
        let projection: CameraProjection = self.ui.projection_mode.into();
        let ndc = OrbitCamera::ndc_from_viewport(position - view.origin, view.size);
        let (origin, dir) = view.camera.screen_ray(ndc, projection);
        let (near, far) = view.camera.near_far();
        // The same span the camera draws, so what can be picked is exactly what
        // can be seen. An infinite perspective far plane has no bound to give.
        let t_max = match projection {
            CameraProjection::Perspective => f32::INFINITY,
            CameraProjection::Orthographic => far,
        };

        if bones {
            // The bone test measures screen distances, so it needs the pointer in
            // the same view-relative pixels the projection hands back, and a
            // tolerance scaled out of points into them.
            let pointer = position - view.origin;
            let tolerance = review_render::BONE_PICK_TOLERANCE_POINTS * self.scale_factor();
            let joints = review_render::posed_joint_positions(
                view.model,
                view.posed.then_some(&self.animation.deform),
            );
            let camera = view.camera;
            let size = view.size;
            return review_render::pick_bone_shape(
                view.model,
                &joints,
                self.ui.debug.skeleton_joint_scale,
                origin,
                dir,
                &|world| camera.project(world, projection, size),
                pointer,
                tolerance,
            )
            .map(|bone| (PickTarget::Bone(bone), None));
        }

        // A hidden mesh is not on screen, so it cannot be clicked; and while a
        // selection is isolated, the isolated set is all there is to click.
        let allow = |node: u32| {
            !hidden.contains(&node)
                && solo
                    .as_ref()
                    .is_none_or(|isolated| isolated.is_empty() || isolated.contains(&node))
        };

        let hit = match (view.posed, self.posed_pick.as_ref()) {
            (true, Some((_, posed))) => {
                let ctx = self.animation.context()?;
                posed.pick(
                    view.bvh,
                    view.model,
                    ctx,
                    &self.animation.deform,
                    origin,
                    dir,
                    near,
                    t_max,
                    allow,
                )
            }
            _ => view.bvh.pick(view.model, origin, dir, near, t_max, allow),
        }?;
        (hit.node != u32::MAX).then_some((PickTarget::Node(hit.node as usize), Some(hit.triangle)))
    }

    /// Rebuild the posed pick index when the pose or the model has moved.
    ///
    /// Keyed on the pose revision, so a paused clip is indexed once however long
    /// it is looked at, and a static model never builds one at all.
    fn sync_posed_pick(&mut self) {
        if !self.animation.active || !self.ui.mode.shows_pose() {
            self.posed_pick = None;
            return;
        }
        let key = (self.scene_revision, self.animation.pose_revision);
        if self
            .posed_pick
            .as_ref()
            .is_some_and(|(seen, _)| *seen == key)
        {
            return;
        }
        let Some(bvh) = self.scene_bvh.as_deref() else {
            return;
        };
        let Some(ctx) = self.animation.context() else {
            return;
        };
        let _z = crate::prof::zone!("Build Posed Pick");
        let posed = PosedScene::build(bvh, &self.scene_model, ctx, &self.animation.deform);
        self.posed_pick = Some((key, posed));
    }

    /// Resolve what the pointer is over, if a move has left one pending.
    ///
    /// Called once per event batch rather than per `CursorMoved`, which can fire
    /// many times between frames: the pointer only has one position that matters
    /// and casting for each of the ones it passed through would be waste. A
    /// redraw is requested only when the answer *changes*, so sliding the pointer
    /// across one part costs a ray and nothing else (invariant 6).
    pub(crate) fn resolve_hover(&mut self) {
        // Nothing is hovered when nothing is pickable, and the check comes first
        // so that leaving Select mode, or switching to a workspace without a
        // scene, drops a highlight that would otherwise sit there unexplained
        // until the pointer next moved.
        //
        // Hover is also suppressed while a clip *plays*: the shape under the
        // pointer changes faster than a highlight could usefully report it, and
        // each pose would cost a fresh index.
        if !self.picking_enabled() || self.animation_playing() {
            self.hover_pending = None;
            self.set_hover(None);
            return;
        }
        let Some(position) = self.hover_pending.take() else {
            return;
        };
        let target = self.cast(position);
        self.set_hover(target.map(PickTarget::as_hover));
    }

    /// Write the hover target, requesting a redraw only if it moved.
    pub(crate) fn set_hover(&mut self, target: Option<HoverTarget>) {
        if self.ui.hover != target {
            self.ui.hover = target;
            self.redraw.requested = true;
        }
    }

    /// Apply a click at `position` to the selection.
    ///
    /// `mode` decides how it combines with what is already selected; the rule
    /// itself is `review_ui`'s, shared with the Outliner's rows so the two can
    /// never disagree about what the primary modifier or Shift means.
    pub(crate) fn pick_click(&mut self, position: Vec2, mode: review_ui::SelectMode) {
        if !self.picking_enabled() {
            return;
        }
        let hit = self.cast_hit(position);
        // In Aud, a click on a highlighted offender picks that finding rather
        // than the part under it.
        if self.ui.mode == WorkspaceMode::Aud
            && let Some(focus) = hit
                .and_then(|(_, triangle)| triangle)
                .and_then(|triangle| self.audit_offender_at(triangle))
        {
            self.ui.select_audit_focus(Some(focus));
            self.redraw.requested = true;
            return;
        }
        let target = hit.map(|(target, _)| target);
        review_ui::apply_pick(&mut self.ui, target.map(PickTarget::as_hover), mode);
        // The pick is also the freshest possible answer for the hover.
        self.set_hover(target.map(PickTarget::as_hover));
        // Framing should start from the new selection rather than continuing
        // whatever the previous one's `F` toggle had reached.
        self.frame_showing_selection = false;
        self.redraw.requested = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The threshold has to absorb the shake of clicking a mouse without ever
    /// letting a deliberate orbit select something.
    #[test]
    fn a_click_is_a_press_that_barely_moved() {
        let press = Vec2::new(400.0, 300.0);
        assert!(is_click(press, press), "not moving at all is a click");
        assert!(
            is_click(press, press + Vec2::new(2.0, 2.0)),
            "a couple of pixels of hand shake is still a click"
        );
        assert!(
            !is_click(press, press + Vec2::new(40.0, 0.0)),
            "a deliberate drag is not"
        );
        // Exactly at the threshold counts as a click, so the boundary is not a
        // dead zone where neither gesture happens.
        assert!(is_click(
            press,
            press + Vec2::new(crate::input::CLICK_MAX_DISTANCE_PX, 0.0)
        ));
    }
}
