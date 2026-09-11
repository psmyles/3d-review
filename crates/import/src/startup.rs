//! Whether this process was asked to start maximized.
//!
//! Nothing to do with FBX: it lives in this crate because it reads the Windows
//! `STARTUPINFOW` the shell hands a launched process, and invariant 9 confines
//! FFI to a named set of crates, of which this is one. `app` reads the answer as
//! a plain `bool` and never sees the Win32 call.

/// Whether the process was asked to start maximized — e.g. launched from a
/// shortcut whose **Run** field is set to *Maximized*. Windows passes that hint
/// through `STARTUPINFO.wShowWindow`, but winit creates its window without
/// consulting it, so `app` queries this to set the initial window state.
/// Always `false` off Windows. (Lives here because invariant 9 funnels every
/// `unsafe`/FFI through `crates/import`.)
#[cfg(windows)]
pub fn startup_show_maximized() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMAXIMIZED;

    // SAFETY: `GetStartupInfoW` fills the entire caller-provided `STARTUPINFOW`
    // and cannot fail; we hand it a zeroed, correctly-sized struct and only read
    // the scalar `dwFlags`/`wShowWindow` fields it sets.
    let info = unsafe {
        let mut info: STARTUPINFOW = std::mem::zeroed();
        GetStartupInfoW(&mut info);
        info
    };

    info.dwFlags & STARTF_USESHOWWINDOW != 0 && i32::from(info.wShowWindow) == SW_SHOWMAXIMIZED
}

/// Off Windows there is no `STARTUPINFO` launch hint; never request maximize.
#[cfg(not(windows))]
pub fn startup_show_maximized() -> bool {
    false
}
