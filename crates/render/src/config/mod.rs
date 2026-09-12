//! Renderer configuration + per-view option types: shading / projection /
//! anti-aliasing / environment / GTAO / tone-map / UV / debug-overlay
//! settings, the adapter-capability probe, and `RendererConfig`. These form the
//! UI→render and model→render *option contract* (a clean seam for the future
//! renderer swap); `Renderer` and the cameras live in the crate root and read
//! these as plain values.//!
//! ## Layout
//!
//! The shading mode, the projection and the anti-aliasing settings are here with
//! [`RendererConfig`] that aggregates everything; [`lighting`] holds the
//! environment / occlusion / tone-mapping options, [`material`] what the mesh is
//! shaded with, and [`debug`] the per-view toggles.

mod debug;
mod lighting;
mod material;

pub use debug::*;
pub use lighting::*;
pub use material::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShadingMode {
    /// Wireframe only: the filled surface is not drawn, just its edges.
    Wireframe,
    /// Filled faces showing the active material as flat emissive color (no light).
    Unlit,
    /// Filled faces lit and shaded with the active material color.
    #[default]
    Shaded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CameraProjection {
    #[default]
    Perspective,
    Orthographic,
}

/// Multisample level for the offscreen scene render (the geometry MSAA): the
/// per-edge antialiasing of the 3D scene, chosen at runtime. `Off` renders
/// single-sample (no resolve); the rest render multisampled and resolve to a
/// single-sample texture the composite pass samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MsaaSamples {
    Off,
    X2,
    #[default]
    X4,
    X8,
    X16,
}

impl MsaaSamples {
    /// Every variant in ascending order, for building UI menus.
    pub const ALL: [MsaaSamples; 5] = [
        MsaaSamples::Off,
        MsaaSamples::X2,
        MsaaSamples::X4,
        MsaaSamples::X8,
        MsaaSamples::X16,
    ];

    /// Short menu label.
    pub fn label(self) -> &'static str {
        match self {
            MsaaSamples::Off => "Off",
            MsaaSamples::X2 => "2x",
            MsaaSamples::X4 => "4x",
            MsaaSamples::X8 => "8x",
            MsaaSamples::X16 => "16x",
        }
    }

    /// The MSAA sample count this level maps to (`Off` = 1).
    pub fn sample_count(self) -> u32 {
        match self {
            MsaaSamples::Off => 1,
            MsaaSamples::X2 => 2,
            MsaaSamples::X4 => 4,
            MsaaSamples::X8 => 8,
            MsaaSamples::X16 => 16,
        }
    }
}

/// The viewer's antialiasing configuration: a master on/off plus the MSAA level.
/// Read by the scene renderer to size the offscreen targets / scene pipelines.
///
/// `enabled` is the toolbar toggle (left-click): when off, the scene renders with
/// no antialiasing at all regardless of `msaa`, but the level is retained so
/// toggling back on restores it. The default is enabled at 4× MSAA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntiAliasing {
    pub enabled: bool,
    pub msaa: MsaaSamples,
}

impl Default for AntiAliasing {
    fn default() -> Self {
        Self {
            enabled: true,
            msaa: MsaaSamples::X4,
        }
    }
}

impl AntiAliasing {
    /// The MSAA sample count to actually render at: the chosen level when AA is
    /// enabled, otherwise 1 (single-sample, no resolve).
    pub fn effective_sample_count(self) -> u32 {
        if self.enabled {
            self.msaa.sample_count()
        } else {
            1
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RendererConfig {
    /// Retained renderer-config seam (linear RGBA, 0..1). The scene pass now clears
    /// its offscreen targets to zero and the *visible* viewport backdrop is the
    /// composited [`ViewportBackground`] (display-space, after tone mapping), so this
    /// no longer drives the on-screen background; kept for the config seam.
    pub clear_color: [f32; 4],
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            clear_color: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_view_next_cycles_through_all_in_order() {
        // `next` walks `ALL` in order and wraps after the last back to the first.
        let mut view = BufferView::ALL[0];
        for expected in BufferView::ALL
            .iter()
            .skip(1)
            .chain(std::iter::once(&BufferView::ALL[0]))
        {
            view = view.next();
            assert_eq!(view, *expected);
        }
        // A full lap returns to the start.
        assert_eq!(view, BufferView::ALL[0]);
    }

    #[test]
    fn viewport_background_next_cycles_through_all_in_order() {
        // `next` walks `ALL` in order and wraps after the last back to the first.
        let mut background = ViewportBackground::ALL[0];
        for expected in ViewportBackground::ALL
            .iter()
            .skip(1)
            .chain(std::iter::once(&ViewportBackground::ALL[0]))
        {
            background = background.next();
            assert_eq!(background, *expected);
        }
        assert_eq!(background, ViewportBackground::ALL[0]);
    }

    #[test]
    fn viewport_background_gradient_is_flat_except_gradient_preset() {
        // Every flat preset returns equal top/bottom; only the gradient differs, and
        // its top (80% grey) is brighter than its bottom (black).
        for background in ViewportBackground::ALL {
            let (top, bottom) = background.gradient_srgb();
            if background == ViewportBackground::Gradient {
                assert!(top[0] > bottom[0], "gradient top should be brighter");
                assert_eq!(bottom, [0.0; 3]);
            } else {
                assert_eq!(top, bottom, "{background:?} should be a flat fill");
            }
        }
    }

    #[test]
    fn buffer_view_shader_indices_are_unique_and_match_all_order() {
        // Each variant's shader index equals its position in `ALL` (the order the
        // `fs_main` buffer-view switch arms read), so the two stay in lockstep.
        for (position, view) in BufferView::ALL.into_iter().enumerate() {
            assert_eq!(view.shader_index(), position as f32, "{view:?}");
        }
    }
}
