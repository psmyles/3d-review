//! Display names for values whose types live in another crate.
//!
//! `render`, `optimize`, `model` and `import` each carry a `label()` on their
//! public enums. Those stay: they are stable English identifiers, and they go
//! into logs, the pipeline's own warnings and the export report, where a
//! translated string makes a bug report harder to read rather than easier.
//! `model` could not localize them in any case — it depends on `glam` alone
//! (invariant 10).
//!
//! So the mapping lives here, on the one side of the boundary that draws text.
//! Each `match` is exhaustive, which is the point: a variant added upstream is a
//! compile error here rather than an English word appearing in a translated
//! screen.

use review_localization::Key;
use review_model::{NodeKind, SkinningMethod};
use review_optimize::{
    AoQuality, AoTarget, FbxFormat, HierarchyMode, LodPackaging, NormalMode, OpKind, RemeshDensity,
    RemeshTopology, SimplifyAlgorithm,
};
use review_render::{
    AlphaMode, BufferView, ChannelSelect, CheckerTexture, EnvironmentMap, GtaoQuality,
    MaterialMode, MsaaSamples, RoughnessWorkflow, TextureSlot, TonemapOperator, VertexColorMode,
    ViewportBackground,
};

use crate::keys;
use crate::opt_state::{ComparisonSide, GhostStyle};
use crate::state::{TextureBackground, ViewportTool, WorkspaceMode};

pub(crate) fn material_mode(mode: MaterialMode) -> Key {
    match mode {
        MaterialMode::Source => keys::ui_enums::MATERIAL_MODE_SOURCE,
        MaterialMode::Standard => keys::ui_enums::MATERIAL_MODE_STANDARD,
        MaterialMode::Unique => keys::ui_enums::MATERIAL_MODE_UNIQUE,
    }
}

pub(crate) fn buffer_view(view: BufferView) -> Key {
    match view {
        BufferView::BaseColor => keys::ui_enums::BUFFER_BASE_COLOR,
        BufferView::WorldNormal => keys::ui_enums::BUFFER_WORLD_NORMAL,
        BufferView::NormalMap => keys::ui_enums::BUFFER_NORMAL_MAP,
        BufferView::GeometricNormal => keys::ui_enums::BUFFER_GEOMETRIC_NORMAL,
        BufferView::Tangent => keys::ui_enums::BUFFER_TANGENT,
        BufferView::Roughness => keys::ui_enums::BUFFER_ROUGHNESS,
        BufferView::Metallic => keys::ui_enums::BUFFER_METALLIC,
        BufferView::AmbientOcclusion => keys::ui_enums::BUFFER_AO,
        BufferView::Emission => keys::ui_enums::BUFFER_EMISSION,
        BufferView::Opacity => keys::ui_enums::BUFFER_OPACITY,
        BufferView::Uv => keys::ui_enums::BUFFER_UV,
    }
}

/// The Roughness buffer's label follows the material's workflow: on a
/// smoothness-workflow material the shader inverts the value, so calling the row
/// "Roughness" would name the opposite of what is drawn.
pub(crate) fn buffer_view_for(view: BufferView, workflow: RoughnessWorkflow) -> Key {
    match (view, workflow) {
        (BufferView::Roughness, RoughnessWorkflow::Smoothness) => {
            keys::ui_enums::WORKFLOW_SMOOTHNESS
        }
        _ => buffer_view(view),
    }
}

pub(crate) fn checker(texture: CheckerTexture) -> Key {
    match texture {
        CheckerTexture::Greyscale => keys::ui_enums::CHECKER_GREYSCALE,
        CheckerTexture::Color => keys::ui_enums::CHECKER_COLOR,
    }
}

pub(crate) fn vertex_color_mode(mode: VertexColorMode) -> Key {
    match mode {
        VertexColorMode::Rgb => keys::ui_enums::VERTEX_COLOR_RGB,
        VertexColorMode::Alpha => keys::ui_enums::VERTEX_COLOR_ALPHA,
        VertexColorMode::RgbAlpha => keys::ui_enums::VERTEX_COLOR_RGB_ALPHA,
    }
}

pub(crate) fn msaa(samples: MsaaSamples) -> Key {
    match samples {
        MsaaSamples::Off => keys::ui_enums::MSAA_OFF,
        MsaaSamples::X2 => keys::ui_enums::MSAA_2X,
        MsaaSamples::X4 => keys::ui_enums::MSAA_4X,
        MsaaSamples::X8 => keys::ui_enums::MSAA_8X,
        MsaaSamples::X16 => keys::ui_enums::MSAA_16X,
    }
}

pub(crate) fn environment(map: EnvironmentMap) -> Key {
    match map {
        EnvironmentMap::Hdr01 => keys::ui_enums::ENV_HDR_01,
        EnvironmentMap::Hdr02 => keys::ui_enums::ENV_HDR_02,
        EnvironmentMap::Hdr03 => keys::ui_enums::ENV_HDR_03,
        EnvironmentMap::Hdr04 => keys::ui_enums::ENV_HDR_04,
        EnvironmentMap::Hdr05 => keys::ui_enums::ENV_HDR_05,
        EnvironmentMap::Hdr06 => keys::ui_enums::ENV_HDR_06,
    }
}

pub(crate) fn gtao_quality(quality: GtaoQuality) -> Key {
    match quality {
        GtaoQuality::Low => keys::ui_enums::QUALITY_LOW,
        GtaoQuality::Medium => keys::ui_enums::QUALITY_MEDIUM,
        GtaoQuality::High => keys::ui_enums::QUALITY_HIGH,
    }
}

pub(crate) fn tonemap(operator: TonemapOperator) -> Key {
    match operator {
        TonemapOperator::PbrNeutral => keys::ui_enums::TONEMAP_PBR_NEUTRAL,
        TonemapOperator::Linear => keys::ui_enums::TONEMAP_LINEAR,
        TonemapOperator::Reinhard => keys::ui_enums::TONEMAP_REINHARD,
        TonemapOperator::Aces => keys::ui_enums::TONEMAP_ACES,
        TonemapOperator::Agx => keys::ui_enums::TONEMAP_AGX,
    }
}

pub(crate) fn viewport_background(background: ViewportBackground) -> Key {
    match background {
        ViewportBackground::Black => keys::ui_enums::BACKGROUND_BLACK,
        ViewportBackground::Grey25 => keys::ui_enums::BACKGROUND_GREY_25,
        ViewportBackground::Grey50 => keys::ui_enums::BACKGROUND_GREY_50,
        ViewportBackground::Grey75 => keys::ui_enums::BACKGROUND_GREY_75,
        ViewportBackground::White => keys::ui_enums::BACKGROUND_WHITE,
        ViewportBackground::Gradient => keys::ui_enums::BACKGROUND_GRADIENT,
    }
}

pub(crate) fn texture_background(background: TextureBackground) -> Key {
    match background {
        TextureBackground::Black => keys::ui_enums::BACKGROUND_BLACK,
        TextureBackground::White => keys::ui_enums::BACKGROUND_WHITE,
        TextureBackground::Grey => keys::ui_enums::BACKGROUND_GREY,
        TextureBackground::Checker => keys::ui_enums::BACKGROUND_CHECKER,
    }
}

pub(crate) fn workflow(workflow: RoughnessWorkflow) -> Key {
    match workflow {
        RoughnessWorkflow::Roughness => keys::ui_enums::WORKFLOW_ROUGHNESS,
        RoughnessWorkflow::Smoothness => keys::ui_enums::WORKFLOW_SMOOTHNESS,
    }
}

pub(crate) fn alpha_mode(mode: AlphaMode) -> Key {
    match mode {
        AlphaMode::Opaque => keys::ui_enums::ALPHA_OPAQUE,
        AlphaMode::Blend => keys::ui_enums::ALPHA_BLEND,
        AlphaMode::Clip => keys::ui_enums::ALPHA_CLIP,
    }
}

pub(crate) fn texture_slot(slot: TextureSlot) -> Key {
    match slot {
        TextureSlot::BaseColor => keys::ui_enums::SLOT_BASE_COLOR,
        TextureSlot::Normal => keys::ui_enums::SLOT_NORMAL,
        TextureSlot::Roughness => keys::ui_enums::SLOT_ROUGHNESS,
        TextureSlot::Metallic => keys::ui_enums::SLOT_METALLIC,
        TextureSlot::Ao => keys::ui_enums::SLOT_AO,
        TextureSlot::Emissive => keys::ui_enums::SLOT_EMISSIVE,
        TextureSlot::Opacity => keys::ui_enums::SLOT_OPACITY,
    }
}

/// A texture slot's own label, except that the roughness slot follows the
/// material's workflow — see [`buffer_view_for`].
pub(crate) fn texture_slot_for(slot: TextureSlot, workflow: RoughnessWorkflow) -> Key {
    match (slot, workflow) {
        (TextureSlot::Roughness, RoughnessWorkflow::Smoothness) => {
            keys::ui_enums::WORKFLOW_SMOOTHNESS
        }
        _ => texture_slot(slot),
    }
}

pub(crate) fn channel(channel: ChannelSelect) -> Key {
    match channel {
        ChannelSelect::R => keys::ui_enums::CHANNEL_R,
        ChannelSelect::G => keys::ui_enums::CHANNEL_G,
        ChannelSelect::B => keys::ui_enums::CHANNEL_B,
        ChannelSelect::A => keys::ui_enums::CHANNEL_A,
        ChannelSelect::Rgb => keys::ui_enums::CHANNEL_RGB,
    }
}

pub(crate) fn node_kind(kind: NodeKind) -> Key {
    match kind {
        NodeKind::Mesh => keys::ui_enums::NODE_MESH,
        NodeKind::Bone => keys::ui_enums::NODE_BONE,
        NodeKind::Light => keys::ui_enums::NODE_LIGHT,
        NodeKind::Camera => keys::ui_enums::NODE_CAMERA,
        NodeKind::Empty => keys::ui_enums::NODE_EMPTY,
        NodeKind::Other => keys::ui_enums::NODE_OTHER,
    }
}

pub(crate) fn skinning_method(method: SkinningMethod) -> Key {
    match method {
        SkinningMethod::Linear => keys::ui_enums::SKINNING_LINEAR,
        SkinningMethod::Rigid => keys::ui_enums::SKINNING_RIGID,
        SkinningMethod::DualQuaternion => keys::ui_enums::SKINNING_DQ,
        SkinningMethod::BlendedDqLinear => keys::ui_enums::SKINNING_BLENDED_DQ,
    }
}

pub(crate) fn workspace(mode: WorkspaceMode) -> Key {
    match mode {
        WorkspaceMode::ThreeD => keys::ui_enums::WORKSPACE_3D,
        WorkspaceMode::Uv => keys::ui_enums::WORKSPACE_UV,
        WorkspaceMode::Texture => keys::ui_enums::WORKSPACE_TEX,
        WorkspaceMode::Opt => keys::ui_enums::WORKSPACE_OPT,
    }
}

pub(crate) fn op_kind(kind: &OpKind) -> Key {
    match kind {
        OpKind::Weld(_) => keys::ui_enums::OP_WELD,
        OpKind::FilterTriangles => keys::ui_enums::OP_FILTER_TRIANGLES,
        OpKind::PruneComponents { .. } => keys::ui_enums::OP_PRUNE_COMPONENTS,
        OpKind::Reduce(_) => keys::ui_enums::OP_REDUCE,
        OpKind::Remesh(_) => keys::ui_enums::OP_REMESH,
        OpKind::Shrinkwrap(_) => keys::ui_enums::OP_SHRINKWRAP,
        OpKind::RecalculateNormals(_) => keys::ui_enums::OP_RECALCULATE_NORMALS,
        OpKind::SimplifyLod(_) => keys::ui_enums::OP_SIMPLIFY_LOD,
        OpKind::BakeAo(_) => keys::ui_enums::OP_BAKE_AO,
        OpKind::VertexCache => keys::ui_enums::OP_VERTEX_CACHE,
        OpKind::Overdraw { .. } => keys::ui_enums::OP_OVERDRAW,
        OpKind::VertexFetch => keys::ui_enums::OP_VERTEX_FETCH,
    }
}

/// An operation's display name, for the progress line `app` shows while a run
/// is going. See [`material_mode_name`]: the stack row and the notice have to
/// call an operation the same thing, and only this crate holds the names.
pub fn op_kind_name(kind: &OpKind) -> String {
    review_localization::tr(op_kind(kind)).into_owned()
}

/// What an operation does, in a sentence - the stack row's tooltip and the
/// heading text over its parameters. Paired with [`op_kind`] so the name and the
/// explanation cannot drift apart.
pub(crate) fn op_description(kind: &OpKind) -> Key {
    match kind {
        OpKind::Weld(_) => keys::ui_enums::OP_WELD_DESCRIPTION,
        OpKind::FilterTriangles => keys::ui_enums::OP_FILTER_TRIANGLES_DESCRIPTION,
        OpKind::PruneComponents { .. } => keys::ui_enums::OP_PRUNE_COMPONENTS_DESCRIPTION,
        OpKind::Reduce(_) => keys::ui_enums::OP_REDUCE_DESCRIPTION,
        OpKind::Remesh(_) => keys::ui_enums::OP_REMESH_DESCRIPTION,
        OpKind::Shrinkwrap(_) => keys::ui_enums::OP_SHRINKWRAP_DESCRIPTION,
        OpKind::RecalculateNormals(_) => keys::ui_enums::OP_RECALCULATE_NORMALS_DESCRIPTION,
        OpKind::SimplifyLod(_) => keys::ui_enums::OP_SIMPLIFY_LOD_DESCRIPTION,
        OpKind::BakeAo(_) => keys::ui_enums::OP_BAKE_AO_DESCRIPTION,
        OpKind::VertexCache => keys::ui_enums::OP_VERTEX_CACHE_DESCRIPTION,
        OpKind::Overdraw { .. } => keys::ui_enums::OP_OVERDRAW_DESCRIPTION,
        OpKind::VertexFetch => keys::ui_enums::OP_VERTEX_FETCH_DESCRIPTION,
    }
}

pub(crate) fn normal_mode(mode: NormalMode) -> Key {
    match mode {
        NormalMode::Project => keys::ui_enums::NORMAL_MODE_PROJECT,
        NormalMode::Generate => keys::ui_enums::NORMAL_MODE_GENERATE,
    }
}

pub(crate) fn remesh_topology(topology: RemeshTopology) -> Key {
    match topology {
        RemeshTopology::Triangles => keys::ui_enums::REMESH_TOPOLOGY_TRIANGLES,
        RemeshTopology::Quads => keys::ui_enums::REMESH_TOPOLOGY_QUADS,
    }
}

pub(crate) fn remesh_density(density: RemeshDensity) -> Key {
    match density {
        RemeshDensity::Ratio => keys::ui_enums::REMESH_DENSITY_RATIO,
        RemeshDensity::Absolute => keys::ui_enums::REMESH_DENSITY_ABSOLUTE,
    }
}

pub(crate) fn simplify_algorithm(algorithm: SimplifyAlgorithm) -> Key {
    match algorithm {
        SimplifyAlgorithm::Standard => keys::ui_enums::SIMPLIFY_STANDARD,
        SimplifyAlgorithm::WithAttributes => keys::ui_enums::SIMPLIFY_ATTRIBUTES,
        SimplifyAlgorithm::Sloppy => keys::ui_enums::SIMPLIFY_SLOPPY,
    }
}

pub(crate) fn ao_quality(quality: AoQuality) -> Key {
    match quality {
        AoQuality::Low => keys::ui_enums::AO_QUALITY_LOW,
        AoQuality::Medium => keys::ui_enums::AO_QUALITY_MEDIUM,
        AoQuality::High => keys::ui_enums::AO_QUALITY_HIGH,
        AoQuality::Ultra => keys::ui_enums::AO_QUALITY_ULTRA,
    }
}

pub(crate) fn ao_target(target: AoTarget) -> Key {
    match target {
        AoTarget::Alpha => keys::ui_enums::AO_TARGET_ALPHA,
        AoTarget::Rgb => keys::ui_enums::AO_TARGET_RGB,
        AoTarget::MultiplyRgb => keys::ui_enums::AO_TARGET_MULTIPLY_RGB,
        AoTarget::Red => keys::ui_enums::AO_TARGET_RED,
        AoTarget::Green => keys::ui_enums::AO_TARGET_GREEN,
        AoTarget::Blue => keys::ui_enums::AO_TARGET_BLUE,
    }
}

pub(crate) fn lod_packaging(packaging: LodPackaging) -> Key {
    match packaging {
        LodPackaging::SingleFileSuffixed => keys::ui_enums::LOD_SINGLE_FILE,
        LodPackaging::FilePerLod => keys::ui_enums::LOD_FILE_PER_LOD,
    }
}

pub(crate) fn hierarchy_mode(mode: HierarchyMode) -> Key {
    match mode {
        HierarchyMode::Rebuild => keys::ui_enums::HIERARCHY_REBUILD,
        HierarchyMode::FlatBaked => keys::ui_enums::HIERARCHY_FLAT,
    }
}

pub(crate) fn fbx_format(format: FbxFormat) -> Key {
    match format {
        FbxFormat::Binary => keys::ui_enums::FBX_BINARY,
        FbxFormat::Ascii => keys::ui_enums::FBX_ASCII,
    }
}

pub(crate) fn ghost_style(style: GhostStyle) -> Key {
    match style {
        GhostStyle::Xray => keys::ui_enums::GHOST_XRAY,
        GhostStyle::Wireframe => keys::ui_enums::GHOST_WIREFRAME,
    }
}

pub(crate) fn comparison_side(side: ComparisonSide) -> Key {
    match side {
        ComparisonSide::Source => keys::ui_enums::SIDE_SOURCE,
        ComparisonSide::Processed => keys::ui_enums::SIDE_PROCESSED,
    }
}

/// A material mode's display name, for `app`'s mode notice.
///
/// `app` raises the notice (it owns the redraw loop and the notification queue),
/// but the *names* belong to the chrome's catalog: the notice and the toolbar
/// button have to agree, and only this crate holds the `ui-enums-` messages.
/// Two functions rather than exposing the whole map, because these are the only
/// two names anything outside this crate needs.
pub fn material_mode_name(mode: MaterialMode) -> String {
    review_localization::tr(material_mode(mode)).into_owned()
}

/// A buffer view's display name, for `app`'s mode notice. See
/// [`material_mode_name`].
pub fn buffer_view_name(view: BufferView) -> String {
    review_localization::tr(buffer_view(view)).into_owned()
}

pub(crate) fn viewport_tool(tool: ViewportTool) -> Key {
    match tool {
        ViewportTool::View => keys::ui_enums::TOOL_VIEW,
        ViewportTool::Select => keys::ui_enums::TOOL_SELECT,
    }
}

/// The viewport tool's display name, for `app`'s mode notice. See
/// [`material_mode_name`].
pub fn viewport_tool_name(tool: ViewportTool) -> String {
    review_localization::tr(viewport_tool(tool)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mapped variant must resolve to real text. `tr` falls back to the
    /// key's own id when no catalog defines it, so an id coming back out is a
    /// message that was never written.
    fn assert_resolves(key: Key) {
        assert_ne!(
            review_localization::tr(key),
            key.id(),
            "`{}` has no message in the catalog",
            key.id()
        );
    }

    #[test]
    fn every_render_enum_variant_resolves() {
        for value in MaterialMode::ALL {
            assert_resolves(material_mode(value));
        }
        for value in BufferView::ALL {
            assert_resolves(buffer_view(value));
        }
        for value in MsaaSamples::ALL {
            assert_resolves(msaa(value));
        }
        for value in EnvironmentMap::ALL {
            assert_resolves(environment(value));
        }
        for value in GtaoQuality::ALL {
            assert_resolves(gtao_quality(value));
        }
        for value in TonemapOperator::ALL {
            assert_resolves(tonemap(value));
        }
        for value in ViewportBackground::ALL {
            assert_resolves(viewport_background(value));
        }
        for value in AlphaMode::ALL {
            assert_resolves(alpha_mode(value));
        }
        for value in TextureSlot::ALL {
            assert_resolves(texture_slot(value));
        }
    }

    #[test]
    fn every_model_and_optimize_enum_variant_resolves() {
        for value in NodeKind::ALL {
            assert_resolves(node_kind(value));
        }
        for value in AoQuality::ALL {
            assert_resolves(ao_quality(value));
        }
        for value in AoTarget::ALL {
            assert_resolves(ao_target(value));
        }
        for value in SimplifyAlgorithm::ALL {
            assert_resolves(simplify_algorithm(value));
        }
        for value in RemeshDensity::ALL {
            assert_resolves(remesh_density(value));
        }
        for value in RemeshTopology::ALL {
            assert_resolves(remesh_topology(value));
        }
        for value in LodPackaging::ALL {
            assert_resolves(lod_packaging(value));
        }
        for value in HierarchyMode::ALL {
            assert_resolves(hierarchy_mode(value));
        }
        for value in FbxFormat::ALL {
            assert_resolves(fbx_format(value));
        }
        // `OpKind::ALL` is a table of builders, because several variants carry
        // parameters and there is no bare value to list.
        for make in OpKind::ALL {
            let kind = make();
            assert_resolves(op_kind(&kind));
            assert_resolves(op_description(&kind));
        }
    }
}
