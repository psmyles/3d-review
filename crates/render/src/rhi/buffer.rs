//! GPU buffer plumbing: an immutable vertex buffer (rebuilt wholesale on change,
//! mirroring the wgpu path) and a dynamic constant buffer updated each frame via
//! `Map(WRITE_DISCARD)` (the equivalent of wgpu's `queue.write_buffer`).

use bytemuck::Pod;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_VERTEX_BUFFER,
    D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_WRITE, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, ID3D11Buffer, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R32_UINT;
use windows::core::Result;

/// An immutable vertex buffer + its stride and vertex count. Geometry is rebuilt
/// wholesale on change (the wgpu path did the same), so immutable storage with
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
    pub(crate) fn new<T: Pod>(device: &ID3D11Device, data: &[T]) -> Result<Self> {
        debug_assert!(!data.is_empty(), "vertex buffer must be non-empty");
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of_val(bytes) as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = None;
        // SAFETY: `desc` matches `init` (immutable buffer with full initial data);
        // `init.pSysMem` points at `bytes`, alive for the call. The out-param is set.
        unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buffer))? };
        Ok(Self {
            buffer: buffer.unwrap(),
            stride: std::mem::size_of::<T>() as u32,
            count: data.len() as u32,
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Bind this buffer to input slot 0.
    pub(crate) fn bind(&self, ctx: &ID3D11DeviceContext) {
        // SAFETY: the buffer is live; the stride/offset locals outlive the call.
        unsafe {
            ctx.IASetVertexBuffers(
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
    pub(crate) fn new(device: &ID3D11Device, indices: &[u32]) -> Result<Self> {
        debug_assert!(!indices.is_empty(), "index buffer must be non-empty");
        let bytes: &[u8] = bytemuck::cast_slice(indices);
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of_val(bytes) as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_INDEX_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = None;
        // SAFETY: immutable buffer with full initial data; `init.pSysMem` points at
        // `bytes`, alive for the call. The out-param is set.
        unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buffer))? };
        Ok(Self {
            buffer: buffer.unwrap(),
            count: indices.len() as u32,
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Bind this index buffer (32-bit indices) to the input assembler.
    pub(crate) fn bind(&self, ctx: &ID3D11DeviceContext) {
        // SAFETY: the buffer is live for the duration of the call.
        unsafe {
            ctx.IASetIndexBuffer(&self.buffer, DXGI_FORMAT_R32_UINT, 0);
        }
    }
}

/// A `USAGE_DYNAMIC` constant buffer updated each frame with `Map(WRITE_DISCARD)`.
/// The byte size is rounded up to a 16-byte multiple (the cbuffer requirement).
pub(crate) struct DynamicConstantBuffer {
    buffer: ID3D11Buffer,
}

impl DynamicConstantBuffer {
    /// Create a dynamic constant buffer sized for `T` (rounded up to 16 bytes).
    pub(crate) fn new<T>(device: &ID3D11Device) -> Result<Self> {
        let size = std::mem::size_of::<T>().next_multiple_of(16) as u32;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: size,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut buffer = None;
        // SAFETY: a dynamic cbuffer with no initial data; the out-param is set.
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer))? };
        Ok(Self {
            buffer: buffer.unwrap(),
        })
    }

    /// Upload `value` into the buffer (discard-and-rewrite). `T` must be no larger
    /// than the buffer's rounded-up size, which holds when the same `T` was used in
    /// [`Self::new`].
    pub(crate) fn update<T: Pod>(&self, ctx: &ID3D11DeviceContext, value: &T) -> Result<()> {
        let bytes = bytemuck::bytes_of(value);
        // SAFETY: WRITE_DISCARD maps the whole dynamic buffer for CPU writes; the
        // mapped region is at least `size_of::<T>()` bytes (the buffer was sized for
        // `T`), so the copy stays in bounds. `Unmap` is paired with the `Map`.
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
    pub(crate) fn bind_vs(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the buffer is live; the one-element array outlives the call.
        unsafe {
            ctx.VSSetConstantBuffers(slot, Some(&[Some(self.buffer.clone())]));
        }
    }

    /// Bind this buffer to the pixel-shader constant slot `slot`.
    pub(crate) fn bind_ps(&self, ctx: &ID3D11DeviceContext, slot: u32) {
        // SAFETY: the buffer is live; the one-element array outlives the call.
        unsafe {
            ctx.PSSetConstantBuffers(slot, Some(&[Some(self.buffer.clone())]));
        }
    }
}
