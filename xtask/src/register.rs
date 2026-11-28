//! `cargo xtask register` / `unregister` (`docs/ARCHITECTURE.md`, D32): this
//! checkout's release build, installed for the current user on Linux.
//!
//! Windows has Setup and macOS the .dmg, so this is only Linux's install step. It needs
//! no root, and everything lands under `~/.local`:
//!
//! * `~/.local/lib/3d-review/`: the folder the Linux tarball unpacks to, laid out by
//!   `scripts/build-linux.sh --stage`, so an install holds exactly what the tarball
//!   ships. The viewer looks for the manual's pictures beside its executable, and the
//!   CLI looks for its render engine beside its own (D31), which is why the three
//!   binaries stay together in one folder.
//! * `~/.local/bin/3d-review` and `~/.local/bin/review3d-cli`: symlinks into that
//!   folder. On Linux `current_exe()` resolves a symlink, so each binary still finds
//!   what sits beside it. `review3d-render` is not linked: it is `review3d-cli
//!   render`'s engine, not a command of its own.
//! * under `$XDG_DATA_HOME` (`~/.local/share`): the `3d-review.desktop` menu entry,
//!   named after the window's Wayland app id so the desktop matches the window to
//!   the entry; the icons in hicolor; and the `application/vnd.fbx` file type, which
//!   shared-mime-info lacks. F3D declares FBX under the same name, so the two apps
//!   share one type rather than competing over the glob. glb and glTF are
//!   shared-mime-info's own `model/gltf-binary` and `model/gltf+json`.
//! * the default application for all three types, set through `xdg-mime` (it writes
//!   `$XDG_CONFIG_HOME/mimeapps.list`).
//!
//! Run it again after a rebuild to update the install. The new folder goes in by a
//! rename, so a viewer that is still running keeps the files it started from.
//!
//! `unregister` removes all of it, and every mention of the entry in the
//! `mimeapps.list` files, so each type falls back to whatever the desktop would pick
//! next. The settings (`~/.config/3D Review`) and logs (`~/.local/share/3D Review/logs`)
//! are the user's, and stay.

#[cfg(target_os = "linux")]
pub use linux::{register, unregister};

#[cfg(not(target_os = "linux"))]
const NOT_LINUX: &str = "register is Linux's install step; Windows installs with Setup \
     (packaging/build-windows-installer.ps1) and macOS with the .dmg (scripts/build-mac.sh)";

#[cfg(not(target_os = "linux"))]
pub fn register(_args: &[String]) -> Result<(), String> {
    Err(NOT_LINUX.into())
}

#[cfg(not(target_os = "linux"))]
pub fn unregister(_args: &[String]) -> Result<(), String> {
    Err(NOT_LINUX.into())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use image::imageops::FilterType;

    /// The FBX type this install declares, and its icon's name in the icon theme: a
    /// type's default icon name is the type with `/` as `-`.
    const FBX: &str = "application/vnd.fbx";
    const FBX_ICON: &str = "application-vnd.fbx";
    /// Every type the entry opens, and is made the default application for.
    const TYPES: [&str; 3] = [FBX, "model/gltf-binary", "model/gltf+json"];
    /// The console tool, linked onto the PATH beside the viewer.
    const CLI: &str = "review3d-cli";
    /// The sizes the hicolor theme looks for.
    const SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

    /// The repository this xtask belongs to.
    fn repo() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    /// The fields of product.json the install writes out.
    struct Product {
        name: String,
        description: String,
        /// The binary's name, which also names the folder, the menu entry and the icon.
        exe: String,
        /// What a file manager calls an FBX file, as on Windows.
        fbx_name: String,
    }

    fn product() -> Result<Product, String> {
        let path = repo().join("product.json");
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let field = |pointer: &str| {
            json.pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("{} has no {pointer}", path.display()))
        };
        Ok(Product {
            name: field("/productName")?,
            description: field("/description")?,
            exe: field("/exeName")?,
            fbx_name: field("/fileAssociation/typeName")?,
        })
    }

    /// Where an install goes.
    struct Places {
        /// `~/.local/lib/3d-review`: the installed folder.
        lib: PathBuf,
        /// `~/.local/bin`: where the PATH links go.
        bin: PathBuf,
        /// `$XDG_DATA_HOME`: the menu entry, the icons and the file type.
        data: PathBuf,
        /// `$XDG_CONFIG_HOME`: where `mimeapps.list` lives.
        config: PathBuf,
        exe: String,
    }

    impl Places {
        fn new(product: &Product) -> Result<Self, String> {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .ok_or("HOME is not set")?;
            Ok(Self {
                lib: home.join(".local/lib").join(&product.exe),
                bin: home.join(".local/bin"),
                data: xdg("XDG_DATA_HOME", || home.join(".local/share")),
                config: xdg("XDG_CONFIG_HOME", || home.join(".config")),
                exe: product.exe.clone(),
            })
        }

        /// The folder a new install is laid out in, before it replaces the old one.
        fn staging(&self) -> PathBuf {
            self.lib.with_extension("part")
        }

        /// The old install, between being replaced and being deleted.
        fn retired(&self) -> PathBuf {
            self.lib.with_extension("old")
        }

        /// Each PATH link, with the binary it points at.
        fn links(&self) -> [(PathBuf, PathBuf); 2] {
            [&self.exe, CLI].map(|name| (self.bin.join(name), self.lib.join(name)))
        }

        fn desktop_id(&self) -> String {
            format!("{}.desktop", self.exe)
        }

        fn desktop_entry(&self) -> PathBuf {
            self.data.join("applications").join(self.desktop_id())
        }

        fn mime_package(&self) -> PathBuf {
            self.data
                .join("mime/packages")
                .join(format!("{}.xml", self.exe))
        }

        /// Every icon file, by size: the app's and the FBX type's.
        fn icons(&self) -> impl Iterator<Item = (u32, [PathBuf; 2])> + '_ {
            SIZES.into_iter().map(|size| {
                let dir = self.data.join(format!("icons/hicolor/{size}x{size}"));
                let app = dir.join("apps").join(format!("{}.png", self.exe));
                let fbx = dir.join("mimetypes").join(format!("{FBX_ICON}.png"));
                (size, [app, fbx])
            })
        }

        /// The `mimeapps.list` files a default can be recorded in, for this user: the
        /// config one `xdg-mime` writes, each current desktop's own, and the old one
        /// under the data directory that some tools still write.
        fn mimeapps_lists(&self) -> Vec<PathBuf> {
            let mut lists = vec![self.config.join("mimeapps.list")];
            let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            for desktop in desktops.split(':').filter(|d| !d.is_empty()) {
                let name = format!("{}-mimeapps.list", desktop.to_lowercase());
                lists.push(self.config.join(name));
            }
            lists.push(self.data.join("applications/mimeapps.list"));
            lists
        }
    }

    /// An XDG base directory: the variable when it holds an absolute path (the spec
    /// says to ignore a relative one), its default otherwise.
    fn xdg(var: &str, default: impl FnOnce() -> PathBuf) -> PathBuf {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(default)
    }

    fn step(name: &str) {
        eprintln!("\n== {name} ==");
    }

    fn io<T>(path: &Path, result: std::io::Result<T>) -> Result<T, String> {
        result.map_err(|e| format!("{}: {e}", path.display()))
    }

    fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            io(dir, fs::create_dir_all(dir))?;
        }
        io(path, fs::write(path, bytes))
    }

    /// Remove a file, if there is one.
    fn remove_file(path: &Path) -> Result<(), String> {
        match fs::remove_file(path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
            _ => Ok(()),
        }
    }

    /// Remove a folder of ours and everything in it, if there is one.
    fn remove_dir(path: &Path) -> Result<(), String> {
        match fs::remove_dir_all(path) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
            _ => Ok(()),
        }
    }

    /// Whether `dir` is an install this tool made, and so safe to replace or delete.
    /// A folder of that name that is not one is someone else's.
    fn is_install(places: &Places, dir: &Path) -> bool {
        dir.join(&places.exe).is_file() && dir.join(CLI).is_file()
    }

    /// Refuse to put a link where a real file is: replacing an earlier link is what
    /// reinstalling is, but replacing a file would be overwriting another program.
    fn check_link_free(link: &Path) -> Result<(), String> {
        match fs::symlink_metadata(link) {
            Ok(meta) if !meta.file_type().is_symlink() => Err(format!(
                "{} is a file, not a link this made; move it away and run register again",
                link.display()
            )),
            Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("{}: {e}", link.display())),
            _ => Ok(()),
        }
    }

    /// Point `link` at `target`, replacing whatever link was there in one rename.
    fn replace_link(link: &Path, target: &Path) -> Result<(), String> {
        if let Some(dir) = link.parent() {
            io(dir, fs::create_dir_all(dir))?;
        }
        let part = link.with_extension("part");
        remove_file(&part)?;
        io(&part, std::os::unix::fs::symlink(target, &part))?;
        io(link, fs::rename(&part, link))
    }

    /// Whether `link` is a link into this install, which `unregister` may remove.
    fn links_into(link: &Path, lib: &Path) -> bool {
        fs::read_link(link).is_ok_and(|target| target.starts_with(lib))
    }

    /// One argument of a desktop entry's `Exec` line: the path, quoted. A path with a
    /// character the quoting would have to escape twice (once for the argument, once
    /// for the file's string) is refused instead; no real home folder has one.
    fn exec_arg(path: &Path) -> Result<String, String> {
        let text = path
            .to_str()
            .ok_or_else(|| format!("{}: not UTF-8", path.display()))?;
        if text.contains(['"', '`', '$', '\\', '%', '\n', '\r']) {
            return Err(format!("{text}: a desktop entry cannot name this path"));
        }
        Ok(format!("\"{text}\""))
    }

    fn desktop_entry(product: &Product, places: &Places) -> Result<String, String> {
        let viewer = places.lib.join(&places.exe);
        let exec = exec_arg(&viewer)?;
        // `%F`: the viewer opens every file it is given as one session, a model and
        // its animation files together.
        Ok(format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={name}\n\
             Comment={comment}\n\
             Exec={exec} %F\n\
             TryExec={try_exec}\n\
             Icon={id}\n\
             Terminal=false\n\
             Categories=Graphics;3DGraphics;Viewer;\n\
             Keywords=FBX;glTF;glb;3D;model;viewer;\n\
             MimeType={types};\n\
             StartupWMClass={id}\n",
            name = product.name,
            comment = product.description,
            try_exec = viewer.display(),
            id = places.exe,
            types = TYPES.join(";"),
        ))
    }

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// The FBX type. The magic catches a file whose name lost its extension: a binary
    /// FBX opens with `Kaydara FBX Binary`, an ASCII one with a `; FBX` comment.
    fn mime_package(product: &Product) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="{FBX}">
    <comment>{comment}</comment>
    <acronym>FBX</acronym>
    <expanded-acronym>Filmbox</expanded-acronym>
    <icon name="{FBX_ICON}"/>
    <magic>
      <match type="string" value="Kaydara FBX Binary" offset="0"/>
      <match type="string" value="; FBX" offset="0"/>
    </magic>
    <glob pattern="*.fbx"/>
  </mime-type>
</mime-info>
"#,
            comment = xml_escape(&product.fbx_name),
        )
    }

    /// The icons, scaled from the logo the window icon is made from.
    fn write_icons(places: &Places) -> Result<(), String> {
        let logo_path = repo().join("assets/icons/application-logo.png");
        let logo = image::open(&logo_path)
            .map_err(|e| format!("{}: {e}", logo_path.display()))?
            .into_rgba8();
        for (size, paths) in places.icons() {
            let icon = image::imageops::resize(&logo, size, size, FilterType::Lanczos3);
            for path in &paths {
                if let Some(dir) = path.parent() {
                    io(dir, fs::create_dir_all(dir))?;
                }
                icon.save_with_format(path, image::ImageFormat::Png)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        Ok(())
    }

    /// Tell the desktop what changed. Desktops ship different tools (KDE has
    /// `kbuildsycoca6`, GNOME has none of its own), so one that is missing is only
    /// reported.
    fn refresh(places: &Places) {
        let hicolor = places.data.join("icons/hicolor");
        let mut mime = Command::new("update-mime-database");
        mime.arg(places.data.join("mime"));
        let mut apps = Command::new("update-desktop-database");
        apps.arg(places.data.join("applications"));
        let mut tools = vec![mime, apps];
        // Only a theme folder with an index can have a cache; the user's usually has
        // neither, and the desktop then reads the folders as they are.
        if hicolor.join("index.theme").is_file() {
            let mut cache = Command::new("gtk-update-icon-cache");
            cache.args(["-f", "-t"]).arg(&hicolor);
            tools.push(cache);
        }
        tools.push(Command::new("kbuildsycoca6"));
        for mut tool in tools {
            let name = tool.get_program().to_string_lossy().into_owned();
            match tool.output() {
                Ok(out) if out.status.success() => {}
                Ok(out) => eprintln!(
                    "  {name} failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
                Err(_) => eprintln!("  {name} not found; skipped"),
            }
        }
    }

    fn run(command: &mut Command) -> Result<(), String> {
        let name = command.get_program().to_string_lossy().into_owned();
        let status = command
            .status()
            .map_err(|e| format!("could not run {name}: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{name} failed ({status})"))
        }
    }

    /// The default application `xdg-mime` reports for a type.
    fn default_for(mime: &str) -> String {
        Command::new("xdg-mime")
            .args(["query", "default", mime])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default()
    }

    /// `list` (a `mimeapps.list`) with `id` taken out of every entry, and an entry
    /// that named nothing else dropped; `None` when it never mentions `id`.
    fn without_entry(list: &str, id: &str) -> Option<String> {
        let mut changed = false;
        let mut out = String::with_capacity(list.len());
        for line in list.lines() {
            let entry = line.trim_start();
            if !entry.starts_with(['#', '['])
                && let Some((key, value)) = line.split_once('=')
                && value.split(';').any(|item| item.trim() == id)
            {
                changed = true;
                let kept: Vec<&str> = value
                    .split(';')
                    .map(str::trim)
                    .filter(|item| !item.is_empty() && *item != id)
                    .collect();
                if !kept.is_empty() {
                    out.push_str(&format!("{key}={};\n", kept.join(";")));
                }
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        changed.then_some(out)
    }

    pub fn register(args: &[String]) -> Result<(), String> {
        let mut build = true;
        for arg in args {
            match arg.as_str() {
                "--no-build" => build = false,
                other => return Err(format!("register: unknown option `{other}`")),
            }
        }
        let product = product()?;
        let places = Places::new(&product)?;
        // Every refusal before the build, so none costs one.
        for (link, _) in places.links() {
            check_link_free(&link)?;
        }
        if places.lib.exists() && !is_install(&places, &places.lib) {
            return Err(format!(
                "{} exists and is not a 3D Review install; move it away and run register again",
                places.lib.display()
            ));
        }
        // Left by a run that was stopped part way; both names are this tool's own.
        remove_dir(&places.staging())?;
        remove_dir(&places.retired())?;

        let staging = places.staging();
        let mut script = Command::new(repo().join("scripts/build-linux.sh"));
        script.arg("--stage").arg(&staging);
        if !build {
            script.arg("--no-build");
        }
        run(&mut script)?;

        step("install");
        // Rename the old folder away rather than delete it under a running viewer:
        // the viewer keeps its open files either way, but the swap leaves no moment
        // with half an install in place.
        if places.lib.exists() {
            io(&places.lib, fs::rename(&places.lib, places.retired()))?;
        }
        io(&places.lib, fs::rename(&staging, &places.lib))?;
        remove_dir(&places.retired())?;
        eprintln!("  {}", places.lib.display());
        for (link, target) in places.links() {
            replace_link(&link, &target)?;
            eprintln!("  {} -> {}", link.display(), target.display());
        }

        step("desktop entry, icons and file type");
        write_icons(&places)?;
        eprintln!("  icons in {}", places.data.join("icons/hicolor").display());
        let entry = desktop_entry(&product, &places)?;
        write(&places.desktop_entry(), entry.as_bytes())?;
        eprintln!("  {}", places.desktop_entry().display());
        write(&places.mime_package(), mime_package(&product).as_bytes())?;
        eprintln!("  {}", places.mime_package().display());
        refresh(&places);

        step("default application");
        let id = places.desktop_id();
        // What xdg-mime says on the way is not the answer; the query after it is. On
        // Plasma 6 it complains that `qtpaths` is missing (Qt 6 names it `qtpaths6`),
        // gives up on its KDE branch, and then writes the same `mimeapps.list` through
        // its generic one. So its messages are shown only when a default did not take.
        let set = Command::new("xdg-mime")
            .arg("default")
            .arg(&id)
            .args(TYPES)
            .output()
            .map_err(|e| format!("could not run xdg-mime: {e}"))?;
        for mime in TYPES {
            let found = default_for(mime);
            if !set.status.success() || found != id {
                return Err(format!(
                    "xdg-mime did not make {id} the default for {mime} (it reports `{found}`): {}",
                    String::from_utf8_lossy(&set.stderr).trim()
                ));
            }
            eprintln!("  {mime} -> {id}");
        }

        let path = std::env::var_os("PATH").unwrap_or_default();
        if !std::env::split_paths(&path).any(|dir| dir == places.bin) {
            eprintln!(
                "\nnote: {} is not on your PATH, so a shell will not find {} or {CLI} until it is",
                places.bin.display(),
                places.exe,
            );
        }
        println!(
            "\n{} is installed for this user in {}, and opens .fbx, .glb and .gltf files \
             (`cargo xtask unregister` takes it out)",
            product.name,
            places.lib.display(),
        );
        Ok(())
    }

    pub fn unregister(args: &[String]) -> Result<(), String> {
        if let Some(other) = args.first() {
            return Err(format!("unregister: unknown option `{other}`"));
        }
        let product = product()?;
        let places = Places::new(&product)?;

        for (link, _) in places.links() {
            if links_into(&link, &places.lib) {
                remove_file(&link)?;
            }
        }
        if is_install(&places, &places.lib) {
            remove_dir(&places.lib)?;
        } else if places.lib.exists() {
            eprintln!(
                "  {} is not a 3D Review install; left as it is",
                places.lib.display()
            );
        }
        remove_dir(&places.staging())?;
        remove_dir(&places.retired())?;

        remove_file(&places.desktop_entry())?;
        remove_file(&places.mime_package())?;
        for path in places.icons().flat_map(|(_, paths)| paths) {
            remove_file(&path)?;
        }

        let id = places.desktop_id();
        for list in places.mimeapps_lists() {
            let Ok(text) = fs::read_to_string(&list) else {
                continue;
            };
            if let Some(kept) = without_entry(&text, &id) {
                let part = list.with_extension("list.part");
                write(&part, kept.as_bytes())?;
                io(&list, fs::rename(&part, &list))?;
                eprintln!("  {id} taken out of {}", list.display());
            }
        }
        refresh(&places);
        println!(
            "{} is no longer installed for this user (its settings and logs stay)",
            product.name
        );
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_default_naming_only_us_goes_and_a_shared_one_loses_us() {
            let list = "[Default Applications]\n\
                        model/gltf-binary=3d-review.desktop\n\
                        application/vnd.fbx=3d-review.desktop;\n\
                        text/plain=org.kde.kate.desktop\n\
                        \n\
                        [Added Associations]\n\
                        model/gltf+json=f3d.desktop;3d-review.desktop;blender.desktop;\n";
            assert_eq!(
                without_entry(list, "3d-review.desktop").as_deref(),
                Some(
                    "[Default Applications]\n\
                     text/plain=org.kde.kate.desktop\n\
                     \n\
                     [Added Associations]\n\
                     model/gltf+json=f3d.desktop;blender.desktop;\n"
                )
            );
        }

        #[test]
        fn a_list_that_never_names_us_is_left_alone() {
            let list = "[Default Applications]\n\
                        # 3d-review.desktop was here\n\
                        model/gltf-binary=f3d.desktop;\n\
                        image/png=my-3d-review.desktop\n";
            assert_eq!(without_entry(list, "3d-review.desktop"), None);
        }

        #[test]
        fn the_exec_line_quotes_a_path_and_refuses_one_it_cannot() {
            assert_eq!(
                exec_arg(Path::new("/home/a b/.local/lib/3d-review/3d-review")).as_deref(),
                Ok("\"/home/a b/.local/lib/3d-review/3d-review\"")
            );
            assert!(exec_arg(Path::new("/home/$x/3d-review")).is_err());
            assert!(exec_arg(Path::new("/home/100%/3d-review")).is_err());
        }
    }
}
