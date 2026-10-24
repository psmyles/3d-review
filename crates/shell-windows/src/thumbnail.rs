//! The thumbnail provider: Explorer hands it a file as a stream and asks for a
//! square bitmap no larger than a size it names.
//!
//! **Stream-initialized, so it runs isolated.** Explorer loads a handler that
//! takes `IInitializeWithStream` into its COM Surrogate rather than into itself,
//! which is what keeps a GPU device and a C parser out of Explorer's own process.
//! The cost is that the file arrives as bytes with no directory, so the only
//! textures a thumbnail can find are the ones the file embeds and the ones it
//! names by an absolute path that still exists. The preview pane, which gets
//! the path, finds the rest. Measured, so as not to be tried again:
//!
//! * the stream's own name (`IStream::Stat`) is the bare file name, and the
//!   stream answers `E_NOINTERFACE` to every interface that might name the file
//!   (`IPersistFile`, `IPersist`, `IServiceProvider`, `IObjectWithSite`,
//!   `IShellItem`, `IPropertyStore`);
//! * offered `IInitializeWithItem` alongside the stream, the surrogate takes the
//!   stream; offered the item alone, it fails the extraction;
//! * `DisableProcessIsolation` does pass an item — by loading the handler into
//!   the *calling* process, which for every folder view is `explorer.exe`, and
//!   giving the class an AppID with a surrogate does not move it back out.
//!   A GPU device and a C parser inside Explorer is exactly what the isolation
//!   is there to prevent, so this is not the trade to make.

#![allow(
    unsafe_code,
    reason = "invariant 9: reading the IStream Explorer hands us and building the DIB section it takes back"
)]

use std::path::PathBuf;
use std::sync::Mutex;

use review_preview::{PreviewScene, SceneSource, render_thumbnail};
use windows::Win32::Foundation::{E_FAIL, E_POINTER, E_UNEXPECTED, S_OK};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows::Win32::System::Com::{CoTaskMemFree, IStream, STATFLAG_DEFAULT, STATSTG};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IInitializeWithStream, IInitializeWithStream_Impl,
};
use windows::Win32::UI::Shell::{
    IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_ARGB,
};
use windows::core::{Error, IUnknown, Ref, Result, implement};

use crate::guard::guarded;
use crate::host::with_host;

/// The largest file read into memory for a thumbnail. Past it Explorer shows
/// the ordinary icon: a file that large is minutes of parsing for a picture a
/// few dozen pixels across.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// The largest thumbnail drawn. Explorer asks for up to 2560 pixels for its
/// largest views, which is a lot of supersampled GPU work for a tile shown at a
/// fraction of that; it scales a smaller bitmap up itself.
const MAX_SIZE: u32 = 1024;

/// What `Initialize` read, for `GetThumbnail` to draw.
struct File {
    bytes: Vec<u8>,
    /// The stream's own name: the full path for an ordinary file, or only a
    /// file name.
    name: PathBuf,
}

#[implement(IThumbnailProvider, IInitializeWithStream)]
struct ThumbnailProvider {
    file: Mutex<Option<File>>,
}

pub(crate) fn create() -> IUnknown {
    ThumbnailProvider {
        file: Mutex::new(None),
    }
    .into()
}

impl IInitializeWithStream_Impl for ThumbnailProvider_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        guarded("thumbnail Initialize", || {
            let stream = stream.ok()?;
            let (size, name) = stat(stream)?;
            log::debug!("thumbnail stream: {} ({size} bytes)", name.display());
            if size > MAX_FILE_BYTES {
                log::info!(
                    "thumbnail skipped: {} is {size} bytes, past the {MAX_FILE_BYTES} limit",
                    name.display()
                );
                return Err(Error::from_hresult(E_FAIL));
            }
            let bytes = read_all(stream, size)?;
            *self
                .file
                .lock()
                .map_err(|_| Error::from_hresult(E_UNEXPECTED))? = Some(File { bytes, name });
            Ok(())
        })
    }
}

impl IThumbnailProvider_Impl for ThumbnailProvider_Impl {
    fn GetThumbnail(&self, cx: u32, bitmap: *mut HBITMAP, alpha: *mut WTS_ALPHATYPE) -> Result<()> {
        guarded("GetThumbnail", || {
            if bitmap.is_null() || alpha.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            let file = self
                .file
                .lock()
                .map_err(|_| Error::from_hresult(E_UNEXPECTED))?
                .take()
                .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
            let size = cx.clamp(1, MAX_SIZE);
            let started = std::time::Instant::now();
            let scene = PreviewScene::load(
                SceneSource::Bytes {
                    bytes: &file.bytes,
                    name: &file.name,
                },
                // Twice the tile, because it is drawn supersampled.
                Some((size * 4).clamp(512, 2048)),
            )
            .map_err(|error| {
                log::info!("no thumbnail for {}: {error}", file.name.display());
                Error::from_hresult(E_FAIL)
            })?;
            drop(file);
            let image =
                with_host(|host| render_thumbnail(host, &scene, size))?.map_err(|error| {
                    log::error!("thumbnail of {} did not draw: {error}", scene.name());
                    Error::from_hresult(E_FAIL)
                })?;
            let made = dib_section(image.width, image.height, &image.to_bgra())?;
            log::debug!(
                "thumbnail of {} at {size}px in {:.0} ms",
                scene.name(),
                started.elapsed().as_secs_f64() * 1000.0
            );
            // SAFETY: both out-pointers were checked non-null above and are, per
            // the interface's contract, valid for one write each.
            unsafe {
                *bitmap = made;
                *alpha = WTSAT_ARGB;
            }
            Ok(())
        })
    }
}

/// The stream's length and name.
fn stat(stream: &IStream) -> Result<(u64, PathBuf)> {
    let mut stat = STATSTG::default();
    // SAFETY: `stat` is a live, writable STATSTG; with STATFLAG_DEFAULT the
    // stream allocates `pwcsName` with the COM allocator, freed just below.
    unsafe { stream.Stat(&mut stat, STATFLAG_DEFAULT) }?;
    let name = if stat.pwcsName.is_null() {
        PathBuf::new()
    } else {
        // SAFETY: a non-null `pwcsName` from `Stat` is a NUL-terminated wide
        // string, read once here and then released with the allocator that
        // made it.
        let name = unsafe { stat.pwcsName.to_string() }.unwrap_or_default();
        // SAFETY: as above; nothing reads `pwcsName` after this.
        unsafe { CoTaskMemFree(Some(stat.pwcsName.0.cast_const().cast())) };
        PathBuf::from(name)
    };
    Ok((stat.cbSize, name))
}

/// Every byte of `stream`, which `Stat` said is `size` long.
fn read_all(stream: &IStream, size: u64) -> Result<Vec<u8>> {
    const CHUNK: usize = 1 << 20;
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut read = 0u32;
        // SAFETY: `chunk` is a live buffer of `CHUNK` bytes, and the stream
        // writes at most that many into it, reporting the count in `read`.
        let result =
            unsafe { stream.Read(chunk.as_mut_ptr().cast(), CHUNK as u32, Some(&mut read)) };
        result.ok()?;
        let read = (read as usize).min(CHUNK);
        bytes.extend_from_slice(&chunk[..read]);
        if read == 0 || result != S_OK || bytes.len() as u64 > MAX_FILE_BYTES {
            break;
        }
    }
    Ok(bytes)
}

/// A top-down 32-bit DIB section holding `bgra`.
fn dib_section(width: u32, height: u32, bgra: &[u8]) -> Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative: the rows run top to bottom, as the image's do.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: `info` describes a 32-bit top-down bitmap; `bits` receives a
    // pointer to its `width × height × 4` bytes, owned by the bitmap.
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    let len = width as usize * height as usize * 4;
    if bits.is_null() || bgra.len() != len {
        // SAFETY: the section was made just above and is not handed out.
        let _ = unsafe { DeleteObject(bitmap.into()) };
        return Err(Error::from_hresult(E_UNEXPECTED));
    }
    // SAFETY: `bits` is the section's own buffer of exactly `len` bytes (a
    // 32-bit row needs no padding), and `bgra` is `len` bytes long.
    unsafe { std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), len) };
    Ok(bitmap)
}
