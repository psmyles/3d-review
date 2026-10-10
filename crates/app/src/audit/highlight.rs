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

/// How close, in points, a click must land to a highlighted edge or dot to
/// pick it: about the drawn dot's radius plus the shake of a click.
pub(crate) const AUDIT_PICK_TOLERANCE_POINTS: f32 = 6.0;

/// Most edge and dot elements one click tests. The highlight is capped far
/// higher, but a click on a scene with a million offending points has its
/// answer long before the millionth.
const PICK_ELEMENT_CAP: usize = 1 << 18;

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

/// Distance from `point` to the segment `a`–`b`.
fn segment_distance(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let length_squared = along.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance(a);
    }
    let t = ((point - a).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(a + along * t)
}

/// The offender of `focus` whose drawn edge or dot lies nearest `pointer`,
/// within `tolerance` — the marks a ray cannot hit because they have no
/// surface of their own. `position` gives a render corner's world position (as
/// drawn: posed when a clip is), `project` a world point's place on screen in
/// the pointer's own pixels.
pub(crate) fn offender_near(
    report: &AuditReport,
    focus: AuditFocus,
    model: &ModelData,
    position: impl Fn(u32) -> Option<Vec3>,
    project: impl Fn(Vec3) -> Option<Vec2>,
    pointer: Vec2,
    tolerance: f32,
) -> Option<AuditFocus> {
    let screen = |corner: u32| position(corner).and_then(&project);
    // Which nodes draw triangles: built once, and only if an object-level
    // offender asks.
    let owns_triangles = std::cell::OnceCell::new();
    let owns = |node: u32| {
        owns_triangles
            .get_or_init(|| {
                let mut owns = vec![false; model.nodes.len()];
                for &owner in &model.triangles.node {
                    if let Some(slot) = owns.get_mut(owner as usize) {
                        *slot = true;
                    }
                }
                owns
            })
            .get(node as usize)
            .copied()
            .unwrap_or(false)
    };
    let mut best: Option<(f32, RuleId, u32)> = None;
    let mut budget = PICK_ELEMENT_CAP;
    let mut consider = |distance: f32, rule: RuleId, node: u32| {
        if distance <= tolerance && best.is_none_or(|(nearest, _, _)| distance < nearest) {
            best = Some((distance, rule, node));
        }
    };
    for (rule, _, offender) in focused(report, focus) {
        let Some(node) = offender.node else {
            continue;
        };
        match (&offender.elements, rule.marker()) {
            (ElementSet::Edges(edges), _) => {
                for &[a, b] in edges.iter().take(budget) {
                    if let (Some(a), Some(b)) = (screen(a), screen(b)) {
                        consider(segment_distance(pointer, a, b), rule, node);
                    }
                }
                budget = budget.saturating_sub(edges.len());
            }
            (ElementSet::Vertices(corners), _) => {
                for &corner in corners.iter().take(budget) {
                    if let Some(at) = screen(corner) {
                        consider(pointer.distance(at), rule, node);
                    }
                }
                budget = budget.saturating_sub(corners.len());
            }
            (ElementSet::Triangles(set), Marker::Dot) => {
                for triangle in set.iter().take(budget) {
                    if let Some(at) = model
                        .indices
                        .get(triangle as usize * 3)
                        .and_then(|&corner| screen(corner))
                    {
                        consider(pointer.distance(at), rule, node);
                    }
                }
                budget = budget.saturating_sub(set.len() as usize);
            }
            (ElementSet::Points(points), _) => {
                for (point, _) in points.iter().take(budget) {
                    if let Some(at) = project(Vec3::from_array(*point)) {
                        consider(pointer.distance(at), rule, node);
                    }
                }
                budget = budget.saturating_sub(points.len());
            }
            (ElementSet::None, _) => {
                // An object drawing nothing of its own is marked at its pivot.
                if !owns(node)
                    && let Some(scene_node) = model.nodes.get(node as usize)
                    && let Some(at) = project(scene_node.transform.w_axis.truncate())
                {
                    consider(pointer.distance(at), rule, node);
                }
            }
            (ElementSet::Triangles(_), _) => {}
        }
        if budget == 0 {
            break;
        }
    }
    best.map(|(_, rule, node)| AuditFocus::Offender(rule, node as usize))
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
            Some(RuleId::LightmapPadding) => review_audit::LIGHTMAP_PADDING_CHANNEL as u32,
            Some(rule @ (RuleId::LightmapOverlap | RuleId::LightmapOutOfRange)) => {
                self.ui
                    .aud
                    .profile
                    .rule(rule)
                    .number("channel")
                    .unwrap_or(1.0) as u32
            }
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
                texture_size: number(RuleId::TexelDensity, "texture_size", 1024.0),
                target: number(RuleId::TexelDensity, "target", 1024.0),
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

    /// The overdraw view the Aud toolbar has chosen, as the renderer's — `None`
    /// on an adapter that cannot draw it (invariant 4), whatever the state says.
    pub(crate) fn audit_overdraw_view(&self) -> review_render::OverdrawView {
        let view = self.ui.aud.view;
        if !self.ui.aud.view_supported(view, &self.ui.capabilities) {
            return review_render::OverdrawView::None;
        }
        match view {
            review_audit::DiagnosticView::Overdraw => review_render::OverdrawView::Layered,
            review_audit::DiagnosticView::QuadOverdraw => review_render::OverdrawView::Quad,
            review_audit::DiagnosticView::Issues
            | review_audit::DiagnosticView::TexelDensity
            | review_audit::DiagnosticView::TriangleDensity => review_render::OverdrawView::None,
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

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use review_audit::{
        AuditReport, ElementSet, Offender, RuleId, RuleResult, Severity, Status, Threshold,
    };
    use review_ui::AuditFocus;

    use super::offender_near;

    fn failing(rule: RuleId, offenders: Vec<Offender>) -> RuleResult {
        RuleResult {
            rule,
            status: Status::Fail,
            severity: Severity::Warning,
            measured: None,
            threshold: Threshold::None,
            total: offenders.iter().map(|offender| offender.count).sum(),
            offenders,
            truncated: false,
            measure_key: 0,
        }
    }

    /// A click near a drawn edge or dot picks its offender, the nearer of two
    /// candidates wins, and one beyond the tolerance picks nothing.
    #[test]
    fn edges_and_dots_are_picked_by_screen_distance() {
        let model = review_model::demo_cube_model();
        let report = AuditReport {
            results: vec![
                failing(
                    RuleId::NonManifoldEdges,
                    vec![Offender::elements(Some(0), ElementSet::Edges(vec![[0, 1]]))],
                ),
                failing(
                    RuleId::IsolatedVertices,
                    vec![Offender::elements(
                        Some(0),
                        ElementSet::Points(vec![([50.0, -3.0, 0.0], 7)]),
                    )],
                ),
            ],
            ..Default::default()
        };
        // Corner 0 at (0, 0) and corner 1 at (100, 0) on screen; the world is
        // the screen, flattened.
        let position = |corner: u32| match corner {
            0 => Some(Vec3::ZERO),
            1 => Some(Vec3::new(100.0, 0.0, 0.0)),
            _ => None,
        };
        let project = |world: Vec3| Some(world.truncate());
        let pick =
            |focus, pointer| offender_near(&report, focus, &model, position, project, pointer, 6.0);

        let edges = AuditFocus::Rule(RuleId::NonManifoldEdges);
        assert_eq!(
            pick(edges, Vec2::new(30.0, 4.0)),
            Some(AuditFocus::Offender(RuleId::NonManifoldEdges, 0)),
            "within the tolerance of the segment's middle",
        );
        assert_eq!(pick(edges, Vec2::new(30.0, 9.0)), None, "beyond it");
        assert_eq!(
            pick(edges, Vec2::new(104.0, 0.0)),
            Some(AuditFocus::Offender(RuleId::NonManifoldEdges, 0)),
            "past the end, but within the tolerance of it",
        );

        // Focused on the object, both marks are candidates: the nearer wins.
        let object = AuditFocus::Object(0);
        assert_eq!(
            pick(object, Vec2::new(50.0, -2.0)),
            Some(AuditFocus::Offender(RuleId::IsolatedVertices, 0)),
            "the dot is nearer than the edge under it",
        );
        assert_eq!(
            pick(object, Vec2::new(20.0, 3.0)),
            Some(AuditFocus::Offender(RuleId::NonManifoldEdges, 0)),
        );
    }
}
