//! The egui renderer: egui's tessellated output drawn through sokol_gfx.
//!
//! This replaced `egui-directx11`, which was the last thing pinning the workspace to
//! Direct3D 11 *and* to egui 0.33 (`mac-port-plan.md` D3/D6). It is deliberately the
//! smallest thing that draws the chrome correctly: one program, one interleaved
//! vertex stream, one index stream, a texture per egui texture id, and a sampler per
//! distinct [`egui::TextureOptions`].
//!
//! ## The frame
//!
//! [`EguiRenderer::prepare`] runs **outside any pass**: it applies the texture deltas
//! (which recreate images) and writes this frame's geometry into the transient
//! buffers, both of which sokol forbids inside a pass. [`EguiRenderer::paint`] runs
//! inside the swapchain pass, after whatever the scene composited, and only issues
//! draws. [`EguiRenderer::free_textures`] runs after the frame, because a texture
//! egui freed may still have been drawn from this one.
//!
//! ## Textures
//!
//! egui patches its font atlas by sub-rectangle, and `sg_update_image` is
//! whole-image and once-per-frame. So each texture keeps a **CPU shadow**: a delta is
//! applied into the shadow (whole or sub-rect) and any texture whose shadow changed
//! is recreated as a new immutable image. The atlas mutates for a few frames after
//! startup or a DPI change and then never again, so this costs a couple of 2048²
//! uploads at launch and nothing at all in steady state.
//!
//! ## Colour
//!
//! egui blends in gamma space: its vertex colours and its atlas are both sRGB bytes
//! with premultiplied alpha, and the fragment is a plain `texture * color` with no
//! conversion. That works because the swapchain is plain UNORM (D20) — a hardware
//! sRGB view on top would double-encode the chrome.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, Vertex};
use egui::{Context, TextureId, TextureOptions, TexturesDelta};

use crate::rhi::{
    Bindings, Blend, Filter, Format, Frame, GpuResult, Pipeline, PipelineDesc, Sampler, Texture,
    TransientBuffer, VertexFormat, shader,
};
use crate::shaders::generated;

/// Bytes per `epaint::Vertex`: `pos` (8) + `uv` (8) + `color` (4).
const VERTEX_SIZE: usize = std::mem::size_of::<Vertex>();
const _: () = assert!(VERTEX_SIZE == 20);

/// Starting capacity for the geometry streams, in bytes. Roughly a full window of
/// chrome, so the common case never reallocates; both buffers grow if it does.
const INITIAL_VERTEX_BYTES: usize = 64 * 1024;
const INITIAL_INDEX_BYTES: usize = 32 * 1024;

/// The `egui_params` uniform block, mirroring `review.glsl`.
///
/// The trailing pad is not ours: std140 rounds a block to 16 bytes, and the generated
/// struct carries it too — which is what the second assertion checks (invariant 11).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EguiUniforms {
    /// The logical screen size in points; the vertex positions arrive in the same
    /// space, and the vertex shader maps them to NDC with Y flipped.
    screen_size: [f32; 2],
    _pad: [f32; 2],
}
const _: () = assert!(std::mem::size_of::<EguiUniforms>() == 16);
const _: () =
    assert!(std::mem::size_of::<EguiUniforms>() == std::mem::size_of::<generated::EguiParams>());
crate::shaders::assert_same_layout!(EguiUniforms => generated::EguiParams, {
    screen_size => screen_size,
    _pad => egui_pad,
});

/// One egui texture: the GPU image and the CPU shadow it is rebuilt from.
struct EguiTexture {
    texture: Texture,
    /// RGBA8, row-major — the authoritative copy, since egui only ever sends patches
    /// of it and sokol cannot patch an image in place.
    shadow: Vec<u8>,
    width: usize,
    height: usize,
    options: TextureOptions,
}

/// One mesh ready to draw: where its slice of the shared streams starts, how much of
/// it there is, and what it reads.
struct PreparedMesh {
    texture: TextureId,
    /// The clip rectangle in physical pixels, already clamped to the framebuffer.
    scissor: [i32; 4],
    vertex_byte_offset: usize,
    index_byte_offset: usize,
    index_count: usize,
}

/// How a texture is sampled — the key its sampler is cached under. egui sets these
/// per texture, and the wrong one shows as a blurred icon or a bled atlas edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SamplerKey {
    min: Filter,
    mag: Filter,
    wrap: crate::rhi::Wrap,
}

impl SamplerKey {
    fn from(options: TextureOptions) -> Self {
        let filter = |f: egui::TextureFilter| match f {
            egui::TextureFilter::Nearest => Filter::Nearest,
            egui::TextureFilter::Linear => Filter::Linear,
        };
        Self {
            min: filter(options.minification),
            mag: filter(options.magnification),
            wrap: match options.wrap_mode {
                egui::TextureWrapMode::ClampToEdge => crate::rhi::Wrap::ClampToEdge,
                egui::TextureWrapMode::Repeat => crate::rhi::Wrap::Repeat,
                egui::TextureWrapMode::MirroredRepeat => crate::rhi::Wrap::MirroredRepeat,
            },
        }
    }
}

/// Draws the egui chrome.
pub struct EguiRenderer {
    pipeline: Pipeline,
    vertices: TransientBuffer,
    indices: TransientBuffer,
    textures: HashMap<TextureId, EguiTexture>,
    samplers: HashMap<SamplerKey, Sampler>,
    /// This frame's draw list, rebuilt by every [`Self::prepare`].
    meshes: Vec<PreparedMesh>,
    /// Scratch the frame's geometry is concatenated into before one upload each.
    /// Kept between frames so a steady-state frame allocates nothing.
    vertex_bytes: Vec<u8>,
    index_bytes: Vec<u8>,
    /// Textures egui freed this frame, dropped by [`Self::free_textures`] once the
    /// frame that may still have drawn from them has been submitted.
    pending_free: Vec<TextureId>,
}

// The egui pipeline uploads `epaint::Vertex` as it comes and declares its three
// attributes packed in order (`PipelineDesc::attributes`), so the struct has to
// be exactly that packing and the shader has to number the attributes the same
// way. Neither is checked at run time: a drift reads the stream shifted.
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(offset_of!(Vertex, pos) == 0);
    assert!(offset_of!(Vertex, uv) == 8);
    assert!(offset_of!(Vertex, color) == 16);
    assert!(size_of::<Vertex>() == 20);
    assert!(generated::ATTR_EGUI_IN_POS == 0);
    assert!(generated::ATTR_EGUI_IN_UV == 1);
    assert!(generated::ATTR_EGUI_IN_COLOR == 2);
};

impl EguiRenderer {
    /// Build the egui pipeline and its geometry streams.
    pub fn new() -> GpuResult<Self> {
        let shader = shader::make(
            generated::egui_shader_desc,
            shader::bytecode!("egui"),
            c"egui",
        )?;
        let pipeline = Pipeline::new(&PipelineDesc {
            attributes: &[
                VertexFormat::Float2,  // pos, in points
                VertexFormat::Float2,  // uv
                VertexFormat::Ubyte4N, // sRGB colour, premultiplied
            ],
            indexed: true,
            blend: Blend::PremultipliedAlpha,
            ..PipelineDesc::swapchain(shader, c"egui")
        })?;
        Ok(Self {
            pipeline,
            vertices: TransientBuffer::vertices(INITIAL_VERTEX_BYTES, c"egui vertices")?,
            indices: TransientBuffer::indices(INITIAL_INDEX_BYTES, c"egui indices")?,
            textures: HashMap::new(),
            samplers: HashMap::new(),
            meshes: Vec::new(),
            vertex_bytes: Vec::new(),
            index_bytes: Vec::new(),
            pending_free: Vec::new(),
        })
    }

    /// Apply this frame's texture deltas and upload its geometry. Runs **outside**
    /// any pass — both halves are resource updates sokol forbids inside one.
    ///
    /// `shapes` and `textures_delta` come straight out of egui's `FullOutput`;
    /// tessellation happens here rather than in `app` so the whole chain from shapes
    /// to draws lives in one place.
    pub fn prepare(
        &mut self,
        ctx: &Context,
        shapes: Vec<egui::epaint::ClippedShape>,
        mut textures_delta: TexturesDelta,
        pixels_per_point: f32,
        framebuffer: (u32, u32),
    ) -> GpuResult<()> {
        // Dropped first, before anything below can fail: `paint` draws from this list
        // against buffers *this* frame wrote, so an early `?` on a failed texture
        // upload must not leave last frame's meshes standing. sokol would catch it —
        // binding a transient buffer that was not written this frame is a validation
        // error — but it would do so on every frame from then on.
        self.meshes.clear();

        // One texture can collect several deltas in a frame — a full image followed by
        // patches — so they are applied in order, each into the CPU shadow.
        // One texture can collect several deltas in a frame — a full image followed
        // by patches — so they apply in order, each into the CPU shadow.
        for (&id, deltas) in &textures_delta.set {
            for delta in deltas {
                self.apply_delta(id, delta)?;
            }
        }
        self.pending_free.clear();
        self.pending_free
            .extend(textures_delta.free.iter().copied());
        // Taken by value and emptied on purpose: a `TexturesDelta` panics if it is
        // dropped with deltas still in it, which is egui's way of catching a renderer
        // that silently ignored an atlas update. Clearing here is the acknowledgement,
        // and it happens only after every delta above has actually been applied — an
        // early `?` leaves it un-cleared, so a failed upload still trips the check.
        textures_delta.clear();

        let primitives = ctx.tessellate(shapes, pixels_per_point);
        self.pack(&primitives, pixels_per_point, framebuffer);
        self.vertices.write(&self.vertex_bytes)?;
        self.indices.write(&self.index_bytes)?;
        Ok(())
    }

    /// Draw the prepared chrome. Runs **inside** the swapchain pass, after everything
    /// the scene composited into it.
    pub fn paint(&self, frame: &Frame<'_>, pixels_per_point: f32) {
        let (width, height) = frame.size();
        if self.meshes.is_empty() || width == 0 || height == 0 {
            return;
        }
        // The composite narrows the viewport to lay the Opt split out; the chrome
        // covers the whole window, so it takes it back first.
        frame.set_viewport(0, 0, width as i32, height as i32);
        frame.apply_pipeline(&self.pipeline);
        frame.apply_uniforms(
            generated::UB_EGUI_PARAMS,
            &EguiUniforms {
                screen_size: [
                    width as f32 / pixels_per_point,
                    height as f32 / pixels_per_point,
                ],
                _pad: [0.0; 2],
            },
        );

        for mesh in &self.meshes {
            let Some(texture) = self.textures.get(&mesh.texture) else {
                // A mesh naming a texture we never received a delta for. egui does
                // not do this for its own (`Managed`) ids; a `User` id would, and
                // nothing in this viewer registers one.
                continue;
            };
            let Some(sampler) = self.samplers.get(&SamplerKey::from(texture.options)) else {
                continue;
            };
            let mut bindings = Bindings::new();
            bindings.vertices(&self.vertices, mesh.vertex_byte_offset);
            bindings.indices(&self.indices, mesh.index_byte_offset);
            bindings.texture(generated::VIEW_EGUI_TEXTURE, &texture.texture);
            bindings.sampler(generated::SMP_EGUI_SAMPLER, sampler);
            frame.apply_bindings(&bindings);
            let [x, y, w, h] = mesh.scissor;
            frame.set_scissor(x, y, w, h);
            frame.draw(0, mesh.index_count);
        }
        // Leave the scissor covering the whole target: sokol carries pass state
        // forward, and the next frame's composite does not set one.
        frame.set_scissor(0, 0, width as i32, height as i32);
    }

    /// Drop the textures egui freed this frame. Called *after* the frame is
    /// submitted, since the frame just drawn may still have read from them.
    pub fn free_textures(&mut self) {
        for id in self.pending_free.drain(..) {
            self.textures.remove(&id);
        }
    }

    /// Apply one delta into the texture's CPU shadow and rebuild its image.
    ///
    /// A whole-image delta replaces the shadow; a positioned one copies its rows into
    /// the existing shadow. Either way the image is recreated, because sokol has no
    /// sub-rectangle update.
    fn apply_delta(&mut self, id: TextureId, delta: &ImageDelta) -> GpuResult<()> {
        let egui::epaint::ImageData::Color(image) = &delta.image;
        let (patch_w, patch_h) = (image.size[0], image.size[1]);
        let patch = image.as_raw();

        match delta.pos {
            None => {
                self.textures.insert(
                    id,
                    EguiTexture {
                        texture: upload(patch, patch_w, patch_h)?,
                        shadow: patch.to_vec(),
                        width: patch_w,
                        height: patch_h,
                        options: delta.options,
                    },
                );
            }
            Some([x, y]) => {
                let Some(entry) = self.textures.get_mut(&id) else {
                    // A patch for a texture that was never allocated. egui does not
                    // emit one; dropping it beats indexing into a shadow that is not
                    // there.
                    return Ok(());
                };
                if x + patch_w > entry.width || y + patch_h > entry.height {
                    // Likewise out of contract, and a silent out-of-bounds write into
                    // the shadow would corrupt the whole atlas rather than one glyph.
                    return Ok(());
                }
                for row in 0..patch_h {
                    let src = row * patch_w * 4;
                    let dst = ((y + row) * entry.width + x) * 4;
                    entry.shadow[dst..dst + patch_w * 4]
                        .copy_from_slice(&patch[src..src + patch_w * 4]);
                }
                entry.options = delta.options;
                entry.texture = upload(&entry.shadow, entry.width, entry.height)?;
            }
        }

        // The sampler this texture wants, created once per distinct options value.
        let key = SamplerKey::from(delta.options);
        if let std::collections::hash_map::Entry::Vacant(slot) = self.samplers.entry(key) {
            slot.insert(Sampler::new(key.min, key.mag, key.wrap, c"egui")?);
        }
        Ok(())
    }

    /// Concatenate the frame's meshes into the two byte streams and record where each
    /// one landed, dropping the ones that cannot draw.
    fn pack(
        &mut self,
        primitives: &[ClippedPrimitive],
        pixels_per_point: f32,
        framebuffer: (u32, u32),
    ) {
        self.meshes.clear();
        self.vertex_bytes.clear();
        self.index_bytes.clear();
        let (fb_width, fb_height) = (framebuffer.0 as i32, framebuffer.1 as i32);

        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else {
                // `Primitive::Callback` is egui's escape hatch for a caller that wants
                // to issue its own draws mid-chrome. This viewer draws its scene in
                // its own passes and registers none.
                continue;
            };
            if mesh.indices.is_empty() {
                continue;
            }
            // egui's clip rect is in points; the scissor is in physical pixels, and
            // must be clamped — a rect measured during the egui pass can outlive a
            // resize landing before the draw.
            let clip = primitive.clip_rect;
            let x = (clip.min.x * pixels_per_point)
                .round()
                .clamp(0.0, fb_width as f32) as i32;
            let y = (clip.min.y * pixels_per_point)
                .round()
                .clamp(0.0, fb_height as f32) as i32;
            let right = (clip.max.x * pixels_per_point)
                .round()
                .clamp(0.0, fb_width as f32) as i32;
            let bottom = (clip.max.y * pixels_per_point)
                .round()
                .clamp(0.0, fb_height as f32) as i32;
            if right <= x || bottom <= y {
                continue;
            }

            self.meshes.push(PreparedMesh {
                texture: mesh.texture_id,
                scissor: [x, y, right - x, bottom - y],
                vertex_byte_offset: self.vertex_bytes.len(),
                index_byte_offset: self.index_bytes.len(),
                index_count: mesh.indices.len(),
            });
            self.vertex_bytes
                .extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
            self.index_bytes
                .extend_from_slice(bytemuck::cast_slice(&mesh.indices));
        }
    }
}

/// Upload one RGBA8 image as an immutable texture.
fn upload(pixels: &[u8], width: usize, height: usize) -> GpuResult<Texture> {
    Texture::immutable_2d(
        pixels,
        width as u32,
        height as u32,
        Format::Rgba8,
        c"egui texture",
    )
}
