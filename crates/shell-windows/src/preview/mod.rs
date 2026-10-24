//! The preview handler: what Explorer's preview pane (Alt+P) asks to show an
//! `.fbx` in.
//!
//! Explorer gives the handler a file, a parent window and a rectangle, and the
//! handler shows the file there — here, as the viewer's own render, which the
//! pointer orbits, pans and zooms ([`review_preview::Interaction`]). It runs in
//! a `prevhost.exe` of our own AppID, so a fault in it cannot take a preview of
//! anything else down with it.
//!
//! It is initialized with the file's *path* (`IInitializeWithFile`, or
//! `IInitializeWithItem` for a host that offers only that), unlike the
//! stream-only thumbnail: preview handlers always run out of Explorer's process,
//! and the path is what lets textures beside the model be found.
//!
//! The COM object is a record of what Explorer has said — the file, the parent,
//! the rectangle, the colors. The window and the pane behind it live on the
//! preview window thread ([`ui`]): every method that touches the window posts
//! the change there and returns, and the thread applies them in order.
//!
//! Keystrokes reach that window's own message loop, not the host's, so
//! `TranslateAccelerator` sees none of them and answers that it handled
//! nothing. While the pane has the keyboard (after a click in it) Explorer's own
//! shortcuts do not reach Explorer; a click back in the file list returns them.

#![allow(
    unsafe_code,
    reason = "invariant 9: the preview handler's COM surface - raw rectangles, messages and out-pointers"
)]

mod pane;
mod ui;
mod window;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use windows::Win32::Foundation::{
    COLORREF, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, HWND, RECT, S_FALSE,
};
use windows::Win32::Graphics::Gdi::LOGFONTW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Ole::{
    IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithFile_Impl};
use windows::Win32::UI::Shell::{
    IInitializeWithItem, IInitializeWithItem_Impl, IPreviewHandler, IPreviewHandler_Impl,
    IPreviewHandlerVisuals, IPreviewHandlerVisuals_Impl, IShellItem, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::MSG;
use windows::core::{BOOL, Error, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};

use crate::guard::guarded;

/// The pane's colors before Explorer says otherwise: its light theme's.
const DEFAULT_BACKGROUND: [u8; 3] = [255, 255, 255];
const DEFAULT_TEXT: [u8; 3] = [0, 0, 0];

/// A window handle as a number: what crosses to the window thread, since an
/// `HWND` is not `Send`.
type Handle = isize;

fn hwnd(handle: Handle) -> HWND {
    HWND(handle as *mut c_void)
}

/// The pane's window, once the window thread has made it.
#[derive(Default)]
struct Pane(Mutex<Option<Handle>>);

impl Pane {
    fn slot(&self) -> MutexGuard<'_, Option<Handle>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self) -> Option<HWND> {
        self.slot().map(hwnd)
    }
}

#[derive(Default)]
struct State {
    path: Option<PathBuf>,
    parent: Option<Handle>,
    rect: RECT,
    pane: Option<Arc<Pane>>,
    site: Option<IUnknown>,
    background: Option<[u8; 3]>,
    text: Option<[u8; 3]>,
}

impl State {
    fn colors(&self) -> ([u8; 3], [u8; 3]) {
        (
            self.background.unwrap_or(DEFAULT_BACKGROUND),
            self.text.unwrap_or(DEFAULT_TEXT),
        )
    }
}

/// A `Mutex` rather than a `RefCell`: in the neutral apartment `prevhost.exe`
/// puts the handler in, its calls arrive on any thread.
#[implement(
    IPreviewHandler,
    IPreviewHandlerVisuals,
    IOleWindow,
    IObjectWithSite,
    IInitializeWithFile,
    IInitializeWithItem
)]
struct PreviewHandler {
    state: Mutex<State>,
}

pub(crate) fn create() -> IUnknown {
    PreviewHandler {
        state: Mutex::default(),
    }
    .into()
}

/// A host that lets the handler go without `Unload` still gets its window
/// taken down (destroying the pane's parent would too, but not until then).
impl Drop for PreviewHandler {
    fn drop(&mut self) {
        if let Some(pane) = self.state().pane.take() {
            ui::post(move || {
                if let Some(window) = pane.slot().take() {
                    window::destroy(hwnd(window));
                }
            });
        }
    }
}

impl PreviewHandler {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Run `apply` on the window thread with the pane's window, if there is one
    /// by the time it runs.
    fn on_window(&self, apply: impl FnOnce(HWND) + Send + 'static) {
        let Some(pane) = self.state().pane.clone() else {
            return;
        };
        ui::post(move || {
            if let Some(window) = pane.get() {
                apply(window);
            }
        });
    }

    /// Hand the window the current colors.
    fn send_colors(&self) {
        let (background, text) = self.state().colors();
        self.on_window(move |window| window::set_colors(window, background, text));
    }
}

fn rgb(color: COLORREF) -> [u8; 3] {
    [
        (color.0 & 0xFF) as u8,
        ((color.0 >> 8) & 0xFF) as u8,
        ((color.0 >> 16) & 0xFF) as u8,
    ]
}

impl IInitializeWithFile_Impl for PreviewHandler_Impl {
    fn Initialize(&self, path: &PCWSTR, _mode: u32) -> Result<()> {
        guarded("preview Initialize (file)", || {
            if path.is_null() {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            // SAFETY: a non-null path from the host is a NUL-terminated wide
            // string, valid for the call.
            let path =
                unsafe { path.to_string() }.map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            self.state().path = Some(PathBuf::from(path));
            Ok(())
        })
    }
}

impl IInitializeWithItem_Impl for PreviewHandler_Impl {
    fn Initialize(&self, item: Ref<IShellItem>, _mode: u32) -> Result<()> {
        guarded("preview Initialize (item)", || {
            let item = item.ok()?;
            // SAFETY: asks the item for its file-system path, which the shell
            // allocates with the COM allocator; read once and freed below.
            let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }?;
            // SAFETY: as above.
            let path = unsafe { name.to_string() };
            // SAFETY: as above; nothing reads `name` after this.
            unsafe { CoTaskMemFree(Some(name.0.cast_const().cast())) };
            let path = path.map_err(|_| Error::from_hresult(E_INVALIDARG))?;
            self.state().path = Some(PathBuf::from(path));
            Ok(())
        })
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, parent: HWND, rect: *const RECT) -> Result<()> {
        guarded("SetWindow", || {
            if rect.is_null() {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            // SAFETY: a non-null rectangle from the host, valid for the call.
            let rect = unsafe { *rect };
            let parent = parent.0 as Handle;
            {
                let mut state = self.state();
                state.parent = Some(parent);
                state.rect = rect;
            }
            self.on_window(move |window| {
                window::set_parent(window, hwnd(parent));
                window::set_rect(window, rect);
            });
            Ok(())
        })
    }

    fn SetRect(&self, rect: *const RECT) -> Result<()> {
        guarded("SetRect", || {
            if rect.is_null() {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            // SAFETY: a non-null rectangle from the host, valid for the call.
            let rect = unsafe { *rect };
            self.state().rect = rect;
            self.on_window(move |window| window::set_rect(window, rect));
            Ok(())
        })
    }

    fn DoPreview(&self) -> Result<()> {
        guarded("DoPreview", || {
            let (path, parent, rect, colors, pane) = {
                let mut state = self.state();
                if state.pane.is_some() {
                    return Ok(());
                }
                let (Some(path), Some(parent)) = (state.path.clone(), state.parent) else {
                    return Err(Error::from_hresult(E_FAIL));
                };
                let pane = Arc::new(Pane::default());
                state.pane = Some(Arc::clone(&pane));
                (path, parent, state.rect, state.colors(), pane)
            };
            log::debug!("preview of {}", path.display());
            ui::post(
                move || match window::create(hwnd(parent), rect, path, colors) {
                    Ok(window) => *pane.slot() = Some(window.0 as Handle),
                    Err(error) => log::error!("no preview window: {error}"),
                },
            );
            Ok(())
        })
    }

    fn Unload(&self) -> Result<()> {
        guarded("Unload", || {
            let pane = {
                let mut state = self.state();
                state.path = None;
                state.pane.take()
            };
            if let Some(pane) = pane {
                ui::post(move || {
                    if let Some(window) = pane.slot().take() {
                        window::destroy(hwnd(window));
                    }
                });
            }
            Ok(())
        })
    }

    fn SetFocus(&self) -> Result<()> {
        guarded("SetFocus", || {
            self.on_window(window::focus);
            Ok(())
        })
    }

    fn QueryFocus(&self) -> Result<HWND> {
        // SAFETY: reads the calling thread's focus window.
        guarded("QueryFocus", || Ok(unsafe { GetFocus() }))
    }

    /// See the module docs: keystrokes for the pane never pass through the
    /// host's loop, so there is nothing here to translate.
    fn TranslateAccelerator(&self, _message: *const MSG) -> Result<()> {
        Err(Error::from_hresult(S_FALSE))
    }
}

impl IPreviewHandlerVisuals_Impl for PreviewHandler_Impl {
    fn SetBackgroundColor(&self, color: COLORREF) -> Result<()> {
        guarded("SetBackgroundColor", || {
            self.state().background = Some(rgb(color));
            self.send_colors();
            Ok(())
        })
    }

    fn SetFont(&self, _font: *const LOGFONTW) -> Result<()> {
        // The status line keeps the system font; the pane's own is for text it
        // lays out, which a picture is not.
        Ok(())
    }

    fn SetTextColor(&self, color: COLORREF) -> Result<()> {
        guarded("SetTextColor", || {
            self.state().text = Some(rgb(color));
            self.send_colors();
            Ok(())
        })
    }
}

impl IOleWindow_Impl for PreviewHandler_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        guarded("GetWindow", || {
            let state = self.state();
            state
                .pane
                .as_ref()
                .and_then(|pane| pane.get())
                .or(state.parent.map(hwnd))
                .ok_or_else(|| Error::from_hresult(E_FAIL))
        })
    }

    fn ContextSensitiveHelp(&self, _enter: BOOL) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

impl IObjectWithSite_Impl for PreviewHandler_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> Result<()> {
        guarded("SetSite", || {
            self.state().site = site.cloned();
            Ok(())
        })
    }

    fn GetSite(&self, riid: *const GUID, site: *mut *mut c_void) -> Result<()> {
        guarded("GetSite", || {
            if riid.is_null() || site.is_null() {
                return Err(Error::from_hresult(E_POINTER));
            }
            // SAFETY: `site` is non-null and, per COM, valid for one write.
            unsafe { *site = std::ptr::null_mut() };
            let current = self.state().site.clone();
            let Some(current) = current else {
                return Err(Error::from_hresult(E_FAIL));
            };
            // SAFETY: both pointers are non-null and valid per COM's contract.
            unsafe { current.query(riid, site) }.ok()
        })
    }
}
