//! Embeds the Windows executable icon. This is the icon Explorer shows for
//! `3d-review.exe` and the one a *pinned*/unlaunched taskbar shortcut uses — it
//! is a Win32 resource baked into the binary at build time, distinct from the
//! runtime window icon set via winit (see `load_window_icon` in `main.rs`).
//!
//! No-op on non-Windows targets. A missing icon compiler (rc.exe / llvm-rc) is
//! downgraded to a warning so the build still succeeds without the embedded
//! icon rather than failing outright.
fn main() {
    #[cfg(windows)]
    {
        const ICON: &str = "../../assets/icons/application-logo.ico";
        println!("cargo:rerun-if-changed={ICON}");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ICON);
        if let Err(err) = res.compile() {
            println!("cargo:warning=failed to embed application icon: {err}");
        }
    }
}
