//! The Vulkan device and the window swapchain sokol_gfx draws through on Linux —
//! invariant 9's sanctioned GPU site there, and the only file in the workspace that
//! names Vulkan (`docs/ARCHITECTURE.md`, D28).
//!
//! It is the twin of [`super::d3d11`] and [`super::metal`]: same contract, same shape,
//! no trait between them (Platform decisions: platform leaves). What sokol_gfx cannot
//! do for itself is create a device, point `sg_environment` at it, put a surface on the
//! window, hand over a render target per frame, and present — and that is all this
//! module is. Five things differ from the other two, and each is load-bearing:
//!
//! * **The host makes the device to sokol's order.** sokol's Vulkan backend needs
//!   Vulkan 1.3 (dynamic rendering, synchronization2, buffer device addresses),
//!   `VK_EXT_descriptor_buffer`, anisotropic samplers and dual-source blending, and the
//!   renderer adds BC texture compression (the baked IBL cubes and the UV checker).
//!   [`Device::create`] picks the best device offering all of it — a discrete GPU over
//!   an integrated one over a software rasterizer such as Mesa's lavapipe, which is the
//!   Vulkan stand-in for the D3D11 side's WARP.
//! * **Every object shares one reference-counted [`Context`].** `Gpu` drops its device
//!   before its swapchain, and a swapchain destroyed after its `VkDevice` is undefined
//!   behaviour; on D3D11 the swapchain's own COM reference keeps the device alive, and
//!   here the swapchain's `Arc` does.
//! * **sokol submits, and waits and signals two semaphores we hand it.** Its frame
//!   submit waits on the swapchain's `present_complete_semaphore` (what
//!   `vkAcquireNextImageKHR` signals) and signals `render_finished_semaphore` (what
//!   `vkQueuePresentKHR` waits on), so the frame's one swapchain pass is what ties the
//!   two together. A frame dropped before its pass leaves one of them signaled, and the
//!   next [`Swapchain::acquire`] rebuilds the chain and its semaphores rather than
//!   reuse them.
//! * **The windowless backbuffer is an image of our own handed over as the swapchain
//!   image** — the D3D11 side's shape, not Metal's. sokol can neither wrap a native
//!   `VkImage` nor say which one is behind an image of its own, so an ordinary sokol
//!   attachment could never be read back; an image we own can, by a copy that waits on
//!   the frame's `render_finished_semaphore`.
//! * **The bake's readback is not here** ([`read_image_subresource`]), for that same
//!   reason: it reads images sokol made. The IBL bake runs on Windows (D19); this OS
//!   builds the `bake` feature so it stays checked, and the tool says why it stops.
//!
//! Surfaces come from winit's raw handles on Wayland, Xlib or XCB, whichever the window
//! is on; the instance enables every surface extension the loader offers, because the
//! device is made on the bring-up thread before any window exists (D8).

#![allow(
    unsafe_code,
    reason = "invariant 9: the Linux device and swapchain leaf"
)]

use std::ffi::{CStr, c_char, c_void};
use std::sync::Arc;

use ash::vk;
use ash::vk::Handle;
use sokol::gfx as sg;
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;

use crate::rhi::error::{GpuError, GpuResult, ResourceKind};
use crate::rhi::format::Format;
use crate::rhi::gpu_profiler::{TIMESTAMP_SLOTS, Zone};
use crate::rhi::present::PresentStatus;

/// What the backbuffer is created as, and what sokol_gfx is told to expect of a
/// swapchain pass. `Bgra8`, as on Metal: it is the one 8-bit UNORM format every
/// Wayland and X11 surface offers, and D20 holds unchanged — a plain UNORM target the
/// composite / egui / Tex shaders sRGB-encode themselves.
pub(crate) const SWAPCHAIN_FORMAT: sg::PixelFormat = sg::PixelFormat::Bgra8;
/// The same, as Vulkan names it (sokol's own mapping of `Bgra8`).
const VK_SWAPCHAIN_FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;

/// How many acquire semaphores the window swapchain cycles through: one more than
/// sokol's frames in flight (`SG_NUM_INFLIGHT_FRAMES`, 2). sokol waits on a frame
/// slot's fence before reusing it, so by the time a semaphore comes round again the
/// submit that waited on it three frames ago has finished.
const ACQUIRE_RING: usize = 3;

/// The backbuffers the window swapchain asks for, at least: two, as the D3D11 flip
/// chain and the Metal layer have, unless the surface insists on more.
const BACKBUFFERS: u32 = 2;

/// How long [`Swapchain::acquire`] waits for an image before skipping the frame.
/// Long enough for any vsync-paced display; short enough that a compositor which stops
/// releasing images (a hidden Wayland window under FIFO) cannot hang the event loop.
const ACQUIRE_TIMEOUT_NS: u64 = 250_000_000;

/// How long a CPU wait on our own GPU work (a readback copy) may take before it is an
/// error rather than a stall. Seconds: a frame plus its copy, on the slowest device.
#[cfg(feature = "headless")]
const COPY_TIMEOUT_NS: u64 = 10_000_000_000;

/// The Khronos validation layer, enabled in debug builds when it is installed
/// (`vulkan-validation-layers`). sokol's own validation covers its API; this covers
/// what the driver is handed, this module's calls included.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// The instance, the device and the queue sokol runs on, destroyed with the last
/// object holding them (see the module header).
struct Context {
    entry: ash::Entry,
    instance: ash::Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    surfaces: SurfaceExtensions,
    physical: vk::PhysicalDevice,
    properties: vk::PhysicalDeviceProperties,
    /// Where the windowless backbuffer's image and readback buffer are allocated.
    #[cfg_attr(
        not(feature = "headless"),
        allow(dead_code, reason = "read only by the windowless backbuffer")
    )]
    memory: vk::PhysicalDeviceMemoryProperties,
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    /// `VK_KHR_swapchain` was enabled: always on a GPU with a display engine, and
    /// what a windowless device may do without.
    swapchain: bool,
    /// The queue family stamps timestamps (`timestampValidBits`), and how many of
    /// each value's bits mean anything.
    timestamp_bits: u32,
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: this is the last reference, so every object made from the device —
        // the swapchain, its surface, the timer, the offscreen target, and sokol's
        // own, which `sg_shutdown` released before the `Device` dropped — is already
        // gone. Waiting for the device to go idle first means nothing the GPU is still
        // executing loses its device, and the messenger goes before its instance.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            if let Some((utils, messenger)) = self.debug.take() {
                utils.destroy_debug_utils_messenger(messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// Which window-system surface extensions the instance enabled.
#[derive(Clone, Copy, Default)]
struct SurfaceExtensions {
    surface: bool,
    wayland: bool,
    xlib: bool,
    xcb: bool,
}

/// A failed Vulkan call as the renderer's error.
fn vk_error(kind: ResourceKind, label: &str) -> impl Fn(vk::Result) -> GpuError + '_ {
    move |result| GpuError::Resource {
        kind,
        label: label.to_owned(),
        detail: format!("{result:?}"),
    }
}

/// A device-creation failure with a sentence for its detail.
fn device_error(detail: impl Into<String>) -> GpuError {
    GpuError::Resource {
        kind: ResourceKind::Device,
        label: "Vulkan device".into(),
        detail: detail.into(),
    }
}

/// A dispatchable or non-dispatchable handle as the opaque pointer sokol's structs
/// hold. Dispatchable handles *are* pointers; the others are 64-bit values sokol
/// casts straight back.
fn raw<H: Handle>(handle: H) -> *const c_void {
    handle.as_raw() as usize as *const c_void
}

/// The process's Vulkan device — what sokol_gfx runs on.
pub(crate) struct Device {
    ctx: Arc<Context>,
}

impl Device {
    /// The best device on the system that offers everything sokol and the renderer
    /// need (see the module header), with one queue that does graphics, compute and
    /// transfer.
    pub(crate) fn create() -> GpuResult<Device> {
        let entry = ash::Entry::linked();

        // SAFETY: global loader queries, valid before any instance exists.
        let (version, extensions, layers) = unsafe {
            (
                entry
                    .try_enumerate_instance_version()
                    .map_err(vk_error(ResourceKind::Device, "Vulkan loader"))?
                    .unwrap_or(vk::API_VERSION_1_0),
                entry
                    .enumerate_instance_extension_properties(None)
                    .map_err(vk_error(ResourceKind::Device, "Vulkan loader"))?,
                entry
                    .enumerate_instance_layer_properties()
                    .unwrap_or_default(),
            )
        };
        if version < vk::API_VERSION_1_3 {
            return Err(device_error(format!(
                "the Vulkan loader is version {}.{}, and 3D Review needs 1.3",
                vk::api_version_major(version),
                vk::api_version_minor(version)
            )));
        }
        let offered = |name: &CStr| {
            extensions
                .iter()
                .any(|ext| ext.extension_name_as_c_str() == Ok(name))
        };

        let mut surfaces = SurfaceExtensions::default();
        let mut enabled: Vec<*const c_char> = Vec::new();
        if offered(ash::khr::surface::NAME) {
            surfaces.surface = true;
            enabled.push(ash::khr::surface::NAME.as_ptr());
            for (name, flag) in [
                (ash::khr::wayland_surface::NAME, &mut surfaces.wayland),
                (ash::khr::xlib_surface::NAME, &mut surfaces.xlib),
                (ash::khr::xcb_surface::NAME, &mut surfaces.xcb),
            ] {
                if offered(name) {
                    *flag = true;
                    enabled.push(name.as_ptr());
                }
            }
        }
        let validate = cfg!(debug_assertions)
            && offered(ash::ext::debug_utils::NAME)
            && layers
                .iter()
                .any(|layer| layer.layer_name_as_c_str() == Ok(VALIDATION_LAYER));
        let mut enabled_layers: Vec<*const c_char> = Vec::new();
        if validate {
            enabled.push(ash::ext::debug_utils::NAME.as_ptr());
            enabled_layers.push(VALIDATION_LAYER.as_ptr());
        }

        let app = vk::ApplicationInfo::default()
            .application_name(c"3D Review")
            .engine_name(c"sokol_gfx")
            .api_version(vk::API_VERSION_1_3);
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&enabled)
            .enabled_layer_names(&enabled_layers);
        // SAFETY: every name pointer lives in `enabled` / `enabled_layers`, which
        // outlive the call, and each was checked against what the loader offers.
        let instance = unsafe { entry.create_instance(&info, None) }
            .map_err(vk_error(ResourceKind::Device, "Vulkan instance"))?;

        let debug = validate.then(|| {
            let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
            let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(log_validation));
            // SAFETY: the instance enabled `VK_EXT_debug_utils` (`validate`), and the
            // callback is a plain function that only reads what it is handed.
            unsafe { utils.create_debug_utils_messenger(&info, None) }
                .ok()
                .map(|messenger| (utils, messenger))
        });
        if validate {
            log::info!("Vulkan validation layer on (debug build)");
        }

        match open_device(&instance) {
            Ok(opened) => Ok(Device {
                ctx: Arc::new(Context {
                    entry,
                    instance,
                    debug: debug.flatten(),
                    surfaces,
                    physical: opened.physical,
                    properties: opened.properties,
                    memory: opened.memory,
                    device: opened.device,
                    queue: opened.queue,
                    queue_family: opened.queue_family,
                    swapchain: opened.swapchain,
                    timestamp_bits: opened.timestamp_bits,
                }),
            }),
            Err(err) => {
                // SAFETY: nothing else was made from this instance; the messenger goes
                // before it, as `Context::drop` does it.
                unsafe {
                    if let Some((utils, messenger)) = debug.flatten() {
                        utils.destroy_debug_utils_messenger(messenger, None);
                    }
                    instance.destroy_instance(None);
                }
                Err(err)
            }
        }
    }

    /// Point `env` at this device, so `sg_setup` runs on it. sokol keeps the handles
    /// as given; `rhi`'s `SokolDevice` shuts it down before this value drops.
    pub(crate) fn fill_environment(&self, env: &mut sg::Environment) {
        env.vulkan = sg::VulkanEnvironment {
            instance: raw(self.ctx.instance.handle()),
            physical_device: raw(self.ctx.physical),
            device: raw(self.ctx.device.handle()),
            queue: raw(self.ctx.queue),
            queue_family_index: self.ctx.queue_family,
        };
    }

    /// The device's name as the driver reports it (`NVIDIA GeForce RTX 3060 Ti`,
    /// `llvmpipe (LLVM 23.1.5, 256 bits)`).
    pub(crate) fn adapter_name(&self) -> String {
        self.ctx
            .properties
            .device_name_as_c_str()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The MSAA sample counts this device supports for **both** formats as attachments
    /// — the subset of `[1, 2, 4, 8, 16]` the scene can actually render at (invariant
    /// 4). `1` is always included. Asked per format, as the D3D11 twin does.
    pub(crate) fn supported_sample_counts(&self, color: Format, depth: Format) -> Vec<u32> {
        let counts = |format: Format, usage: vk::ImageUsageFlags| -> vk::SampleCountFlags {
            let Some(format) = vk_format(format) else {
                return vk::SampleCountFlags::TYPE_1;
            };
            // SAFETY: a read-only capability query on the device's own physical device.
            unsafe {
                self.ctx
                    .instance
                    .get_physical_device_image_format_properties(
                        self.ctx.physical,
                        format,
                        vk::ImageType::TYPE_2D,
                        vk::ImageTiling::OPTIMAL,
                        usage,
                        vk::ImageCreateFlags::empty(),
                    )
            }
            .map(|props| props.sample_counts)
            .unwrap_or(vk::SampleCountFlags::TYPE_1)
        };
        let both = counts(color, vk::ImageUsageFlags::COLOR_ATTACHMENT)
            & counts(depth, vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT);
        [
            (1u32, vk::SampleCountFlags::TYPE_1),
            (2, vk::SampleCountFlags::TYPE_2),
            (4, vk::SampleCountFlags::TYPE_4),
            (8, vk::SampleCountFlags::TYPE_8),
            (16, vk::SampleCountFlags::TYPE_16),
        ]
        .into_iter()
        .filter(|&(count, flag)| count == 1 || both.contains(flag))
        .map(|(count, _)| count)
        .collect()
    }
}

/// The renderer's attachment formats as Vulkan names them — sokol's own mapping, for
/// the two formats the sample-count question is asked about. Every other format is
/// sampled only, and answers single-sample.
fn vk_format(format: Format) -> Option<vk::Format> {
    match format {
        Format::Rgba16F => Some(vk::Format::R16G16B16A16_SFLOAT),
        Format::Depth32F => Some(vk::Format::D32_SFLOAT),
        Format::Swapchain => Some(VK_SWAPCHAIN_FORMAT),
        _ => None,
    }
}

/// What [`open_device`] picked and made.
struct Opened {
    physical: vk::PhysicalDevice,
    properties: vk::PhysicalDeviceProperties,
    memory: vk::PhysicalDeviceMemoryProperties,
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    swapchain: bool,
    timestamp_bits: u32,
}

/// A physical device that can run the renderer, ranked.
struct Candidate {
    physical: vk::PhysicalDevice,
    rank: u32,
    queue_family: u32,
    swapchain: bool,
    timestamp_bits: u32,
}

/// Pick the best suitable physical device and create the logical device on it, with
/// exactly the features sokol's Vulkan backend turns on (`sokol_app.h`'s
/// `_sapp_vk_create_device`, the reference host), plus BC compression.
fn open_device(instance: &ash::Instance) -> GpuResult<Opened> {
    // SAFETY: a read-only query on a live instance.
    let physicals = unsafe { instance.enumerate_physical_devices() }
        .map_err(vk_error(ResourceKind::Device, "Vulkan devices"))?;
    let mut rejected = Vec::new();
    let mut best: Option<Candidate> = None;
    for physical in physicals {
        match suitability(instance, physical) {
            Ok(candidate) => {
                if best.as_ref().is_none_or(|best| candidate.rank > best.rank) {
                    best = Some(candidate);
                }
            }
            Err(reason) => rejected.push(reason),
        }
    }
    let Some(picked) = best else {
        return Err(device_error(if rejected.is_empty() {
            "no Vulkan device: install the GPU's Vulkan driver (or Mesa's lavapipe)".into()
        } else {
            format!(
                "no Vulkan device can run 3D Review: {}",
                rejected.join("; ")
            )
        }));
    };

    let mut extensions = vec![ash::ext::descriptor_buffer::NAME.as_ptr()];
    if picked.swapchain {
        extensions.push(ash::khr::swapchain::NAME.as_ptr());
    }
    let mut descriptor_buffer =
        vk::PhysicalDeviceDescriptorBufferFeaturesEXT::default().descriptor_buffer(true);
    let mut vk12 = vk::PhysicalDeviceVulkan12Features::default().buffer_device_address(true);
    let mut vk13 = vk::PhysicalDeviceVulkan13Features::default()
        .dynamic_rendering(true)
        .synchronization2(true);
    let core = vk::PhysicalDeviceFeatures::default()
        .sampler_anisotropy(true)
        .dual_src_blend(true)
        .texture_compression_bc(true);
    let mut features = vk::PhysicalDeviceFeatures2::default()
        .features(core)
        .push_next(&mut descriptor_buffer)
        .push_next(&mut vk12)
        .push_next(&mut vk13);
    let priorities = [1.0f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(picked.queue_family)
        .queue_priorities(&priorities)];
    let info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queues)
        .enabled_extension_names(&extensions)
        .push_next(&mut features);
    // SAFETY: every extension and feature asked for was checked as supported by
    // `suitability`, and every pointer in `info` outlives the call.
    let device = unsafe { instance.create_device(picked.physical, &info, None) }
        .map_err(vk_error(ResourceKind::Device, "Vulkan device"))?;
    // SAFETY: queue 0 of the family the device was just created with one queue of;
    // the property queries are read-only.
    let (queue, properties, memory) = unsafe {
        (
            device.get_device_queue(picked.queue_family, 0),
            instance.get_physical_device_properties(picked.physical),
            instance.get_physical_device_memory_properties(picked.physical),
        )
    };
    Ok(Opened {
        physical: picked.physical,
        properties,
        memory,
        device,
        queue,
        queue_family: picked.queue_family,
        swapchain: picked.swapchain,
        timestamp_bits: picked.timestamp_bits,
    })
}

/// Whether `physical` can run the renderer, and how much it is preferred; or, when it
/// cannot, a phrase saying why, for the error when nothing can.
fn suitability(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Result<Candidate, String> {
    // SAFETY: read-only queries on a physical device the instance enumerated.
    let (properties, extensions, families) = unsafe {
        (
            instance.get_physical_device_properties(physical),
            instance
                .enumerate_device_extension_properties(physical)
                .unwrap_or_default(),
            instance.get_physical_device_queue_family_properties(physical),
        )
    };
    let name = properties
        .device_name_as_c_str()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if properties.api_version < vk::API_VERSION_1_3 {
        return Err(format!(
            "{name} has Vulkan {}.{}, not 1.3",
            vk::api_version_major(properties.api_version),
            vk::api_version_minor(properties.api_version)
        ));
    }
    let offered = |wanted: &CStr| {
        extensions
            .iter()
            .any(|ext| ext.extension_name_as_c_str() == Ok(wanted))
    };
    if !offered(ash::ext::descriptor_buffer::NAME) {
        return Err(format!("{name} has no VK_EXT_descriptor_buffer"));
    }

    let mut descriptor_buffer = vk::PhysicalDeviceDescriptorBufferFeaturesEXT::default();
    let mut vk12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut vk13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut features = vk::PhysicalDeviceFeatures2::default()
        .push_next(&mut descriptor_buffer)
        .push_next(&mut vk12)
        .push_next(&mut vk13);
    // SAFETY: a read-only query; every struct chained into `features` outlives it.
    unsafe { instance.get_physical_device_features2(physical, &mut features) };
    let core = features.features;
    let missing: Vec<&str> = [
        (descriptor_buffer.descriptor_buffer, "descriptor buffers"),
        (vk12.buffer_device_address, "buffer device addresses"),
        (vk13.dynamic_rendering, "dynamic rendering"),
        (vk13.synchronization2, "synchronization2"),
        (core.sampler_anisotropy, "anisotropic filtering"),
        (core.dual_src_blend, "dual-source blending"),
        (core.texture_compression_bc, "BC texture compression"),
    ]
    .into_iter()
    .filter(|&(supported, _)| supported != vk::TRUE)
    .map(|(_, feature)| feature)
    .collect();
    if !missing.is_empty() {
        return Err(format!("{name} lacks {}", missing.join(", ")));
    }

    let wanted = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE | vk::QueueFlags::TRANSFER;
    let Some((queue_family, family)) = families
        .iter()
        .enumerate()
        .find(|(_, family)| family.queue_flags.contains(wanted))
    else {
        return Err(format!("{name} has no graphics queue"));
    };

    let rank = match properties.device_type {
        vk::PhysicalDeviceType::DISCRETE_GPU => 4,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
        vk::PhysicalDeviceType::CPU => 0,
        _ => 1,
    };
    let timestamps = properties.limits.timestamp_compute_and_graphics == vk::TRUE;
    Ok(Candidate {
        physical,
        rank,
        queue_family: queue_family as u32,
        swapchain: offered(ash::khr::swapchain::NAME),
        timestamp_bits: if timestamps {
            family.timestamp_valid_bits
        } else {
            0
        },
    })
}

/// The validation layer's messages, into the log — warnings as warnings, errors as
/// errors — the way `rhi/sokol_log.rs` routes sokol's own.
unsafe extern "system" fn log_validation(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: the layer hands a valid callback-data struct for the length of the
    // call, whose message, when present, is a NUL-terminated string.
    let message = unsafe {
        data.as_ref()
            .and_then(|data| data.message_as_c_str())
            .map(|text| text.to_string_lossy().into_owned())
    };
    let message = message.unwrap_or_default();
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        log::error!("vulkan: {message}");
    } else {
        log::warn!("vulkan: {message}");
    }
    vk::FALSE
}

// ---------------------------------------------------------------------------
// The swapchain: the window's, or a windowless backbuffer
// ---------------------------------------------------------------------------

/// The window's swapchain and the image it is currently rendering into — or, for a
/// windowless GPU, the offscreen image that stands in for both.
pub(crate) struct Swapchain {
    ctx: Arc<Context>,
    surface: Surface,
    width: u32,
    height: u32,
}

/// What a [`Swapchain`]'s frames land in.
enum Surface {
    /// The window's surface — the viewer's only kind.
    Window(WindowSurface),
    /// An image nothing presents, for the windowless `Gpu` (`review-snapshot`,
    /// `review-preview`). Created on demand, and dropped on resize.
    #[cfg(feature = "headless")]
    Offscreen(Option<Offscreen>),
}

/// A `VkSurfaceKHR` on the window and the `VkSwapchainKHR` on it.
struct WindowSurface {
    surface_fns: ash::khr::surface::Instance,
    swapchain_fns: ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    /// Null until the first build, and after a failed one.
    swapchain: vk::SwapchainKHR,
    extent: vk::Extent2D,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    /// Per swapchain image: what sokol's submit signals and the present waits on.
    render_finished: Vec<vk::Semaphore>,
    /// The acquire ring (see [`ACQUIRE_RING`]).
    acquired: [vk::Semaphore; ACQUIRE_RING],
    next_acquire: usize,
    /// The image acquired for the frame in flight, until it is presented.
    in_flight: Option<u32>,
    /// What the next build asks for: vsync (FIFO) or not.
    vsync: bool,
    /// Rebuild before the next acquire: resized, out of date, suboptimal, a vsync
    /// change, or a frame that was dropped before it presented.
    stale: bool,
}

impl Swapchain {
    /// Put a surface on `window` and a `width`×`height` swapchain on it: vsync-paced
    /// (FIFO), two backbuffers, `B8G8R8A8_UNORM` — the closest equivalent of the
    /// flip-model chain the D3D11 side makes.
    pub(crate) fn new(
        device: &Device,
        window: &Window,
        width: u32,
        height: u32,
    ) -> GpuResult<Self> {
        let ctx = Arc::clone(&device.ctx);
        if !ctx.surfaces.surface || !ctx.swapchain {
            return Err(GpuError::Resource {
                kind: ResourceKind::Swapchain,
                label: "window swapchain".into(),
                detail: "the Vulkan device has no window-system support (VK_KHR_surface, \
                         VK_KHR_swapchain)"
                    .into(),
            });
        }
        let surface = create_surface(&ctx, window)?;
        let surface_fns = ash::khr::surface::Instance::new(&ctx.entry, &ctx.instance);
        // SAFETY: a read-only query about a surface just created on this instance.
        let presents = unsafe {
            surface_fns.get_physical_device_surface_support(ctx.physical, ctx.queue_family, surface)
        }
        .unwrap_or(false);
        if !presents {
            // SAFETY: the surface was created above and nothing else refers to it.
            unsafe { surface_fns.destroy_surface(surface, None) };
            return Err(GpuError::Resource {
                kind: ResourceKind::Swapchain,
                label: "window swapchain".into(),
                detail: "the Vulkan device cannot present to this window".into(),
            });
        }
        let mut acquired = [vk::Semaphore::null(); ACQUIRE_RING];
        for slot in &mut acquired {
            *slot = create_semaphore(&ctx)?;
        }
        let mut window = WindowSurface {
            swapchain_fns: ash::khr::swapchain::Device::new(&ctx.instance, &ctx.device),
            surface_fns,
            surface,
            swapchain: vk::SwapchainKHR::null(),
            extent: vk::Extent2D::default(),
            images: Vec::new(),
            views: Vec::new(),
            render_finished: Vec::new(),
            acquired,
            next_acquire: 0,
            in_flight: None,
            vsync: true,
            stale: true,
        };
        let (width, height) = (width.max(1), height.max(1));
        // Built now, so a device that cannot make the chain fails here, where the
        // caller can report it; a failure frees the surface and the semaphores.
        if let Err(err) = window.rebuild(&ctx, width, height) {
            window.destroy(&ctx);
            return Err(err);
        }
        Ok(Swapchain {
            surface: Surface::Window(window),
            ctx,
            width,
            height,
        })
    }

    /// A `width`×`height` backbuffer with no window behind it: an image of our own in
    /// the swapchain format, which every frame is copied out of for
    /// [`Self::read_back`]. Everything the frame draws reaches it exactly as it would
    /// reach the window's image.
    #[cfg(feature = "headless")]
    pub(crate) fn offscreen(device: &Device, width: u32, height: u32) -> GpuResult<Self> {
        let ctx = Arc::clone(&device.ctx);
        let (width, height) = (width.max(1), height.max(1));
        // Created now rather than on the first frame, so a device that cannot make
        // the target fails here, where the caller can report it.
        let target = Offscreen::new(&ctx, width, height)?;
        Ok(Swapchain {
            ctx,
            surface: Surface::Offscreen(Some(target)),
            width,
            height,
        })
    }

    /// The offscreen backbuffer's pixels as tight `R8G8B8A8` rows, top row first —
    /// what the last frame drew, once [`crate::rhi::Frame::finish`] has submitted it.
    /// The image is `BGRA8` (the swapchain format), so the rows are swizzled on the
    /// way out: `Gpu::read_back` promises RGBA on every OS.
    #[cfg(feature = "headless")]
    pub(crate) fn read_back(&mut self) -> GpuResult<Vec<u8>> {
        let Surface::Offscreen(target) = &mut self.surface else {
            return Err(GpuError::invalid_arg(
                "only an offscreen swapchain can be read back",
            ));
        };
        let target = target
            .as_mut()
            .ok_or_else(|| GpuError::invalid_arg("no frame has been drawn at this size"))?;
        target.read(&self.ctx)
    }

    /// Always `None`: a windowless frame's image is handed to sokol as an ordinary
    /// swapchain image (see the module header), as on D3D11.
    pub(crate) fn pass_attachment(&self) -> Option<sg::View> {
        None
    }

    /// The backbuffer size in physical pixels.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Resize the backbuffers. **Infallible by design**, as on the other two: the
    /// chain is rebuilt at the next acquire, and a zero dimension (a minimized window)
    /// is remembered and the frame skipped.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        match &mut self.surface {
            Surface::Window(window) => window.stale = true,
            // Recreated at the new size on the next acquire.
            #[cfg(feature = "headless")]
            Surface::Offscreen(target) => *target = None,
        }
    }

    /// Nothing to do: the swapchain's images *are* the window's pixels, as on D3D11.
    /// winit tells the compositor the window's scale itself.
    pub(crate) fn set_scale_factor(&mut self, scale: f64) {
        let _ = scale;
    }

    /// Acquire this frame's image and point `sc` at it, reporting whether there is a
    /// frame to draw. `false` (out of date, timed out, minimized) means skip the frame
    /// rather than draw into nothing. `sc`'s size is set to the image's, which is what
    /// the surface decided when the chain was built.
    pub(crate) fn acquire(&mut self, sc: &mut sg::Swapchain) -> bool {
        let ctx = Arc::clone(&self.ctx);
        let (width, height) = (self.width, self.height);
        let (image, view, extent, present_complete, render_finished) = match &mut self.surface {
            Surface::Window(window) => match window.acquire(&ctx, width, height) {
                Some(frame) => frame,
                None => return false,
            },
            #[cfg(feature = "headless")]
            Surface::Offscreen(target) => {
                if target.is_none() {
                    match Offscreen::new(&ctx, width, height) {
                        Ok(made) => *target = Some(made),
                        Err(err) => {
                            log::error!("offscreen backbuffer: {err}");
                            return false;
                        }
                    }
                }
                let Some(target) = target.as_mut() else {
                    return false;
                };
                match target.acquire(&ctx) {
                    Ok(frame) => frame,
                    Err(err) => {
                        log::error!("offscreen backbuffer: {err}");
                        return false;
                    }
                }
            }
        };
        sc.width = extent.width as i32;
        sc.height = extent.height as i32;
        sc.vulkan.render_image = raw(image);
        sc.vulkan.render_view = raw(view);
        sc.vulkan.present_complete_semaphore = raw(present_complete);
        sc.vulkan.render_finished_semaphore = raw(render_finished);
        true
    }

    /// Finish the frame: present the window's image once sokol's submit has rendered
    /// it, or copy the windowless image out for [`Self::read_back`].
    ///
    /// `vsync` picks the present mode — FIFO, or the first of immediate and mailbox the
    /// surface offers — and a change takes effect from the next frame, rebuilding the
    /// chain. The viewer never changes it; the gate harness is what turns it off.
    pub(crate) fn present(&mut self, vsync: bool) -> PresentStatus {
        let ctx = Arc::clone(&self.ctx);
        match &mut self.surface {
            Surface::Window(window) => window.present(&ctx, vsync),
            #[cfg(feature = "headless")]
            Surface::Offscreen(target) => {
                if let Some(target) = target.as_mut()
                    && let Err(err) = target.copy_out(&ctx)
                {
                    log::error!("offscreen backbuffer: {err}");
                }
                PresentStatus::Presented
            }
        }
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        let ctx = Arc::clone(&self.ctx);
        match &mut self.surface {
            Surface::Window(window) => window.destroy(&ctx),
            #[cfg(feature = "headless")]
            Surface::Offscreen(target) => {
                if let Some(target) = target.take() {
                    target.destroy(&ctx);
                }
            }
        }
    }
}

/// A `VkSurfaceKHR` for `window`, through whichever window system it is on.
fn create_surface(ctx: &Context, window: &Window) -> GpuResult<vk::SurfaceKHR> {
    let unsupported = |what: &str| GpuError::Resource {
        kind: ResourceKind::Swapchain,
        label: "window surface".into(),
        detail: format!("{what}: no Vulkan surface extension for it"),
    };
    let display = window
        .display_handle()
        .map_err(|err| GpuError::invalid_arg(format!("the window has no display handle: {err}")))?
        .as_raw();
    let handle = window
        .window_handle()
        .map_err(|err| GpuError::invalid_arg(format!("the window has no handle: {err}")))?
        .as_raw();
    let made = match (display, handle) {
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) => {
            if !ctx.surfaces.wayland {
                return Err(unsupported("a Wayland window"));
            }
            let fns = ash::khr::wayland_surface::Instance::new(&ctx.entry, &ctx.instance);
            let info = vk::WaylandSurfaceCreateInfoKHR::default()
                .display(display.display.as_ptr())
                .surface(handle.surface.as_ptr());
            // SAFETY: winit hands out the live `wl_display` and `wl_surface` of the
            // window it owns, which outlives the swapchain made on this surface.
            unsafe { fns.create_wayland_surface(&info, None) }
        }
        (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(handle)) => {
            if !ctx.surfaces.xlib {
                return Err(unsupported("an X11 (Xlib) window"));
            }
            let Some(dpy) = display.display else {
                return Err(GpuError::invalid_arg("the X11 window has no display"));
            };
            let fns = ash::khr::xlib_surface::Instance::new(&ctx.entry, &ctx.instance);
            let info = vk::XlibSurfaceCreateInfoKHR::default()
                .dpy(dpy.as_ptr().cast())
                .window(handle.window);
            // SAFETY: the live X display and window of the window winit owns.
            unsafe { fns.create_xlib_surface(&info, None) }
        }
        (RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(handle)) => {
            if !ctx.surfaces.xcb {
                return Err(unsupported("an X11 (XCB) window"));
            }
            let Some(connection) = display.connection else {
                return Err(GpuError::invalid_arg("the X11 window has no connection"));
            };
            let fns = ash::khr::xcb_surface::Instance::new(&ctx.entry, &ctx.instance);
            let info = vk::XcbSurfaceCreateInfoKHR::default()
                .connection(connection.as_ptr())
                .window(handle.window.get());
            // SAFETY: the live XCB connection and window of the window winit owns.
            unsafe { fns.create_xcb_surface(&info, None) }
        }
        _ => return Err(unsupported("this window system")),
    };
    made.map_err(vk_error(ResourceKind::Swapchain, "window surface"))
}

fn create_semaphore(ctx: &Context) -> GpuResult<vk::Semaphore> {
    // SAFETY: a plain object creation on a live device.
    unsafe {
        ctx.device
            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
    }
    .map_err(vk_error(ResourceKind::Swapchain, "semaphore"))
}

/// One frame's handles for sokol's swapchain pass: the image and its view, its size,
/// and the semaphore its submit waits on and the one it signals.
type FrameHandles = (
    vk::Image,
    vk::ImageView,
    vk::Extent2D,
    vk::Semaphore,
    vk::Semaphore,
);

impl WindowSurface {
    /// Acquire the next image, rebuilding the chain first when it is stale. `None`
    /// skips the frame.
    fn acquire(&mut self, ctx: &Context, width: u32, height: u32) -> Option<FrameHandles> {
        if width == 0 || height == 0 {
            return None;
        }
        // A frame that acquired and was dropped before it presented — a render error
        // propagated out past `finish` — left an image acquired and a semaphore
        // signaled that nothing will wait on. Neither can be reused; a rebuild
        // replaces both.
        if self.in_flight.take().is_some() {
            self.stale = true;
        }
        if (self.stale || self.swapchain == vk::SwapchainKHR::null())
            && let Err(err) = self.rebuild(ctx, width, height)
        {
            log::warn!("swapchain rebuild: {err}");
            return None;
        }
        let semaphore = self.acquired[self.next_acquire];
        // SAFETY: a live swapchain and an unsignaled semaphore of this device (the
        // ring's oldest; see `ACQUIRE_RING`).
        let acquired = unsafe {
            self.swapchain_fns.acquire_next_image(
                self.swapchain,
                ACQUIRE_TIMEOUT_NS,
                semaphore,
                vk::Fence::null(),
            )
        };
        let index = match acquired {
            Ok((index, suboptimal)) => {
                // Still a usable image: draw it, and rebuild for the next frame.
                self.stale |= suboptimal;
                index
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                return None;
            }
            Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => return None,
            Err(err) => {
                log::warn!("vkAcquireNextImageKHR: {err:?}");
                self.stale = true;
                return None;
            }
        };
        self.next_acquire = (self.next_acquire + 1) % ACQUIRE_RING;
        self.in_flight = Some(index);
        let index = index as usize;
        Some((
            self.images[index],
            self.views[index],
            self.extent,
            semaphore,
            self.render_finished[index],
        ))
    }

    fn present(&mut self, ctx: &Context, vsync: bool) -> PresentStatus {
        if vsync != self.vsync {
            self.vsync = vsync;
            self.stale = true;
        }
        let Some(index) = self.in_flight.take() else {
            return PresentStatus::Presented;
        };
        let waits = [self.render_finished[index as usize]];
        let swapchains = [self.swapchain];
        let indices = [index];
        let info = vk::PresentInfoKHR::default()
            .wait_semaphores(&waits)
            .swapchains(&swapchains)
            .image_indices(&indices);
        // SAFETY: the image was acquired from this swapchain this frame, and its
        // semaphore is the one sokol's submit signaled.
        match unsafe { self.swapchain_fns.queue_present(ctx.queue, &info) } {
            Ok(suboptimal) => self.stale |= suboptimal,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.stale = true,
            Err(vk::Result::ERROR_DEVICE_LOST) => {
                return PresentStatus::DeviceLost {
                    reason: vk::Result::ERROR_DEVICE_LOST.as_raw(),
                };
            }
            Err(err) => {
                log::warn!("vkQueuePresentKHR: {err:?}");
                self.stale = true;
            }
        }
        PresentStatus::Presented
    }

    /// (Re)build the chain at the surface's size — or `width`×`height` where the
    /// surface leaves it to us, as Wayland does — with fresh semaphores.
    ///
    /// The device is drained first, so no image, view or semaphore being replaced is
    /// still in use. A rebuild happens on a resize, a vsync change or an out-of-date
    /// chain, never per frame.
    fn rebuild(&mut self, ctx: &Context, width: u32, height: u32) -> GpuResult<()> {
        let failed = vk_error(ResourceKind::Swapchain, "window swapchain");
        // SAFETY: read-only surface queries, after draining the device so that the
        // objects released below are no longer in use.
        let (caps, formats, modes) = unsafe {
            let _ = ctx.device.device_wait_idle();
            (
                self.surface_fns
                    .get_physical_device_surface_capabilities(ctx.physical, self.surface)
                    .map_err(&failed)?,
                self.surface_fns
                    .get_physical_device_surface_formats(ctx.physical, self.surface)
                    .map_err(&failed)?,
                self.surface_fns
                    .get_physical_device_surface_present_modes(ctx.physical, self.surface)
                    .map_err(&failed)?,
            )
        };
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };
        if extent.width == 0 || extent.height == 0 {
            return Err(GpuError::invalid_arg("the window has no area to draw into"));
        }
        if !formats.iter().any(|format| {
            format.format == VK_SWAPCHAIN_FORMAT
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        }) {
            return Err(GpuError::Resource {
                kind: ResourceKind::Swapchain,
                label: "window swapchain".into(),
                detail: "the surface offers no B8G8R8A8_UNORM format".into(),
            });
        }
        let mode = if self.vsync {
            vk::PresentModeKHR::FIFO
        } else {
            [vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::MAILBOX]
                .into_iter()
                .find(|mode| modes.contains(mode))
                .unwrap_or(vk::PresentModeKHR::FIFO)
        };
        let mut count = caps.min_image_count.max(BACKBUFFERS);
        if caps.max_image_count > 0 {
            count = count.min(caps.max_image_count);
        }
        let alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|alpha| caps.supported_composite_alpha.contains(*alpha))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(count)
            .image_format(VK_SWAPCHAIN_FORMAT)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(mode)
            .clipped(true)
            .old_swapchain(self.swapchain);
        // SAFETY: the surface is live and supported by this queue family (checked in
        // `Swapchain::new`); the old chain, if any, is retired by this call.
        let swapchain =
            unsafe { self.swapchain_fns.create_swapchain(&info, None) }.map_err(&failed)?;
        self.release(ctx);
        self.swapchain = swapchain;
        self.extent = extent;
        // SAFETY: images of the chain just created.
        self.images =
            unsafe { self.swapchain_fns.get_swapchain_images(swapchain) }.map_err(&failed)?;
        for &image in &self.images {
            self.views.push(create_view(ctx, image)?);
            self.render_finished.push(create_semaphore(ctx)?);
        }
        // The acquire ring too: a dropped frame may have left one signaled.
        for slot in &mut self.acquired {
            // SAFETY: the device is idle (above), so no acquire or wait uses it.
            unsafe { ctx.device.destroy_semaphore(*slot, None) };
            *slot = create_semaphore(ctx)?;
        }
        self.next_acquire = 0;
        self.in_flight = None;
        self.stale = false;
        Ok(())
    }

    /// Release the chain's views and semaphores, and the chain itself. The caller has
    /// drained the device.
    fn release(&mut self, ctx: &Context) {
        // SAFETY: the device is idle (every caller waited), so nothing destroyed here
        // is still in use; each handle was made by this device and is dropped once.
        unsafe {
            for view in self.views.drain(..) {
                ctx.device.destroy_image_view(view, None);
            }
            for semaphore in self.render_finished.drain(..) {
                ctx.device.destroy_semaphore(semaphore, None);
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_fns.destroy_swapchain(self.swapchain, None);
                self.swapchain = vk::SwapchainKHR::null();
            }
        }
        self.images.clear();
    }

    fn destroy(&mut self, ctx: &Context) {
        // SAFETY: draining the device first; the surface goes after the chain on it.
        unsafe {
            let _ = ctx.device.device_wait_idle();
        }
        self.release(ctx);
        // SAFETY: as above.
        unsafe {
            for slot in &mut self.acquired {
                ctx.device.destroy_semaphore(*slot, None);
                *slot = vk::Semaphore::null();
            }
            self.surface_fns.destroy_surface(self.surface, None);
        }
    }
}

fn create_view(ctx: &Context, image: vk::Image) -> GpuResult<vk::ImageView> {
    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(VK_SWAPCHAIN_FORMAT)
        .subresource_range(COLOR_RANGE);
    // SAFETY: a view of a live colour image of this device, in its own format.
    unsafe { ctx.device.create_image_view(&info, None) }
        .map_err(vk_error(ResourceKind::Swapchain, "backbuffer view"))
}

/// The one mip and layer of a backbuffer.
const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// The first memory type of `type_bits` that has every one of `wanted`.
#[cfg(feature = "headless")]
fn memory_type(ctx: &Context, type_bits: u32, wanted: vk::MemoryPropertyFlags) -> Option<u32> {
    (0..ctx.memory.memory_type_count).find(|&index| {
        type_bits & (1 << index) != 0
            && ctx.memory.memory_types[index as usize]
                .property_flags
                .contains(wanted)
    })
}

// ---------------------------------------------------------------------------
// The windowless backbuffer
// ---------------------------------------------------------------------------

/// The windowless backbuffer: a colour image handed to sokol as the swapchain image,
/// a host-visible buffer each frame is copied into, and the two semaphores a
/// swapchain pass needs (see the module header).
#[cfg(feature = "headless")]
struct Offscreen {
    width: u32,
    height: u32,
    image: vk::Image,
    image_memory: vk::DeviceMemory,
    view: vk::ImageView,
    staging: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    mapped: *const u8,
    /// The staging memory is host-coherent, so no invalidate is needed before a read.
    coherent: bool,
    pool: vk::CommandPool,
    copy: vk::CommandBuffer,
    /// Signaled when the last copy has landed in `staging`.
    copied: vk::Fence,
    /// A copy was submitted and `copied` not yet waited on.
    copy_pending: bool,
    /// At least one frame was copied out, so `staging` holds a picture.
    has_frame: bool,
    present_complete: vk::Semaphore,
    render_finished: vk::Semaphore,
    /// `acquire` signaled `present_complete` and the copy has not consumed
    /// `render_finished` yet.
    acquired: bool,
}

// SAFETY: `mapped` points into memory this value owns and maps for its whole life;
// it is read only through `&mut self`, so moving the value to another thread (never
// sharing it) moves the only reader with it — what lets a windowless `Gpu` live
// behind a shell's process-wide lock, as on the other two OSes.
#[cfg(feature = "headless")]
unsafe impl Send for Offscreen {}

#[cfg(feature = "headless")]
impl Offscreen {
    fn new(ctx: &Context, width: u32, height: u32) -> GpuResult<Self> {
        let failed = vk_error(ResourceKind::Swapchain, "offscreen backbuffer");
        let mut made = Offscreen {
            width,
            height,
            image: vk::Image::null(),
            image_memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            staging: vk::Buffer::null(),
            staging_memory: vk::DeviceMemory::null(),
            mapped: std::ptr::null(),
            coherent: true,
            pool: vk::CommandPool::null(),
            copy: vk::CommandBuffer::null(),
            copied: vk::Fence::null(),
            copy_pending: false,
            has_frame: false,
            present_complete: vk::Semaphore::null(),
            render_finished: vk::Semaphore::null(),
            acquired: false,
        };
        // Filled in place, so a failure part way frees what was made through
        // `destroy`.
        if let Err(err) = made.fill(ctx, &failed) {
            made.destroy(ctx);
            return Err(err);
        }
        Ok(made)
    }

    fn fill(&mut self, ctx: &Context, failed: &dyn Fn(vk::Result) -> GpuError) -> GpuResult<()> {
        let bytes = u64::from(self.width) * u64::from(self.height) * 4;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(VK_SWAPCHAIN_FORMAT)
            .extent(vk::Extent3D {
                width: self.width,
                height: self.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let buffer_info = vk::BufferCreateInfo::default()
            .size(bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: plain object creation on a live device; each object is stored in
        // `self` the moment it exists, so `destroy` frees it on any later failure, and
        // memory is bound to the object it was sized for.
        unsafe {
            self.image = ctx.device.create_image(&image_info, None).map_err(failed)?;
            let needs = ctx.device.get_image_memory_requirements(self.image);
            let kind = memory_type(
                ctx,
                needs.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )
            .or_else(|| {
                memory_type(
                    ctx,
                    needs.memory_type_bits,
                    vk::MemoryPropertyFlags::empty(),
                )
            })
            .ok_or_else(|| GpuError::invalid_arg("no memory type for the offscreen image"))?;
            self.image_memory = ctx
                .device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
                .map_err(failed)?;
            ctx.device
                .bind_image_memory(self.image, self.image_memory, 0)
                .map_err(failed)?;
            self.view = create_view(ctx, self.image)?;

            self.staging = ctx
                .device
                .create_buffer(&buffer_info, None)
                .map_err(failed)?;
            let needs = ctx.device.get_buffer_memory_requirements(self.staging);
            let visible = vk::MemoryPropertyFlags::HOST_VISIBLE;
            let coherent = visible | vk::MemoryPropertyFlags::HOST_COHERENT;
            let (kind, is_coherent) = match memory_type(ctx, needs.memory_type_bits, coherent) {
                Some(kind) => (kind, true),
                None => (
                    memory_type(ctx, needs.memory_type_bits, visible).ok_or_else(|| {
                        GpuError::invalid_arg("no host-visible memory for the readback buffer")
                    })?,
                    false,
                ),
            };
            self.coherent = is_coherent;
            self.staging_memory = ctx
                .device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
                .map_err(failed)?;
            ctx.device
                .bind_buffer_memory(self.staging, self.staging_memory, 0)
                .map_err(failed)?;
            self.mapped = ctx
                .device
                .map_memory(
                    self.staging_memory,
                    0,
                    vk::WHOLE_SIZE,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(failed)?
                .cast_const()
                .cast();

            self.pool = ctx
                .device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(ctx.queue_family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(failed)?;
            self.copy = ctx
                .device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(failed)?[0];
            self.copied = ctx
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(failed)?;
        }
        self.present_complete = create_semaphore(ctx)?;
        self.render_finished = create_semaphore(ctx)?;
        Ok(())
    }

    /// Wait for the last copy, if one is outstanding, so the command buffer and the
    /// staging bytes are ours again.
    fn wait_copy(&mut self, ctx: &Context) -> GpuResult<()> {
        if !self.copy_pending {
            return Ok(());
        }
        let failed = vk_error(ResourceKind::Swapchain, "offscreen readback");
        // SAFETY: a fence of this device that the last copy's submit signals.
        unsafe {
            ctx.device
                .wait_for_fences(&[self.copied], true, COPY_TIMEOUT_NS)
                .map_err(&failed)?;
            ctx.device.reset_fences(&[self.copied]).map_err(&failed)?;
        }
        self.copy_pending = false;
        self.has_frame = true;
        Ok(())
    }

    /// Hand sokol this frame's image: signal `present_complete` (what an acquire would
    /// do) ahead of the frame's submit, in queue order.
    fn acquire(&mut self, ctx: &Context) -> GpuResult<FrameHandles> {
        self.wait_copy(ctx)?;
        if self.acquired {
            // The last frame was dropped before its copy: one of its semaphores may be
            // signaled with nothing to wait on it. Replace both.
            // SAFETY: draining the device first, so neither is in use.
            unsafe {
                let _ = ctx.device.device_wait_idle();
                ctx.device.destroy_semaphore(self.present_complete, None);
                ctx.device.destroy_semaphore(self.render_finished, None);
            }
            self.present_complete = vk::Semaphore::null();
            self.render_finished = vk::Semaphore::null();
            self.present_complete = create_semaphore(ctx)?;
            self.render_finished = create_semaphore(ctx)?;
        }
        let signal = [self.present_complete];
        let submit = vk::SubmitInfo::default().signal_semaphores(&signal);
        // SAFETY: an empty batch on sokol's queue signaling an unsignaled semaphore;
        // sokol's own submit, later in queue order, waits on it.
        unsafe {
            ctx.device
                .queue_submit(ctx.queue, &[submit], vk::Fence::null())
                .map_err(vk_error(ResourceKind::Swapchain, "offscreen backbuffer"))?;
        }
        self.acquired = true;
        Ok((
            self.image,
            self.view,
            vk::Extent2D {
                width: self.width,
                height: self.height,
            },
            self.present_complete,
            self.render_finished,
        ))
    }

    /// Copy the frame sokol just submitted into the staging buffer, once its submit
    /// has signaled `render_finished`. The CPU does not wait here; [`Self::read`] and
    /// the next acquire do.
    fn copy_out(&mut self, ctx: &Context) -> GpuResult<()> {
        if !self.acquired {
            return Ok(());
        }
        self.wait_copy(ctx)?;
        let failed = vk_error(ResourceKind::Swapchain, "offscreen readback");
        let to_copy = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(COLOR_RANGE);
        let to_host = vk::BufferMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(self.staging)
            .offset(0)
            .size(vk::WHOLE_SIZE);
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: self.width,
                height: self.height,
                depth: 1,
            });
        let waits = [self.render_finished];
        let stages = [vk::PipelineStageFlags::TRANSFER];
        let buffers = [self.copy];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&buffers);
        // SAFETY: the command buffer is idle (`wait_copy`), the image is in the layout
        // sokol's swapchain pass ends in (`PRESENT_SRC_KHR`), and the copy writes the
        // whole of a buffer sized for exactly `width * height * 4` bytes.
        unsafe {
            ctx.device
                .reset_command_buffer(self.copy, vk::CommandBufferResetFlags::empty())
                .map_err(&failed)?;
            ctx.device
                .begin_command_buffer(
                    self.copy,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(&failed)?;
            ctx.device.cmd_pipeline_barrier2(
                self.copy,
                &vk::DependencyInfo::default().image_memory_barriers(&[to_copy]),
            );
            ctx.device.cmd_copy_image_to_buffer(
                self.copy,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.staging,
                &[region],
            );
            ctx.device.cmd_pipeline_barrier2(
                self.copy,
                &vk::DependencyInfo::default().buffer_memory_barriers(&[to_host]),
            );
            ctx.device.end_command_buffer(self.copy).map_err(&failed)?;
            ctx.device
                .queue_submit(ctx.queue, &[submit], self.copied)
                .map_err(&failed)?;
        }
        self.acquired = false;
        self.copy_pending = true;
        Ok(())
    }

    /// The last copied frame as tight RGBA rows.
    fn read(&mut self, ctx: &Context) -> GpuResult<Vec<u8>> {
        self.wait_copy(ctx)?;
        if !self.has_frame {
            return Err(GpuError::invalid_arg(
                "no frame has been drawn at this size",
            ));
        }
        if !self.coherent {
            let range = vk::MappedMemoryRange::default()
                .memory(self.staging_memory)
                .offset(0)
                .size(vk::WHOLE_SIZE);
            // SAFETY: the memory is mapped for this value's whole life.
            unsafe { ctx.device.invalidate_mapped_memory_ranges(&[range]) }
                .map_err(vk_error(ResourceKind::Swapchain, "offscreen readback"))?;
        }
        let len = self.width as usize * self.height as usize * 4;
        // SAFETY: the copy has completed (`wait_copy`), and the mapping covers the
        // whole buffer, which was created `width * height * 4` bytes long.
        let mut rgba = unsafe { std::slice::from_raw_parts(self.mapped, len) }.to_vec();
        for texel in rgba.as_chunks_mut::<4>().0 {
            texel.swap(0, 2);
        }
        Ok(rgba)
    }

    fn destroy(mut self, ctx: &Context) {
        // SAFETY: draining the device first, so nothing destroyed here is in use;
        // every handle was made by this device and the null ones (a failed `new`) are
        // ignored by Vulkan.
        unsafe {
            let _ = ctx.device.device_wait_idle();
            if !self.mapped.is_null() {
                ctx.device.unmap_memory(self.staging_memory);
                self.mapped = std::ptr::null();
            }
            ctx.device.destroy_semaphore(self.present_complete, None);
            ctx.device.destroy_semaphore(self.render_finished, None);
            ctx.device.destroy_fence(self.copied, None);
            ctx.device.destroy_command_pool(self.pool, None);
            ctx.device.destroy_buffer(self.staging, None);
            ctx.device.free_memory(self.staging_memory, None);
            ctx.device.destroy_image_view(self.view, None);
            ctx.device.destroy_image(self.image, None);
            ctx.device.free_memory(self.image_memory, None);
        }
    }
}

// ---------------------------------------------------------------------------
// The GPU-timing leaf (Platform decisions D18)
// ---------------------------------------------------------------------------

/// One frame's resolved GPU timestamps, handed back to [`crate::rhi::gpu_profiler`]
/// to become Tracy zones. The same struct the other leaves fill.
pub(crate) struct FrameTimings {
    /// Bit per timestamp slot that the frame actually recorded.
    pub(crate) written: u32,
    /// Ticks per second.
    pub(crate) frequency: u64,
    /// The raw begin/end tick pairs, in [`crate::rhi::gpu_profiler::Zone`] order.
    pub(crate) times: [u64; TIMESTAMP_SLOTS],
}

/// Ring depth, as on the other leaves: a frame's timestamps have long landed by the
/// time its slot comes round again, so a read never blocks.
const RING: usize = 4;

/// GPU timing on Vulkan: two tiny command buffers per frame on sokol's own queue, one
/// stamping a timestamp before the frame's work and one after it (D18).
///
/// **This measures the frame, not the passes, and for Metal's reason**: timing a pass
/// means writing a timestamp into the command buffer sokol records into, which it does
/// not hand out. What submission order gives instead is the bracket. The closing
/// buffer opens with a barrier on every stage, whose first scope is everything earlier
/// in submission order on the queue — sokol's frame included — so its timestamp lands
/// after the frame's work has finished. RenderDoc or Nsight covers per-pass timing.
pub(crate) struct GpuTimer {
    ctx: Arc<Context>,
    pool: vk::CommandPool,
    queries: vk::QueryPool,
    /// Per ring slot: the opening and closing command buffers, and the fence the
    /// closing one signals.
    begin: [vk::CommandBuffer; RING],
    end: [vk::CommandBuffer; RING],
    done: [vk::Fence; RING],
    /// Whether a slot's buffers were submitted and its fence not yet read.
    used: [bool; RING],
    frame_no: u64,
    cur_slot: usize,
    open: bool,
    frequency: u64,
    mask: u64,
}

impl GpuTimer {
    /// The query pool, the command buffers and their fences. Fails on a queue that
    /// stamps no timestamps, which the caller treats as "no GPU profiling this run".
    pub(crate) fn new(device: &Device) -> GpuResult<Self> {
        let ctx = Arc::clone(&device.ctx);
        if ctx.timestamp_bits == 0 {
            return Err(GpuError::Backend(
                "this Vulkan queue records no timestamps".into(),
            ));
        }
        let failed = vk_error(ResourceKind::Query, "GPU timer");
        let period = f64::from(ctx.properties.limits.timestamp_period);
        let mut timer = GpuTimer {
            pool: vk::CommandPool::null(),
            queries: vk::QueryPool::null(),
            begin: [vk::CommandBuffer::null(); RING],
            end: [vk::CommandBuffer::null(); RING],
            done: [vk::Fence::null(); RING],
            used: [false; RING],
            frame_no: 0,
            cur_slot: 0,
            open: false,
            // `timestampPeriod` is nanoseconds per tick.
            frequency: if period > 0.0 {
                (1.0e9 / period) as u64
            } else {
                1_000_000_000
            },
            mask: if ctx.timestamp_bits >= 64 {
                u64::MAX
            } else {
                (1u64 << ctx.timestamp_bits) - 1
            },
            ctx,
        };
        // SAFETY: plain object creation on a live device; each is stored the moment
        // it exists so `Drop` frees whatever was made if a later step fails.
        unsafe {
            let device = &timer.ctx.device;
            timer.pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(timer.ctx.queue_family)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(&failed)?;
            timer.queries = device
                .create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count((RING * 2) as u32),
                    None,
                )
                .map_err(&failed)?;
            let buffers = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(timer.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count((RING * 2) as u32),
                )
                .map_err(&failed)?;
            timer.begin.copy_from_slice(&buffers[..RING]);
            timer.end.copy_from_slice(&buffers[RING..]);
            for fence in &mut timer.done {
                *fence = device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
                    .map_err(&failed)?;
            }
        }
        Ok(timer)
    }

    /// Open a frame: pick its ring slot, read back whatever older frame that slot still
    /// holds, and submit the opening timestamp. A slot whose older frame has not
    /// finished is skipped for this frame rather than waited on — a profiler that
    /// stalls the frame it measures measures the stall.
    pub(crate) fn begin_frame(&mut self) -> Option<FrameTimings> {
        self.frame_no += 1;
        self.cur_slot = (self.frame_no % RING as u64) as usize;
        let slot = self.cur_slot;
        let (drained, free) = self.read_slot(slot);
        self.open = free && self.record(slot, true).is_ok();
        drained
    }

    /// A no-op: there are no per-pass timestamps to record here (see the type).
    pub(crate) fn timestamp(&mut self, index: usize) {
        let _ = index;
    }

    /// Close the frame with the trailing timestamp. Called after `sg_commit`, so this
    /// buffer is behind the frame's own submit on the queue.
    pub(crate) fn end_frame(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        if self.record(self.cur_slot, false).is_ok() {
            self.used[self.cur_slot] = true;
        }
    }

    /// Record and submit one of a slot's two command buffers: the opening one resets
    /// the slot's two queries and stamps the first; the closing one waits on every
    /// earlier command on the queue, stamps the second and signals the slot's fence.
    fn record(&mut self, slot: usize, opening: bool) -> GpuResult<()> {
        let failed = vk_error(ResourceKind::Query, "GPU timer");
        let device = &self.ctx.device;
        let buffer = if opening {
            self.begin[slot]
        } else {
            self.end[slot]
        };
        let first = (slot * 2) as u32;
        let everything = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);
        let buffers = [buffer];
        let submit = vk::SubmitInfo::default().command_buffers(&buffers);
        let fence = if opening {
            vk::Fence::null()
        } else {
            self.done[slot]
        };
        // SAFETY: the slot's buffers are idle — its fence was read (or it was never
        // used) before the opening buffer is re-recorded, and the closing buffer is
        // recorded once per use after it — and the queries are the slot's own.
        unsafe {
            device
                .reset_command_buffer(buffer, vk::CommandBufferResetFlags::empty())
                .map_err(&failed)?;
            device
                .begin_command_buffer(
                    buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(&failed)?;
            if opening {
                device.cmd_reset_query_pool(buffer, self.queries, first, 2);
                device.cmd_write_timestamp2(
                    buffer,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    self.queries,
                    first,
                );
            } else {
                device.cmd_pipeline_barrier2(
                    buffer,
                    &vk::DependencyInfo::default().memory_barriers(&[everything]),
                );
                device.cmd_write_timestamp2(
                    buffer,
                    vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                    self.queries,
                    first + 1,
                );
            }
            device.end_command_buffer(buffer).map_err(&failed)?;
            device
                .queue_submit(self.ctx.queue, &[submit], fence)
                .map_err(&failed)?;
        }
        Ok(())
    }

    /// Read one slot's pair back, if the frame it belongs to has finished on the GPU,
    /// and say whether the slot is free to record into.
    fn read_slot(&mut self, slot: usize) -> (Option<FrameTimings>, bool) {
        if !self.used[slot] {
            return (None, true);
        }
        let device = &self.ctx.device;
        // SAFETY: a fence of this device; a status query does not wait.
        let finished = unsafe { device.get_fence_status(self.done[slot]) }.unwrap_or(false);
        if !finished {
            return (None, false);
        }
        let mut ticks = [0u64; 2];
        // SAFETY: the slot's two queries, both written by buffers whose fence has
        // signaled, read without waiting into a buffer of exactly two values.
        let read = unsafe {
            device.get_query_pool_results(
                self.queries,
                (slot * 2) as u32,
                &mut ticks,
                vk::QueryResultFlags::TYPE_64,
            )
        };
        // SAFETY: as above; the fence is reset only once its frame is done with it.
        let _ = unsafe { device.reset_fences(&[self.done[slot]]) };
        self.used[slot] = false;
        if read.is_err() {
            return (None, true);
        }
        let mut times = [0u64; TIMESTAMP_SLOTS];
        let base = Zone::Frame.base();
        times[base] = ticks[0] & self.mask;
        times[base + 1] = ticks[1] & self.mask;
        (
            Some(FrameTimings {
                written: Zone::Frame.slot_bits(),
                frequency: self.frequency,
                times,
            }),
            true,
        )
    }
}

impl Drop for GpuTimer {
    fn drop(&mut self) {
        // SAFETY: draining the device first; each handle was made by this device and
        // the null ones (a failed `new`) are ignored.
        unsafe {
            let device = &self.ctx.device;
            let _ = device.device_wait_idle();
            for fence in self.done {
                device.destroy_fence(fence, None);
            }
            device.destroy_query_pool(self.queries, None);
            device.destroy_command_pool(self.pool, None);
        }
    }
}

// ---------------------------------------------------------------------------
// The readback leaf, for the offline bake (Platform decisions D19)
// ---------------------------------------------------------------------------

/// Copy one subresource of a sokol image back to the CPU — **not on Vulkan**.
///
/// The other leaves ask sokol for the native texture behind an image
/// (`sg_d3d11_query_image_info`, `sg_mtl_query_image_info`), and this sokol has no
/// Vulkan twin of either, so there is no `VkImage` to copy from. The bake that calls
/// this runs on Windows (D19), as it already did rather than on the Mac; the windowless
/// `Gpu` does not need it here, because its backbuffer is an image of our own.
#[cfg(feature = "bake")]
pub(crate) fn read_image_subresource(
    image: sg::Image,
    mip: u32,
    slice: u32,
    width: u32,
    height: u32,
    format: Format,
    bpp: u32,
) -> GpuResult<Vec<u8>> {
    let _ = (image, mip, slice, width, height, format, bpp);
    Err(GpuError::Backend(
        "reading a sokol image back is not available on Vulkan, whose sokol_gfx names no \
         native image; run the IBL bake on Windows (docs/ARCHITECTURE.md, D19)"
            .into(),
    ))
}
