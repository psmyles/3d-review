# Credits

3D Review is built on the projects listed below. For license terms and per-package copyright
notices, see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## FBX

- **[ufbx](https://github.com/ufbx/ufbx)** - Samuli Raivio. FBX import: geometry, materials, the
  scene graph, skins, blend shapes and animation.
- **[ufbx_write](https://github.com/ufbx/ufbx-write)** - Samuli Raivio. FBX export from the Opt
  workspace. 3D Review's patches are listed in `third_party/ufbx-write/NOTICE.txt`.

## Mesh optimization

- **[meshoptimizer](https://github.com/zeux/meshoptimizer)** - Arseny Kapoulkine. Welding,
  simplification, LOD chains, the vertex-cache / overdraw / vertex-fetch reorders, and the
  ACMR / ATVR / overdraw / overfetch measurements in the Opt workspace.

## Textures and image decoding

- **[image](https://github.com/image-rs/image)** - the image-rs developers. PNG, TGA, TIFF,
  Radiance HDR, BMP, GIF and PNM textures, and the interface icons. Includes
  **[png](https://github.com/image-rs/image-png)**,
  **[tiff](https://github.com/image-rs/image-tiff)**,
  **[fdeflate](https://github.com/image-rs/fdeflate)** and
  **[weezl](https://github.com/image-rs/weezl)**, plus **[fax](https://github.com/pdf-rs/fax)** by
  the pdf-rs contributors and **[moxcms](https://github.com/awxkee/moxcms)** /
  **[pxfm](https://github.com/awxkee/pxfm)** by Radzivon Bartoshyk.
- **[zune-image](https://github.com/etemesi254/zune-image)** - Caleb Etemesi and the zune-image
  developers. JPEG textures.
- **[psd_sdk](https://github.com/MolecularMatters/psd_sdk)** - Stefan Reinalter / Molecular
  Matters. Photoshop `.psd` textures.
- **[half](https://github.com/VoidStarKat/half-rs)** - Kathryn Long. `f16` for HDR data.

## Window, GPU and interface

- **[winit](https://github.com/rust-windowing/winit)** - the winit contributors, Pierre Krieger,
  Kirill Chibisov and others. Window, event loop, input, DPI and drag-and-drop, with
  **[dpi](https://github.com/rust-windowing/winit)**,
  **[raw-window-handle](https://github.com/rust-windowing/raw-window-handle)** and
  **[cursor-icon](https://github.com/rust-windowing/cursor-icon)**.
- **[sokol](https://github.com/floooh/sokol)** and
  **[sokol-rust](https://github.com/floooh/sokol-rust)** - Andre Weissflog (bindings by Erik
  Wilhelm Gren). `sokol_gfx` draws over Direct3D 11 and Metal; `sokol-shdc` generates the
  per-backend shaders and their reflection.
- **[egui](https://github.com/emilk/egui)** - Emil Ernerfeldt and contributors. The interface,
  with **egui-winit**, **egui_extras**, **epaint**, **emath** and **ecolor**. Text shaping and
  rendering:
  **[skrifa, read-fonts and font-types](https://github.com/googlefonts/fontations)** (the
  Fontations developers), **[harfrust](https://github.com/harfbuzz/harfrust)** (the HarfBuzz
  developers and Yevhenii Reizner), and **[vello_cpu](https://github.com/linebender/vello)**,
  **[kurbo](https://github.com/linebender/kurbo)**, **[peniko](https://github.com/linebender/peniko)**
  and **[color](https://github.com/linebender/color)** (Raph Levien and the Linebender and Vello
  authors).
- **[egui_commonmark](https://github.com/lampsitter/egui_commonmark)** - Erlend Walstad, over
  **[pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark)** by Raph Levien and Marcus
  Klaas de Vries. The in-app manual.
- **[AccessKit](https://github.com/AccessKit/accesskit)** - the AccessKit contributors.
  Accessibility.
- **[arboard](https://github.com/1Password/arboard)** - the Arboard contributors, with
  **[clipboard-win](https://github.com/DoumanAsh/clipboard-win)** by Douman. Clipboard.
- **[webbrowser](https://github.com/amodm/webbrowser-rs)** - Amod Malviya. Opens links in the
  browser.
- **[glam](https://github.com/bitshifter/glam-rs)** - Cameron Hart. Vector and matrix math.

## Typefaces

- **[Inter](https://github.com/rsms/inter)** - Rasmus Andersson and the Inter Project Authors.
  Interface text.
- **[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono)** - JetBrains, designed by
  Philipp Nurullin and Konstantin Bulenkov. Numeric value boxes.
- egui's fallback faces:
  **[Ubuntu](https://design.ubuntu.com/font)** (Canonical), **[Hack](https://github.com/source-foundry/Hack)**
  (Source Foundry, from Bitstream Vera and DejaVu), **[Noto Emoji](https://github.com/googlefonts/noto-emoji)**
  (Google) and **[emoji-icon-font](https://github.com/jslegers/emoji-icon-font)** (John Slegers).

## Rendering research

Techniques 3D Review's shaders use or port. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
lists which parts are ported.

- **[XeGTAO](https://github.com/GameTechDev/XeGTAO)** - Filip Strugar and Steve McCalla at Intel.
  The ambient occlusion pass ports its horizon search and depth prefilter.
- **Jorge Jimenez and colleagues at Activision** - "Practical Real-Time Strategies for Accurate
  Indirect Occlusion" (2016). The ground-truth ambient occlusion technique and its spatial and
  temporal patterns.
- **[Khronos PBR Neutral](https://github.com/KhronosGroup/ToneMapping)** - the Khronos Group. The
  default tone mapper.
- **[three.js](https://github.com/mrdoob/three.js)** - the three.js authors. The ACES Filmic and
  AgX tone mappers.
- **Stephen Hill** - the fitted ACES curve, as published in MJP's
  [BakingLab](https://github.com/TheRealMJP/BakingLab).
- **Troy Sobotka** - AgX, via Google Filament's implementation and Benjamin Wrensch's contrast
  curve fit.

## Lighting environments

- **[Poly Haven](https://polyhaven.com)** - the six HDR environments used for lighting and the
  skybox. CC0.

## Localization

- **[Project Fluent](https://projectfluent.org)** - Mozilla and the Fluent contributors, including
  Zibi Braniecki, Staś Małolepszy, Caleb Maclennan and Bruce Mitchener. Interface strings, through
  **[fluent-bundle](https://github.com/projectfluent/fluent-rs)**,
  **fluent-syntax**, **fluent-langneg**, **intl-memoizer**,
  **[intl_pluralrules](https://github.com/zbraniecki/pluralrules)** and
  **[unic-langid](https://github.com/zbraniecki/unic-locale)**.
- **[sys-locale](https://github.com/1Password/sys-locale)** - 1Password. Detects the system
  language.
- **[ICU4X](https://github.com/unicode-org/icu4x)** - the ICU4X Project Developers, Unicode, Inc.
  Unicode normalization, via `url` and `idna`.

## Platform

- **[windows and windows-sys](https://github.com/microsoft/windows-rs)** - Microsoft. Win32 and
  COM bindings for the D3D11 device and DXGI swapchain.
- **[objc2](https://github.com/madsmtm/objc2)** - Mads Marquart, Steven Sheldon and contributors.
  Objective-C bindings for the macOS `CAMetalLayer` and shell.
- **[muda](https://github.com/tauri-apps/muda)** - the Tauri programme. The macOS menu bar, with
  **[keyboard-types](https://github.com/pyfisch/keyboard-types)** by Pyfisch.
- **[rfd](https://github.com/PolyMeilex/rfd)** - Bartłomiej Maryńczak. Open and save dialogs and
  the startup error box.
- **[dirs](https://github.com/soc/dirs-rs)** - Simon Ochsenreither. Per-user config location, with
  **[option-ext](https://github.com/soc/option-ext)**.
- **[notify](https://github.com/notify-rs/notify)** - Félix Saparelli, Daniel Faust, Aron Heinecke
  and contributors. Texture auto-reload.
- **[Inno Setup](https://jrsoftware.org/isinfo.php)** - Jordan Russell and Martijn Laan. The
  Windows installer.

## Profiling

- **[Tracy](https://github.com/wolfpld/tracy)** - Bartosz Taudul. The `--tracy` CPU and GPU
  profiler, through **[tracy-client](https://github.com/nagisa/rust_tracy_client)** by Simonas
  Kazlauskas.

## Infrastructure

- **[serde](https://github.com/serde-rs/serde)** and
  **[serde_json](https://github.com/serde-rs/json)** - Erick Tryzelaar, David Tolnay and
  contributors. Opt workspace presets.
- **[bytemuck](https://github.com/Lokathor/bytemuck)** - Daniel "Lokathor" Gee. GPU buffer casts.
- **[syn](https://github.com/dtolnay/syn)**, **[quote](https://github.com/dtolnay/quote)**,
  **[proc-macro2](https://github.com/dtolnay/proc-macro2)**,
  **[thiserror](https://github.com/dtolnay/thiserror)**,
  **[anyhow](https://github.com/dtolnay/anyhow)** - David Tolnay.
- **[flate2](https://github.com/rust-lang/flate2-rs)**,
  **[miniz_oxide](https://github.com/Frommi/miniz_oxide)**,
  **[zerocopy](https://github.com/google/zerocopy)**,
  **[parking_lot](https://github.com/Amanieu/parking_lot)**,
  **[smallvec](https://github.com/servo/rust-smallvec)**,
  **[hashbrown](https://github.com/rust-lang/hashbrown)** and others - see
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for the complete list of 201 crates.

## Build tooling

Used to build 3D Review; not linked into the binary.

- **[cc](https://github.com/rust-lang/cc-rs)** - the Rust project. Compiles ufbx, ufbx_write,
  meshoptimizer, psd_sdk and sokol.
- **`sokol-shdc`** - Andre Weissflog. Shader cross-compilation to Direct3D and Metal.
- **`fxc` (Windows SDK) and `xcrun metal` (Xcode)** - Microsoft and Apple. Offline shader
  compilers.
- **[intel_tex_2](https://github.com/Traverse-Research/intel-tex-rs-2)** - Traverse Research,
  over Intel's ISPC Texture Compressor, with **[ispc-rs](https://github.com/Twinklebear/ispc-rs)**
  by Will Usher. BC6H encoding in the offline IBL bake.
- **[winresource](https://github.com/BenjaminRi/winresource)** - Benjamin Richner. Windows version
  resource and icon.
- **[sha2](https://github.com/RustCrypto/hashes)** - the RustCrypto developers. The
  shader-bytecode manifest.
- **[ImageMagick](https://imagemagick.org)** - ImageMagick Studio LLC. Environment thumbnails and
  application icons, at packaging time.
- **[mdBook](https://github.com/rust-lang/mdBook)** - the Rust project. The manual's website.
- **The Rust project and its contributors** - the language, compiler, standard library and Cargo.

## Development

- **[Claude Code](https://claude.com/claude-code)** - Anthropic. Coding assistant.

## Original work

The following are original to 3D Review and MIT licensed with the rest of the project: the toolbar
and node icons and application logo (`assets/icons/`), the UV checker textures
(`assets/textures/T_UV_Checker_*.png`), the theme (`crates/ui/src/theme/`), the shaders
(`crates/render/src/shaders/review.glsl`) apart from the ported parts listed above, the
retopologizer (`crates/optimize/src/remesh/`), the message catalogs and the manual.

---

_Something missing or miscredited? Open an issue at
<https://github.com/psmyles/3d-review/issues/new>._
