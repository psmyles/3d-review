//! What the Aud viewport highlights: the focused finding's offenders, resolved
//! into the triangle, edge and dot lists the renderer draws, plus the box that
//! frames them.
//!
//! Resolved once per change of focus or report, never per frame, and handed to
//! the renderer as borrowed slices under a revision (`AuditOverlay`): the lists
//! index the model's own triangles and corners, so nothing is copied and the
//! highlight deforms with the mesh.

use glam::{Vec2, Vec3};
use review_audit::{AuditReport, ElementSet, Marker, Offender, RuleId, Severity};
use review_model::{Bounds, ModelData};
use review_ui::AuditFocus;

use crate::App;

/// How many points a framing box and the on-screen test sample at most.
const SAMPLE_CAP: usize = 4096;

/// The resolved highlight, and what it was resolved from.
#[derive(Debug, Default)]
pub(crate) struct AuditHighlight {
    /// The report (by address) and focus this was built for.
    key: Option<(usize, AuditFocus)>,
    pub(crate) revision: u64,
    pub(crate) fills: [Vec<u32>; 3],
    pub(crate) edges: [Vec<[u32; 2]>; 3],
    pub(crate) corner_dots: [Vec<u32>; 3],
    pub(crate) point_dots: [Vec<([f32; 3], u32)>; 3],
    /// Up to [`SAMPLE_CAP`] world positions spread over the offenders.
    samples: Vec<Vec3>,
    pub(crate) bounds: Option<Bounds>,
}

impl AuditHighlight {
    fn clear(&mut self) {
        let revision = self.revision;
        *self = AuditHighlight {
            revision,
            ..Default::default()
        };
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.key.is_none()
    }

    /// Add one offender of a rule at `severity`.
    fn add(&mut self, model: &ModelData, rule: RuleId, severity: Severity, offender: &Offender) {
        let s = severity.index();
        match (&offender.elements, rule.marker()) {
            (ElementSet::Triangles(set), Marker::Dot) => {
                // Too small to see as a fill: a dot on each triangle's first corner.
                for triangle in set.iter() {
                    if let Some(&corner) = model.indices.get(triangle as usize * 3) {
                        self.corner_dots[s].push(corner);
                    }
                }
            }
            (ElementSet::Triangles(set), _) => self.fills[s].extend(set.iter()),
            (ElementSet::Edges(edges), _) => self.edges[s].extend_from_slice(edges),
            (ElementSet::Vertices(corners), _) => self.corner_dots[s].extend_from_slice(corners),
            (ElementSet::Points(points), _) => {
                let node = offender.node.unwrap_or(0);
                self.point_dots[s].extend(points.iter().map(|(position, _)| (*position, node)));
            }
            (ElementSet::None, _) => {
                // An object-level finding: the whole object, or a dot at its
                // pivot when it draws nothing of its own.
                let Some(node) = offender.node else {
                    return;
                };
                let before = self.fills[s].len();
                if model.triangles.node.len() == model.indices.len() / 3 {
                    self.fills[s].extend(
                        model
                            .triangles
                            .node
                            .iter()
                            .enumerate()
                            .filter(|&(_, &owner)| owner == node)
                            .map(|(triangle, _)| triangle as u32),
                    );
                }
                if self.fills[s].len() == before
                    && let Some(scene_node) = model.nodes.get(node as usize)
                {
                    let position = scene_node.transform.w_axis.truncate().to_array();
                    self.point_dots[s].push((position, node));
                }
            }
        }
    }

    /// Spread up to [`SAMPLE_CAP`] positions over everything highlighted and
    /// bound them.
    fn measure(&mut self, model: &ModelData) {
        let corner = |corner: u32| model.vertices.get(corner as usize).map(|v| v.position);
        let mut points: Vec<Vec3> = Vec::new();
        for s in 0..3 {
            points.extend(
                self.fills[s]
                    .iter()
                    .filter_map(|&t| model.indices.get(t as usize * 3).copied().and_then(corner)),
            );
            points.extend(self.edges[s].iter().filter_map(|&[a, _]| corner(a)));
            points.extend(self.corner_dots[s].iter().filter_map(|&c| corner(c)));
            points.extend(self.point_dots[s].iter().map(|(p, _)| Vec3::from_array(*p)));
        }
        let step = (points.len() / SAMPLE_CAP).max(1);
        self.samples = points.into_iter().step_by(step).collect();
        let mut bounds = Bounds::EMPTY;
        for &point in &self.samples {
            bounds.include_point(point);
        }
        self.bounds = (!bounds.is_empty()).then_some(bounds);
    }
}

/// Every (rule, severity, offender) the focus covers.
fn focused(report: &AuditReport, focus: AuditFocus) -> Vec<(RuleId, Severity, &Offender)> {
    let mut out = Vec::new();
    for result in report.results.iter().filter(|result| result.failed()) {
        for offender in &result.offenders {
            let wanted = match focus {
                AuditFocus::Rule(rule) => result.rule == rule,
                AuditFocus::Offender(rule, node) => {
                    result.rule == rule && offender.node == Some(node as u32)
                }
                AuditFocus::Object(node) => offender.node == Some(node as u32),
            };
            if wanted {
                out.push((result.rule, result.severity, offender));
            }
        }
    }
    out
}

impl App {
    /// Bring the highlight in line with the focus and the report, and frame the
    /// offenders when a new focus has none of them on screen.
    pub(crate) fn sync_audit_highlight(&mut self) {
        let focus = (self.ui.mode == review_ui::WorkspaceMode::Aud)
            .then_some(self.ui.aud.focus)
            .flatten();
        let report = self.ui.aud.report.clone();
        let key = focus
            .zip(report.as_ref())
            .map(|(focus, report)| (std::sync::Arc::as_ptr(report) as usize, focus));
        let highlight = &mut self.audit.highlight;
        if highlight.key == key {
            return;
        }
        let focus_changed = highlight.key.map(|(_, focus)| focus) != key.map(|(_, focus)| focus);
        highlight.clear();
        highlight.revision = highlight.revision.wrapping_add(1);
        let (Some(key), Some(report)) = (key, report) else {
            self.redraw.requested = true;
            return;
        };
        highlight.key = Some(key);
        let model = &self.scene_model;
        for (rule, severity, offender) in focused(&report, key.1) {
            highlight.add(model, rule, severity, offender);
        }
        highlight.measure(model);
        self.redraw.requested = true;
        if focus_changed {
            if self.ui.aud.split() {
                self.fit_audit_uv_camera();
            }
            self.frame_audit_if_off_screen();
        }
    }

    /// The UV set the focused rule reads: the lightmap rules' profile channel,
    /// UV0 for the rest.
    pub(crate) fn audit_uv_channel(&self) -> u32 {
        match self.ui.aud.focus.and_then(AuditFocus::rule) {
            Some(
                rule @ (RuleId::LightmapOverlap
                | RuleId::LightmapPadding
                | RuleId::LightmapOutOfRange),
            ) => self
                .ui
                .aud
                .profile
                .rule(rule)
                .number("channel")
                .unwrap_or(1.0) as u32,
            _ => 0,
        }
    }

    /// The objects the focus names, sorted — what the UV half lays out.
    pub(crate) fn audit_focus_nodes(&self) -> Vec<u32> {
        let (Some(focus), Some(report)) = (self.ui.aud.focus, self.ui.aud.report.as_ref()) else {
            return Vec::new();
        };
        let mut nodes: Vec<u32> = focused(report, focus)
            .into_iter()
            .filter_map(|(_, _, offender)| offender.node)
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
    }

    /// Frame the Aud split's UV half on the focused offenders' UVs.
    fn fit_audit_uv_camera(&mut self) {
        let channel = self.audit_uv_channel() as usize;
        let model = &self.scene_model;
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for fills in &self.audit.highlight.fills {
            for &triangle in fills {
                for &corner in model
                    .indices
                    .get(triangle as usize * 3..triangle as usize * 3 + 3)
                    .unwrap_or(&[])
                {
                    let uv = model.uv_for_channel(corner as usize, channel);
                    min = min.min(uv);
                    max = max.max(uv);
                }
            }
        }
        if min.x.is_finite()
            && let Some(renderer) = self.renderer.as_mut()
        {
            renderer.fit_aud_uv_camera(min, max);
        }
    }

    /// The density view the Aud toolbar has chosen, as the renderer's heat map,
    /// parameterised by the checks it mirrors.
    pub(crate) fn audit_heat_map(&self) -> review_render::HeatMap {
        let profile = &self.ui.aud.profile;
        let number = |rule: RuleId, key: &str, fallback: f64| {
            profile.rule(rule).number(key).unwrap_or(fallback) as f32
        };
        match self.ui.aud.view {
            review_audit::DiagnosticView::TexelDensity => review_render::HeatMap::TexelDensity {
                texture_size: number(RuleId::TexelDensity, "texture_size", 2048.0),
                target: number(RuleId::TexelDensity, "target", 512.0),
                tolerance: number(RuleId::TexelDensity, "tolerance", 2.0),
            },
            review_audit::DiagnosticView::TriangleDensity => {
                review_render::HeatMap::TriangleDensity {
                    min_pixel_area: number(RuleId::TriangleLod, "min_pixel_area", 10.0),
                    screen_height: number(RuleId::TriangleLod, "screen_height", 1080.0),
                }
            }
            _ => review_render::HeatMap::None,
        }
    }

    /// Ease the camera onto the focused offenders, but only when none of them is
    /// in view — a click that already shows its problem should not move the view.
    fn frame_audit_if_off_screen(&mut self) {
        if self.audit.highlight.bounds.is_none() {
            return;
        }
        let (Some(renderer), Some(egui_ctx), Some(viewport)) = (
            self.renderer.as_ref(),
            self.egui_ctx.as_ref(),
            self.ui.scene_viewport,
        ) else {
            return;
        };
        let screen = egui_ctx.content_rect();
        let size = Vec2::new(screen.width(), screen.height());
        let projection = self.ui.projection_mode.into();
        let visible = self.audit.highlight.samples.iter().any(|&point| {
            renderer
                .camera
                .project(point, projection, size)
                .is_some_and(|pixel| viewport.contains(egui::pos2(pixel.x, pixel.y)))
        });
        if !visible {
            self.frame_audit_focus();
        }
    }

    /// Frame the focused offenders (the `F` key in Aud with a focus). A box with
    /// no size — one point — is padded to a share of the model so the camera
    /// does not dive into it.
    pub(crate) fn frame_audit_focus(&mut self) -> bool {
        let Some(mut bounds) = self.audit.highlight.bounds else {
            return false;
        };
        let minimum = self
            .scene_model
            .bounds
            .map_or(0.1, |model| model.radius() * 0.1);
        let half = (bounds.size() * 0.5).max(Vec3::splat(minimum));
        let center = bounds.center();
        bounds = Bounds {
            min: center - half,
            max: center + half,
        };
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.animate_camera_to_bounds(bounds);
        }
        true
    }

    /// The offender under a click on triangle `triangle`, as the focus that
    /// narrows to it — or `None` when the click hit no highlighted face.
    pub(crate) fn audit_offender_at(&self, triangle: u32) -> Option<AuditFocus> {
        let focus = self.ui.aud.focus?;
        let report = self.ui.aud.report.as_ref()?;
        focused(report, focus)
            .into_iter()
            .find(|(rule, _, offender)| match &offender.elements {
                ElementSet::Triangles(set) => {
                    rule.marker() != Marker::Dot && set.contains(triangle)
                }
                ElementSet::None => {
                    let owner = self.scene_model.triangles.node.get(triangle as usize);
                    offender.node.is_some() && owner.copied() == offender.node
                }
                _ => false,
            })
            .and_then(|(rule, _, offender)| {
                offender
                    .node
                    .map(|node| AuditFocus::Offender(rule, node as usize))
            })
    }
}
