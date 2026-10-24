//! A model's thumbnail: one still frame at the viewer's default angle.

use review_render::GpuResult;

use crate::{Host, Image, PreviewScene};

/// More frames than ambient occlusion ever takes to settle (24); a cap, not a
/// target.
const MAX_FRAMES: usize = 64;

/// Draw `scene` as a `size`×`size` thumbnail over no background: the model
/// framed at the viewer's default three-quarter view, with its textures and its
/// ambient occlusion fully converged.
///
/// Drawn at twice the size and halved, so a tile a few dozen pixels across
/// still has clean silhouettes — the scene's MSAA alone leaves the stair-steps
/// of a small image visible, and supersampling is nearly free at this size.
pub fn render_thumbnail(host: &mut Host, scene: &PreviewScene, size: u32) -> GpuResult<Image> {
    let side = size.max(1) * 2;
    let mut view = host.thumbnails();
    view.camera().frame(scene, (side, side));
    for _ in 0..MAX_FRAMES {
        view.render(scene, (side, side))?;
        if !view.camera().is_settling() {
            break;
        }
    }
    Ok(view.read_back()?.halved())
}
