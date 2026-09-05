//! GPU buffer plumbing: an immutable vertex buffer (rebuilt wholesale on change)
//! and a dynamic constant buffer updated each frame via `Map(WRITE_DISCARD)`.

use bytemuck::Pod;
use windows::Win32::Graphics::Direct3D::D3D_SRV_DIMENSION_BUFFER;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_FLAG, D3D11_BIND_INDEX_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VERTEX_BUFFER, D3D11_BUFFER_DESC, D3D11_BUFFER_SRV,
    D3D11_BUFFER_SRV_0, D3D11_BUFFER_SRV_1, D3D11_CPU_ACCESS_FLAG, D3D11_CPU_ACCESS_WRITE,
    D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_SUBRESOURCE_DATA,
    D3D11_USAGE, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, ID3D11Buffer,
    ID3D11ShaderResourceView,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32_UINT, DXGI_FORMAT_UNKNOWN};

use super::{Gpu, GpuError, GpuResult, ResourceContext, ResourceKind, out_param};

/// Build a `D3D11_BUFFER_DESC` with this module's common defaults (no misc flags,
/// no structured stride). The four arguments are the only fields that vary across
/// the vertex / index / constant buffers below.
fn buffer_desc(
    size: u32,
    usage: D3D11_USAGE,
    bind: D3D11_BIND_FLAG,
    cpu_access: D3D11_CPU_ACCESS_FLAG,
) -> D3D11_BUFFER_DESC {
    D3D11_BUFFER_DESC {
        ByteWidth: size,
        Usage: usage,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: cpu_access.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    }
}

/// An immutable vertex buffer + its stride and vertex count. Geometry is rebuilt
/// wholesale on change, so immutable storage with
/// initial data is the natural fit.
pub(crate) struct VertexBuffer {
    buffer: ID3D11Buffer,
    stride: u32,
    count: u32,
}

impl VertexBuffer {
    /// Create an immutable vertex buffer from `data`. `data` must be non-empty
    /// (D3D11 rejects a zero-byte buffer); callers that may have no geometry skip
    /// the draw rather than building an empty buffer.
    pub(crate) fn new<T: Pod>(gpu: &Gpu, data: &[T]) -> GpuResult<Self> {
        let device = gpu.device();
        // A real error, not a debug_assert: in release an empty slice would reach
        // `CreateBuffer` with `ByteWidth: 0` and fail as an opaque E_INVALIDARG.
        if data.is_empty() {
            return Err(GpuError::invalid_arg("vertex buffer must be non-empty"));
        }
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let desc = buffer_desc(
            std::mem::size_of_val(bytes) as u32,
            D3D11_USAGE_IMMUTABLE,
            D3D11_BIND_VERTEX_BUFFER,
            D3D11_CPU_ACCESS_FLAG(0),
        );
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = None;
        // SAFETY: `desc` matches `init` (immutable buffer with full initial data);
        // `init.pSysMem` points at `bytes`, alive for the call. The out-param is set.
        unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buffer)) }
            .resource(ResourceKind::Buffer, "vertex buffer")?;
        Ok(Self {
            buffer: out_param(buffer),
            stride: std::mem::size_of::<T>() as u32,
            count: data.len() as u32,
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Bind this buffer to input slot 0.
    pub(crate) fn bind(&self, gpu: &Gpu) {
        // SAFETY: the buffer is live; the stride/offset locals outlive the call.
        unsafe {
            gpu.context().IASetVertexBuffers(
                0,
                1,
                Some(&Some(self.buffer.clone())),
                Some(&self.stride),
                Some(&0),
            );
        }
    }
}

/// An immutable 32-bit index buffer + its index count. Like [`VertexBuffer`],
/// rebuilt wholesale when the mesh changes.
pub(crate) struct IndexBuffer {
    buffer: ID3D11Buffer,
    count: u32,
}

impl IndexBuffer {
    /// Create an immutable index buffer from `indices` (must be non-empty).
    pub(crate) fn new(gpu: &Gpu, indices: &[u32]) -> GpuResult<Self> {
        let device = gpu.device();
        // See `VertexBuffer::new` — checked in release too.
        if indices.is_empty() {
            return Err(GpuError::invalid_arg("index buffer must be non-empty"));
        }
        let bytes: &[u8] = bytemuck::cast_slice(indices);
        let desc = buffer_desc(
            std::mem::size_of_val(bytes) as u32,
            D3D11_USAGE_IMMUTABLE,
            D3D11_BIND_INDEX_BUFFER,
            D3D11_CPU_ACCESS_FLAG(0),
        );
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = None;
        // SAFETY: immutable buffer with full initial data; `init.pSysMem` points at
        // `bytes`, alive for the call. The out-param is set.
        unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buffer)) }
            .resource(ResourceKind::Buffer, "index buffer")?;
        Ok(Self {
            buffer: out_param(buffer),
            count: indices.len() as u32,
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Bind this index buffer (32-bit indices) to the input assembler.
    pub(crate) fn bind(&self, gpu: &Gpu) {
        // SAFETY: the buffer is live for the duration of the call.
        unsafe {
            gpu.context()
                .IASetIndexBuffer(&self.buffer, DXGI_FORMAT_R32_UINT, 0);
        }
    }
}

/// A `USAGE_DYNAMIC` constant buffer updated each frame with `Map(WRITE_DISCARD)`.
/// The byte size is rounded up to a 16-byte multiple (the cbuffer requirement).
pub(crate) struct DynamicConstantBuffer {
    buffer: ID3D11Buffer,
    /// The rounded-up byte size the buffer was created with; [`Self::update`]
    /// bounds its copy against it so a mismatched `T` can never scribble past the
    /// mapped region.
    size: u32,
}

impl DynamicConstantBuffer {
    /// Create a dynamic constant buffer sized for `T` (rounded up to 16 bytes).
    pub(crate) fn new<T>(gpu: &Gpu) -> GpuResult<Self> {
        let device = gpu.device();
        let size = std::mem::size_of::<T>().next_multiple_of(16) as u32;
        let desc = buffer_desc(
            size,
            D3D11_USAGE_DYNAMIC,
            D3D11_BIND_CONSTANT_BUFFER,
            D3D11_CPU_ACCESS_WRITE,
        );
        let mut buffer = None;
        // SAFETY: a dynamic cbuffer with no initial data; the out-param is set.
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }
            .resource(ResourceKind::Buffer, "constant buffer")?;
        Ok(Self {
            buffer: out_param(buffer),
            size,
        })
    }

    /// Upload `value` into the buffer (discard-and-rewrite). `T` must fit the
    /// buffer's rounded-up size — i.e. be the `T` passed to [`Self::new`] — and a
    /// mismatch is a checked error, never an out-of-bounds GPU write.
    pub(crate) fn update<T: Pod>(&self, gpu: &Gpu, value: &T) -> GpuResult<()> {
        let ctx = gpu.context();
        let bytes = bytemuck::bytes_of(value);
        if bytes.len() > self.size as usize {
            return Err(GpuError::invalid_arg(
                "constant-buffer update is larger than the buffer it was created for",
            ));
        }
        // SAFETY: WRITE_DISCARD maps the whole dynamic buffer for CPU writes; the
        // mapped region is `self.size` bytes and `bytes.len() <= self.size` was
        // just checked, so the copy stays in bounds. `Unmap` pairs with the `Map`.
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(
                &self.buffer,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, bytes.len());
            ctx.Unmap(&self.buffer, 0);
        }
        Ok(())
    }

    /// Bind this buffer to the vertex-shader constant slot `slot`.
    pub(crate) fn bind_vs(&self, gpu: &Gpu, slot: u32) {
        // SAFETY: the buffer is live; the one-element array outlives the call.
        unsafe {
            gpu.context()
                .VSSetConstantBuffers(slot, Some(&[Some(self.buffer.clone())]));
        }
    }

    /// Bind this buffer to the pixel-shader constant slot `slot`.
    pub(crate) fn bind_ps(&self, gpu: &Gpu, slot: u32) {
        // SAFETY: the buffer is live; the one-element array outlives the call.
        unsafe {
            gpu.context()
                .PSSetConstantBuffers(slot, Some(&[Some(self.buffer.clone())]));
        }
    }
}

/// A structured buffer read by the vertex shader through a shader-resource view
/// — the deform path's influence runs, palette and blend-shape deltas. Either
/// immutable (built once with the mesh) or dynamic (re-uploaded when the pose
/// changes, via `Map(WRITE_DISCARD)` like the constant buffers). Core at feature
/// level 11_0, the renderer's floor, so it needs no capability gate.
pub(crate) struct StructuredBuffer<T> {
    buffer: ID3D11Buffer,
    srv: ID3D11ShaderResourceView,
    /// Element capacity the buffer was created with; [`Self::update`] bounds its
    /// copy against it.
    capacity: u32,
    _element: std::marker::PhantomData<T>,
}

impl<T: Pod> StructuredBuffer<T> {
    /// An immutable structured buffer holding `data` (must be non-empty).
    pub(crate) fn immutable(gpu: &Gpu, data: &[T]) -> GpuResult<Self> {
        if data.is_empty() {
            return Err(GpuError::invalid_arg("structured buffer must be non-empty"));
        }
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        Self::create(
            gpu,
            data.len(),
            D3D11_USAGE_IMMUTABLE,
            D3D11_CPU_ACCESS_FLAG(0),
            Some(&init),
        )
    }

    /// A dynamic structured buffer with room for `capacity` elements (must be
    /// non-zero), filled by [`Self::update`].
    pub(crate) fn dynamic(gpu: &Gpu, capacity: usize) -> GpuResult<Self> {
        if capacity == 0 {
            return Err(GpuError::invalid_arg("structured buffer must be non-empty"));
        }
        Self::create(
            gpu,
            capacity,
            D3D11_USAGE_DYNAMIC,
            D3D11_CPU_ACCESS_WRITE,
            None,
        )
    }

    fn create(
        gpu: &Gpu,
        capacity: usize,
        usage: D3D11_USAGE,
        cpu_access: D3D11_CPU_ACCESS_FLAG,
        init: Option<&D3D11_SUBRESOURCE_DATA>,
    ) -> GpuResult<Self> {
        let device = gpu.device();
        let stride = std::mem::size_of::<T>();
        // Structured-buffer strides must be a multiple of 4 and the element count
        // must fit the view's 32-bit range; both are checked rather than assumed.
        if stride == 0 || !stride.is_multiple_of(4) {
            return Err(GpuError::invalid_arg(
                "structured buffer element size must be a non-zero multiple of 4",
            ));
        }
        let capacity_u32 = u32::try_from(capacity)
            .map_err(|_| GpuError::invalid_arg("structured buffer has too many elements"))?;
        let byte_width = stride
            .checked_mul(capacity)
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or_else(|| GpuError::invalid_arg("structured buffer is too large"))?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: byte_width,
            Usage: usage,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: cpu_access.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: stride as u32,
        };
        let mut buffer = None;
        // SAFETY: `desc` describes a structured buffer of `capacity` elements; when
        // `init` is given it points at exactly that many elements, alive for the
        // call. The out-param is set on success.
        unsafe {
            device.CreateBuffer(
                &desc,
                init.map(|init| init as *const D3D11_SUBRESOURCE_DATA),
                Some(&mut buffer),
            )
        }
        .resource(ResourceKind::Buffer, "structured buffer")?;
        let buffer = out_param(buffer);

        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_UNKNOWN,
            ViewDimension: D3D_SRV_DIMENSION_BUFFER,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                Buffer: D3D11_BUFFER_SRV {
                    Anonymous1: D3D11_BUFFER_SRV_0 { FirstElement: 0 },
                    Anonymous2: D3D11_BUFFER_SRV_1 {
                        NumElements: capacity_u32,
                    },
                },
            },
        };
        let mut srv = None;
        // SAFETY: `buffer` is shader-resource-bindable and structured; the view
        // covers exactly its `capacity` elements. Out-param set on success.
        unsafe { device.CreateShaderResourceView(&buffer, Some(&srv_desc), Some(&mut srv)) }
            .resource(ResourceKind::Buffer, "structured-buffer view")?;
        Ok(Self {
            buffer,
            srv: out_param(srv),
            capacity: capacity_u32,
            _element: std::marker::PhantomData,
        })
    }

    /// Element capacity.
    pub(crate) fn capacity(&self) -> usize {
        self.capacity as usize
    }

    /// Upload `data` into a dynamic buffer (discard-and-rewrite). `data` must fit
    /// the capacity the buffer was created with — a checked error, never an
    /// out-of-bounds GPU write. Elements past `data.len()` are left undefined,
    /// so callers must not index them.
    pub(crate) fn update(&self, gpu: &Gpu, data: &[T]) -> GpuResult<()> {
        let ctx = gpu.context();
        let bytes: &[u8] = bytemuck::cast_slice(data);
        if data.len() > self.capacity as usize {
            return Err(GpuError::invalid_arg(
                "structured-buffer update is larger than the buffer it was created for",
            ));
        }
        // SAFETY: WRITE_DISCARD maps the whole dynamic buffer for CPU writes; the
        // mapped region is `capacity * stride` bytes and `data.len() <= capacity`
        // was just checked, so the copy stays in bounds. `Unmap` pairs with `Map`.
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(
                &self.buffer,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, bytes.len());
            ctx.Unmap(&self.buffer, 0);
        }
        Ok(())
    }

    /// Bind this buffer's view to vertex-shader resource slot `slot`.
    pub(crate) fn bind_vs(&self, gpu: &Gpu, slot: u32) {
        // SAFETY: the SRV is live; the one-element array outlives the call.
        unsafe {
            gpu.context()
                .VSSetShaderResources(slot, Some(&[Some(self.srv.clone())]));
        }
    }
}
