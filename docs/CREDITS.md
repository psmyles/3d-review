# Credits

3D Review is a small program standing on a lot of other people's work. Almost none of the hard
parts - reading FBX correctly, simplifying a mesh without wrecking it, decoding a dozen texture
formats, driving two GPU APIs, drawing a UI - are 3D Review's own. This page says who wrote them.

For the formal license terms and per-package copyright notices, see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## FBX

3D Review exists to look at FBX files, and every one of them is read by somebody else's parser.

* **[ufbx](https://github.com/ufbx/ufbx)** - Samuli Raivio. The FBX reader: geometry, materials,
  the scene graph, skins, blend shapes and every animation stack, from a single C file that is
  both fast and correct on the files real DCC tools write. A great deal of what 3D Review can
  show - rest pose versus bind pose, baked animation, the authored properties the export gives
  back - is only possible because ufbx already worked it out.
* **[ufbx_write](https://github.com/ufbx/ufbx-write)** - Samuli Raivio. The FBX writer behind the
  Opt workspace's export. 3D Review carries a handful of patches on top of it, documented in
  `third_party/ufbx-write/NOTICE.txt`.

## Mesh optimization

* **[meshoptimizer](https://github.com/zeux/meshoptimizer)** - Arseny Kapoulkine. The Opt
  workspace: welding, simplification and LOD chains, the vertex-cache / overdraw / vertex-fetch
  reorders, and the ACMR / ATVR / overdraw / overfetch figures that make those reorders worth
  having. The retopologizer is 3D Review's own, but it stands on meshoptimizer's measurements.

## Textures and image decoding

* **[image](https://github.com/image-rs/image)** - the image-rs developers. PNG, TGA, TIFF,
  Radiance HDR, BMP, GIF and PNM textures, and the icons in the chrome. With it come
  **[png](https://github.com/image-rs/image-png)**,
  **[tiff](https://github.com/image-rs/image-tiff)**,
  **[fdeflate](https://github.com/image-rs/fdeflate)** and
  **[weezl](https://github.com/image-rs/weezl)**, plus **[fax](https://github.com/pdf-rs/fax)** by
  the pdf-rs contributors and **[moxcms](https://github.com/awxkee/moxcms)** /
  **[pxfm](https://github.com/awxkee/pxfm)** by Radzivon Bartoshyk.
* **[zune-image](https://github.com/etemesi254/zune-image)** - Caleb Etemesi and the zune-image
  developers. The fast path for JPEG textures.
* **[psd_sdk](https://github.com/MolecularMatters/psd_sdk)** - Stefan Reinalter / Molecular
  Matters. Layered Photoshop `.psd` source art, read without a bundled ImageMagick.
* **[half](https://github.com/VoidStarKat/half-rs)** - Kathryn Long. `f16` for HDR data.

## Window, GPU and interface

3D Review's viewport and chrome are the same code on Windows and macOS because these projects
absorb the difference.

* **[winit](https://github.com/rust-windowing/winit)** - the winit contributors, Pierre Krieger,
  Kirill Chibisov and many others. The window, the event loop, input, DPI and drag-and-drop on
  both operating systems, along with **[dpi](https://github.com/rust-windowing/winit)**,
  **[raw-window-handle](https://github.com/rust-windowing/raw-window-handle)** and
  **[cursor-icon](https://github.com/rust-windowing/cursor-icon)**.
* **[sokol](https://github.com/floooh/sokol)** and
  **[sokol-rust](https://github.com/floooh/sokol-rust)** - Andre Weissflog (bindings by Erik
  Wilhelm Gren). `sokol_gfx` is the one drawing API over Direct3D 11 and Metal, and `sokol-shdc`
  turns 3D Review's one shader source into both backends' code and the reflection to bind it.
* **[egui](https://github.com/emilk/egui)** - Emil Ernerfeldt and contributors. Every panel,
  window, slider and button of the chrome, with **egui-winit**, **egui_extras**, **epaint**,
  **emath** and **ecolor**. egui's text is shaped and drawn by Linebender's stack:
  **[skrifa, read-fonts and font-types](https://github.com/googlefonts/fontations)** (the
  Fontations developers), **[harfrust](https://github.com/harfbuzz/harfrust)** (the HarfBuzz
  developers and Yevhenii Reizner), and **[vello_cpu](https://github.com/linebender/vello)**,
  **[kurbo](https://github.com/linebender/kurbo)**, **[peniko](https://github.com/linebender/peniko)**
  and **[color](https://github.com/linebender/color)** (Raph Levien and the Linebender and Vello
  authors).
* **[egui_commonmark](https://github.com/lampsitter/egui_commonmark)** - Erlend Walstad, over
  **[pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark)** by Raph Levien and Marcus
  Klaas de Vries. The in-app manual: the same markdown the manual's website is built from, drawn
  with egui's own widgets.
* **[AccessKit](https://github.com/AccessKit/accesskit)** - the AccessKit contributors.
* **[arboard](https://github.com/1Password/arboard)** - the Arboard contributors, with
  **[clipboard-win](https://github.com/DoumanAsh/clipboard-win)** by Douman. Copy and paste.
* **[webbrowser](https://github.com/amodm/webbrowser-rs)** - Amod Malviya. Opens a link from the
  manual in your browser.
* **[glam](https://github.com/bitshifter/glam-rs)** - Cameron Hart. Every vector and matrix in
  the camera, the import and the optimizer.

## Typefaces

* **[Inter](https://github.com/rsms/inter)** - Rasmus Andersson and the Inter Project Authors.
  The interface typeface.
* **[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono)** - JetBrains, designed by
  Philipp Nurullin and Konstantin Bulenkov. The numeric value boxes.
* egui's own fallback faces, carried along for glyphs the two above lack:
  **[Ubuntu](https://design.ubuntu.com/font)** (Canonical), **[Hack](https://github.com/source-foundry/Hack)**
  (Source Foundry, from Bitstream Vera and DejaVu), **[Noto Emoji](https://github.com/googlefonts/noto-emoji)**
  (Google) and **[emoji-icon-font](https://github.com/jslegers/emoji-icon-font)** (John Slegers).

## Rendering research

Most of 3D Review's shader code is its own, but the parts that make it look right are largely
other people's - some ported outright (see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for which), and all of it built on their ideas:

* **[XeGTAO](https://github.com/GameTechDev/XeGTAO)** - Filip Strugar and Steve McCalla at Intel.
  The ambient occlusion pass is a translation of its horizon search and depth prefilter, which
  is where most of the pass's quality comes from.
* **Jorge Jimenez and colleagues at Activision** - "Practical Real-Time Strategies for Accurate
  Indirect Occlusion" (2016), the ground-truth ambient occlusion technique itself and the
  spatial and temporal patterns the pass converges with.
* **[Khronos PBR Neutral](https://github.com/KhronosGroup/ToneMapping)** - the Khronos Group. The
  default tone mapper, ported from their reference.
* **[three.js](https://github.com/mrdoob/three.js)** - the three.js authors. The ACES Filmic and
  AgX tone mappers are ported from three.js.
* **Stephen Hill** - the fitted ACES curve, as published in MJP's
  [BakingLab](https://github.com/TheRealMJP/BakingLab).
* **Troy Sobotka** - AgX, with Google Filament's implementation and Benjamin Wrensch's fit of its
  contrast curve in between.

## Lighting environments

* **[Poly Haven](https://polyhaven.com)** - the six HDR environments the Shaded view is lit by,
  and which the skybox shows. Released under CC0, free for anyone to use; credited here because
  the viewer would look a great deal worse without them.

## Localization

* **[Project Fluent](https://projectfluent.org)** - Mozilla and the Fluent contributors, Zibi
  Braniecki, Staś Małolepszy, Caleb Maclennan and Bruce Mitchener among them. Every string on
  screen, through **[fluent-bundle](https://github.com/projectfluent/fluent-rs)**,
  **fluent-syntax**, **fluent-langneg**, **intl-memoizer**,
  **[intl_pluralrules](https://github.com/zbraniecki/pluralrules)** and
  **[unic-langid](https://github.com/zbraniecki/unic-locale)**.
* **[sys-locale](https://github.com/1Password/sys-locale)** - 1Password. Starting in the language
  your operating system is set to.
* **[ICU4X](https://github.com/unicode-org/icu4x)** - the ICU4X Project Developers, Unicode, Inc.
  Unicode normalization, reached through `url` and `idna`.

## Platform

* **[windows and windows-sys](https://github.com/microsoft/windows-rs)** - Microsoft. Rust
  bindings for the Win32 and COM surface 3D Review names directly on Windows: the D3D11 device and
  DXGI swapchain the renderer draws through.
* **[objc2](https://github.com/madsmtm/objc2)** - Mads Marquart, Steven Sheldon and contributors.
  The Objective-C bridge behind the macOS leaf: the `CAMetalLayer` sokol_gfx draws through.
* **[muda](https://github.com/tauri-apps/muda)** - the Tauri programme. The macOS menu bar, with
  **[keyboard-types](https://github.com/pyfisch/keyboard-types)** by Pyfisch.
* **[rfd](https://github.com/PolyMeilex/rfd)** - Bartłomiej Maryńczak. The native open and save
  dialogs and the startup error box.
* **[dirs](https://github.com/soc/dirs-rs)** - Simon Ochsenreither. Where the window's position
  and your settings are kept on each OS, with **[option-ext](https://github.com/soc/option-ext)**.
* **[notify](https://github.com/notify-rs/notify)** - Félix Saparelli, Daniel Faust, Aron Heinecke
  and contributors. The file watcher that reloads a texture the moment you save it.
* **[Inno Setup](https://jrsoftware.org/isinfo.php)** - Jordan Russell and Martijn Laan. The
  Windows installer.

## Profiling

* **[Tracy](https://github.com/wolfpld/tracy)** - Bartosz Taudul. The `--tracy` CPU and GPU
  profiler 3D Review's performance work is measured with, through
  **[tracy-client](https://github.com/nagisa/rust_tracy_client)** by Simonas Kazlauskas.

## Infrastructure

The crates that do not show up in a feature list but without which none of the above compiles:

* **[serde](https://github.com/serde-rs/serde)** and
  **[serde_json](https://github.com/serde-rs/json)** - Erick Tryzelaar, David Tolnay and
  contributors. The Opt workspace's saved presets.
* **[bytemuck](https://github.com/Lokathor/bytemuck)** - Daniel "Lokathor" Gee. Every vertex
  and uniform block goes to the GPU as a safe cast.
* **[syn](https://github.com/dtolnay/syn)**, **[quote](https://github.com/dtolnay/quote)**,
  **[proc-macro2](https://github.com/dtolnay/proc-macro2)**,
  **[thiserror](https://github.com/dtolnay/thiserror)**,
  **[anyhow](https://github.com/dtolnay/anyhow)** - David Tolnay. Half the Rust ecosystem, really.
* **[flate2](https://github.com/rust-lang/flate2-rs)**,
  **[miniz_oxide](https://github.com/Frommi/miniz_oxide)**,
  **[zerocopy](https://github.com/google/zerocopy)**,
  **[parking_lot](https://github.com/Amanieu/parking_lot)**,
  **[smallvec](https://github.com/servo/rust-smallvec)**,
  **[hashbrown](https://github.com/rust-lang/hashbrown)** and the rest of the long tail - see
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for the complete list of 201 crates.

## Build tooling

Not linked into the 3D Review binary, but 3D Review would not exist without them:

* **[cc](https://github.com/rust-lang/cc-rs)** - the Rust project. Compiles ufbx, ufbx_write,
  meshoptimizer, psd_sdk and sokol from source on every build.
* **`sokol-shdc`** - Andre Weissflog. One shader source in, Direct3D and Metal shaders out.
* **`fxc` (Windows SDK) and `xcrun metal` (Xcode)** - Microsoft and Apple. The offline shader
  compilers that keep shader compilation off 3D Review's launch path on each OS.
* **[intel_tex_2](https://github.com/Traverse-Research/intel-tex-rs-2)** - Traverse Research,
  over Intel's ISPC Texture Compressor, with **[ispc-rs](https://github.com/Twinklebear/ispc-rs)**
  by Will Usher. Encodes the baked lighting maps to BC6H in the offline bake tool.
* **[winresource](https://github.com/BenjaminRi/winresource)** - Benjamin Richner. The version
  resource and Explorer icon on Windows.
* **[sha2](https://github.com/RustCrypto/hashes)** - the RustCrypto developers. The
  shader-bytecode manifest.
* **[ImageMagick](https://imagemagick.org)** - ImageMagick Studio LLC. The environment thumbnails
  and the application icons, at packaging time.
* **[mdBook](https://github.com/rust-lang/mdBook)** - the Rust project. The manual's website.
* **The Rust project and its contributors** - the language, the compiler, the standard library,
  and Cargo.

## Development

* **[Claude Code](https://claude.com/claude-code)** - Anthropic. Used throughout 3D Review's
  development as a coding assistant.

## Not third-party

3D Review's toolbar and node icons and its application logo (`assets/icons/`), its UV checker
textures (`assets/textures/T_UV_Checker_*.png`), its theme (`crates/ui/src/theme/`), its shaders
(`crates/render/src/shaders/review.glsl`) apart from the ported parts named above, its retopologizer
(`crates/optimize/src/remesh/`), its message catalogs and its manual are original work, MIT
licensed along with the rest of 3D Review.

---

*Something missing or miscredited? Please open an issue at
<https://github.com/psmyles/3d-review/issues/new>.*
