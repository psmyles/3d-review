# Third-party notices

3D Review is a single statically-linked executable - `3d-review.exe` on Windows, the binary inside
`3D Review.app` on macOS. Everything it needs at runtime is compiled into that one file, so the
binary you receive contains code, typefaces and image data from the projects listed below. Their
licenses require that their copyright notices and license terms travel with the binary - this
document, together with the [`../licenses/`](../licenses/) directory, is how they do.

3D Review's own code is MIT licensed, copyright (c) 2026 Chandan Singh; see
[`../LICENSE`](../LICENSE).

* **Section 1** covers what is not a Rust crate: the native C/C++ libraries compiled into the
  binary, the typefaces embedded in it, the shader code ported from other projects, and the
  embedded image assets. Read the typeface note - the font licenses are the only terms in the
  distribution that say anything about *modifying* a component.
* **Section 2** covers the Rust crates (201 of them). One, `option-ext`, is MPL-2.0; the note
  under the table says what that does and does not require.
* **Section 3** notes what is *not* here, and why.

The crate table is derived from `Cargo.lock` (runtime dependencies only, for the union of the
two shipped targets `x86_64-pc-windows-msvc` and `aarch64-apple-darwin` - a crate that appears on
only one of them is still listed, since both binaries are built from this tree). Build-time-only
tooling - `cc`, `sha2`, `winresource`, `sokol-shdc`, `fxc`, `xcrun metal`, the IBL bake's
`intel_tex_2`, ImageMagick - is excluded: it produces the binary but no part of it is linked into
the binary.

---

## 1. Native libraries and embedded assets

### Native libraries

These are C/C++ libraries, vendored as source under `third_party/`, `crates/psd/vendor/` and
`vendor/`, or carried by a `-sys` crate, compiled with `cc` and statically linked.

#### ufbx - MIT (offered as MIT OR Unlicense; 3D Review elects MIT)

*The FBX reader. Every model 3D Review opens goes through it. Version 0.22.0
(`UFBX_HEADER_VERSION`), vendored at `third_party/ufbx/` and compiled by `crates/import/build.rs`
together with the project's own C bridge.*
Copyright (c) 2020 Samuli Raivio - <https://github.com/ufbx/ufbx>
License text: [`licenses/ufbx-MIT-OR-Unlicense.txt`](../licenses/ufbx-MIT-OR-Unlicense.txt)

The vendored `ufbx.c` / `ufbx.h` pair is upstream's `v0.22.0` tag (commit
`ee203dfe62d60c98227288f6d74b9a117857c065`), unmodified; upstream's `LICENSE` sits beside it and
`third_party/ufbx/NOTICE.txt` records the provenance.

#### ufbx_write - MIT (offered as MIT OR Unlicense; 3D Review elects MIT)

*The FBX writer behind the Opt workspace's export. Vendored at `third_party/ufbx-write/` from
upstream commit `2b65caaaa7fada7db9a2d25cd924997838169056` and compiled by
`crates/optimize/build.rs`.*
Copyright (c) 2026 Samuli Raivio - <https://github.com/ufbx/ufbx-write>
License text: [`licenses/ufbx-write-MIT-OR-Unlicense.txt`](../licenses/ufbx-write-MIT-OR-Unlicense.txt)

**This copy is modified.** 3D Review adds five patches (property flags and blob values, the `Null`
node attribute, crease / hole / visibility / polygon-group layers, LOD groups and layered
textures, and curve nodes that carry only the curves asked for). Each hunk is marked `review patch
P<n>` in the source, `third_party/ufbx-write/NOTICE.txt` describes every one, and
`third_party/ufbx-write/review.patch` is the exact diff against the pristine upstream files. The
MIT license does not require modifications to be marked; they are documented anyway.

#### meshoptimizer - MIT

*The mesh operations of the Opt workspace: welding, simplification, LOD generation, and the
vertex-cache / overdraw / vertex-fetch reorders and their metrics. Version 1.3, upstream commit
`9e1f07b159d3cb777f1c67ed31fc11fd117986f4`, vendored unmodified at `third_party/meshoptimizer/`
and compiled by `crates/optimize/build.rs`.*
Copyright (c) 2016-2026 Arseny Kapoulkine - <https://github.com/zeux/meshoptimizer>
License text: [`licenses/meshoptimizer-MIT.txt`](../licenses/meshoptimizer-MIT.txt)

#### psd_sdk - BSD-2-Clause

*The Photoshop `.psd` reader behind PSD textures. Vendored unmodified at `crates/psd/vendor/Psd/`
from upstream commit `f51449543273cbf12058ae92b230e0c4209f5066` and compiled by
`crates/psd/build.rs` with the project's C-ABI wrapper.*
Copyright 2011-2020, Molecular Matters GmbH - <https://github.com/MolecularMatters/psd_sdk>
License text: [`licenses/BSD-2-Clause.txt`](../licenses/BSD-2-Clause.txt)

#### sokol - Zlib

*`sokol_gfx.h`, the one drawing API over Direct3D 11 and Metal, compiled as C into the binary.
Vendored at `vendor/sokol-rust/` (floooh/sokol-rust @ `b22a545`), which carries the sokol
headers and compiles them from its own `build.rs`; only what the renderer references is linked.
The one local change is a `rustfmt.toml` line that switches formatting off - no source is
altered.*
Copyright (c) 2018 Andre Weissflog - <https://github.com/floooh/sokol>
The `sokol-rust` bindings themselves are copyright (c) 2023 Erik Wilhelm Gren, under the same
license - <https://github.com/floooh/sokol-rust>
License text: [`licenses/Zlib.txt`](../licenses/Zlib.txt)

#### Tracy profiler client - BSD-3-Clause

*The `--tracy` CPU and GPU profiler. Tracy 0.14.1, compiled from the C++ sources carried by the
`tracy-client-sys` crate (0.30.0). It is in every build, release included, but inert: the client
never starts unless 3D Review is launched with `--tracy`, and opens no socket otherwise.*
Copyright (c) 2017-2026, Bartosz Taudul - <https://github.com/wolfpld/tracy>
License text: [`licenses/Tracy-BSD-3-Clause.txt`](../licenses/Tracy-BSD-3-Clause.txt)

The Tracy client in turn bundles, each under its own terms:

* **LZ4** - BSD-2-Clause, copyright (C) 2011-2020, Yann Collet
  ([`licenses/BSD-2-Clause.txt`](../licenses/BSD-2-Clause.txt)).
* **moodycamel::ConcurrentQueue** - Simplified BSD (BSD-2-Clause), copyright (c) 2013-2016,
  Cameron Desrochers ([`licenses/BSD-2-Clause.txt`](../licenses/BSD-2-Clause.txt)).
* **SPSCQueue** - MIT, copyright (c) 2020 Erik Rigtorp
  ([`licenses/MIT.txt`](../licenses/MIT.txt)).
* **rpmalloc** - public domain, 2016 Mattias Jansson.
* **libbacktrace** (macOS build only; Windows uses the OS's own DbgHelp) - BSD-3-Clause-style,
  copyright (C) 2012-2016 Free Software Foundation, Inc.
  ([`licenses/BSD-3-Clause.txt`](../licenses/BSD-3-Clause.txt)).

### Typefaces embedded in the binary

3D Review draws its interface in two bundled typefaces, and egui keeps its own four built-in faces
registered behind them as fallbacks for glyphs the first two lack, so all six are compiled into
the binary:

| Typeface | Version | License | Copyright | Embedded by |
|---|---|---|---|---|
| Inter | 4.001 | OFL-1.1 | Copyright 2016 The Inter Project Authors (<https://github.com/rsms/inter>, designer Rasmus Andersson) | `crates/ui` (`assets/fonts/InterVariable.ttf`) |
| JetBrains Mono | 2.304 | OFL-1.1 | Copyright 2020 The JetBrains Mono Project Authors (<https://github.com/JetBrains/JetBrainsMono>, designers Philipp Nurullin, Konstantin Bulenkov) | `crates/ui` (`assets/fonts/JetBrainsMono.ttf`) |
| Ubuntu Light | - | Ubuntu-font-1.0 | Copyright 2011 Canonical Ltd. | `epaint_default_fonts` 0.36.1 |
| Hack | - | MIT, with Bitstream Vera License for the Vera-derived glyphs | Copyright (c) 2018 Source Foundry Authors; Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved. Bitstream Vera is a trademark of Bitstream, Inc. (DejaVu changes are public domain) | `epaint_default_fonts` 0.36.1 |
| Noto Emoji | - | OFL-1.1 | Copyright 2013 Google Inc. All Rights Reserved. | `epaint_default_fonts` 0.36.1 |
| emoji-icon-font | - | MIT | Copyright (c) 2014 John Slegers | `epaint_default_fonts` 0.36.1 |

License texts: [`licenses/OFL-1.1.txt`](../licenses/OFL-1.1.txt),
[`licenses/Ubuntu-font-1.0.txt`](../licenses/Ubuntu-font-1.0.txt),
[`licenses/MIT.txt`](../licenses/MIT.txt),
[`licenses/Bitstream-Vera.txt`](../licenses/Bitstream-Vera.txt).

> #### What the font licenses ask
>
> The SIL Open Font License 1.1, the Ubuntu Font Licence 1.0 and the Bitstream Vera License all
> permit bundling a font with software and redistributing it, including commercially, on three
> conditions that matter here:
>
> * **The copyright notice and license must travel with the font** - which is what the table
>   above and the texts in `licenses/` do.
> * **A font may not be sold by itself** (OFL condition 1, and the Bitstream Vera License). 3D
>   Review never offers the fonts separately from the program.
> * **A modified font must be renamed** (Reserved Font Names under the OFL, the "Bitstream" /
>   "Vera" names, the naming rules of the UFL) and stays under the same license. 3D Review embeds
>   every font file byte for byte as published; nothing is subset, renamed or altered.
>
> These are copyleft terms *for the fonts only*. Both the OFL and the UFL expressly allow the fonts
> to be bundled and embedded with any software; their requirement to stay under the same license
> applies to the fonts and fonts derived from them, not to the program they are embedded in.

### Shader code ported from other projects

`crates/render/src/shaders/review.glsl` is 3D Review's own shader source, but parts of it are
translations of other projects' shader code rather than re-implementations from a paper, so their
licenses apply to those parts. Each ported function says so in a comment beside it.

* **XeGTAO** - Intel's ground-truth ambient occlusion. The occlusion pass (`fs_gtao`) is a GLSL
  translation of `XeGTAO_MainPass` without its bent normals - the horizon search, the depth-MIP
  sampling offset, the smooth radius falloff, the thin-occluder compensation and the arc
  integration follow it line for line - and the depth prefilter (`fs_gtao_depth_mip`) is a
  translation of `XeGTAO_DepthMIPFilter`. The denoiser and the temporal accumulation are 3D
  Review's own. MIT, Copyright (C) 2016-2021, Intel Corporation -
  <https://github.com/GameTechDev/XeGTAO>
  ([`licenses/MIT.txt`](../licenses/MIT.txt)).
* **Khronos PBR Neutral** - the default tone mapper (`pbr_neutral_tonemap`) is a port of the
  Khronos Group's reference `PBR_Neutral/pbrNeutral.glsl`, unchanged apart from names and
  formatting. Apache-2.0, Copyright 2024 The Khronos Group, Inc. -
  <https://github.com/KhronosGroup/ToneMapping>
  ([`licenses/Apache-2.0.txt`](../licenses/Apache-2.0.txt); the repository carries no `NOTICE`
  file).
* **three.js** - the ACES Filmic (`aces_tonemap`) and AgX (`agx_tonemap`) operators are ports of
  three.js's `ACESFilmicToneMapping` and `AgXToneMapping`, with its matrices, constants and its
  1/0.6 exposure lift for ACES. MIT, Copyright (c) 2010-2026 three.js authors -
  <https://github.com/mrdoob/three.js>
  ([`licenses/MIT.txt`](../licenses/MIT.txt)). three.js credits the work these rest on, and so
  does this document: the ACES curve is Stephen Hill's RRT + ODT fit as published in MJP's
  BakingLab (<https://github.com/TheRealMJP/BakingLab>, MIT); AgX is Troy Sobotka's, by way of
  Blender and Google Filament's implementation, with Benjamin Wrensch's polynomial fit of its
  contrast curve.
* **GTAO spatial and temporal patterns** - Jorge Jimenez et al., "Practical Real-Time Strategies
  for Accurate Indirect Occlusion" (Activision, 2016). Implemented from the paper.

### Embedded image assets

* **Toolbar and node icons** (`assets/icons/*.png`), **the application logo**
  (`assets/icons/application-logo.png` / `.ico`) - original to 3D Review, covered by its MIT
  license.
* **UV checker textures** (`assets/textures/T_UV_Checker_BW.png`, `T_UV_Checker_CLR.png`),
  embedded by `crates/render` - original to 3D Review, covered by its MIT license.
* **Lighting environments** - six equirectangular Radiance HDR images
  (`assets/textures/T_HDR_01.hdr` ... `T_HDR_06.hdr`) from **Poly Haven**
  (<https://polyhaven.com>), released under **CC0** (<https://polyhaven.com/license>). The HDRs
  themselves are not shipped, but what is derived from them is embedded in the binary: the BC6H
  environment, irradiance and prefiltered cube maps baked from them
  (`assets/ibl_baked/T_IBL_0*_*.bin`, embedded by `crates/render`) and the preview thumbnails in
  the Environment dropdown (`assets/thumbnails/T_HDR_0*.png`, embedded by `crates/ui`). CC0 is a
  public-domain dedication and asks for no attribution; Poly Haven is credited anyway. Full text:
  [`licenses/CC0-1.0.txt`](../licenses/CC0-1.0.txt). (The shared BRDF lookup table,
  `T_IBL_BRDF.bin`, is computed by the bake from first principles and has no external source.)

---

## 2. Rust crates

201 crates are linked into the 3D Review binary (the union of both shipped targets). Where a crate
offers a choice of licenses, the column below records **the license 3D Review elects**, not the
full SPDX expression - 3D Review elects MIT wherever MIT is offered, then the most permissive of
what is left. A crate whose license is a conjunction (`AND`) lists every term. Full texts:

| License | Text |
|---|---|
| MIT (168 crates, plus 5 where it is combined with another term) | [`licenses/MIT.txt`](../licenses/MIT.txt) |
| Unicode-3.0 (18, plus 1 in addition to MIT) | [`licenses/Unicode-3.0.txt`](../licenses/Unicode-3.0.txt) |
| Apache-2.0 (3, plus 1 in addition to MIT) | [`licenses/Apache-2.0.txt`](../licenses/Apache-2.0.txt) |
| BSD-3-Clause (2, plus 1 in addition to MIT) | [`licenses/BSD-3-Clause.txt`](../licenses/BSD-3-Clause.txt); Tracy's own text is [`licenses/Tracy-BSD-3-Clause.txt`](../licenses/Tracy-BSD-3-Clause.txt) |
| BSL-1.0 (2) | [`licenses/BSL-1.0.txt`](../licenses/BSL-1.0.txt) |
| Zlib (1) | [`licenses/Zlib.txt`](../licenses/Zlib.txt) |
| MPL-2.0 (1) | [`licenses/MPL-2.0.txt`](../licenses/MPL-2.0.txt) - see the note below |
| CC0-1.0 (1) | [`licenses/CC0-1.0.txt`](../licenses/CC0-1.0.txt) |
| OFL-1.1 and Ubuntu-font-1.0 (1, in addition to MIT) | the typefaces - see section 1 |
| IJG (1, in addition to MIT) | see the note below the table |

Some crates ship a license file with no copyright line filled in, or no license file at all.
Rather than invent one, those rows name the authors the crate itself declares.

| Crate | Version | License | Copyright |
|---|---|---|---|
| `accesskit` | 0.24.1 | MIT | (no notice in crate; authors: The AccessKit contributors) |
| `adler2` | 2.0.1 | MIT | Copyright (C) Jonas Schievink <jonasschievink@gmail.com> |
| `ahash` | 0.8.12 | MIT | Copyright (c) 2018 Tom Kaitchuck |
| `anyhow` | 1.0.103 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `arboard` | 3.6.1 | MIT | Copyright (c) 2022 The Arboard contributors |
| `arrayvec` | 0.7.7 | MIT | Copyright (c) Ulrik Sverdrup "bluss" 2015-2023 |
| `bitflags` | 1.3.2 | MIT | Copyright (c) 2014 The Rust Project Developers |
| `bitflags` | 2.13.0 | MIT | Copyright (c) 2014 The Rust Project Developers |
| `block2` | 0.5.1 | MIT | (no notice in crate; authors: Steven Sheldon, Mads Marquart <mads@marquart.dk>) |
| `block2` | 0.6.2 | MIT | (no notice in crate; authors: Mads Marquart <mads@marquart.dk>) |
| `bytemuck` | 1.25.0 | MIT | Copyright (c) 2019 Daniel "Lokathor" Gee. |
| `bytemuck_derive` | 1.10.2 | MIT | Copyright (c) 2019 Daniel "Lokathor" Gee. |
| `byteorder-lite` | 0.1.0 | MIT | Copyright (c) 2015 Andrew Gallant |
| `cfg-if` | 1.0.4 | MIT | Copyright (c) 2014 Alex Crichton |
| `clipboard-win` | 5.4.1 | BSL-1.0 | (no notice in crate; authors: Douman <douman@gmx.se>) |
| `color` | 0.3.3 | MIT | (no copyright notice supplied by the crate) |
| `core-foundation` | 0.9.4 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `core-foundation` | 0.10.1 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `core-foundation-sys` | 0.8.7 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `core-graphics` | 0.23.2 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `core-graphics-types` | 0.1.3 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `crc32fast` | 1.5.0 | MIT | Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors |
| `crossbeam-channel` | 0.5.16 | MIT | Copyright (c) 2019 The Crossbeam Project Developers |
| `crossbeam-utils` | 0.8.21 | MIT | Copyright (c) 2019 The Crossbeam Project Developers |
| `cursor-icon` | 1.2.0 | MIT | Copyright (c) 2023 Kirill Chibisov |
| `dirs` | 6.0.0 | MIT | Copyright (c) 2018-2019 dirs-rs contributors |
| `dirs-sys` | 0.5.0 | MIT | Copyright (c) 2018-2019 dirs-rs contributors |
| `dispatch` | 0.2.0 | MIT | (no notice in crate; authors: Steven Sheldon) |
| `dispatch2` | 0.3.1 | MIT | (no notice in crate; authors: Mads Marquart <mads@marquart.dk>, Mary <mary@mary.zone>) |
| `displaydoc` | 0.2.6 | MIT | (no notice in crate; authors: Jane Lusby <jlusby@yaah.dev>) |
| `dpi` | 0.1.2 | Apache-2.0 AND MIT | Apache-2.0 part: (no notice in crate; the winit contributors). MIT part, a port of libm: Copyright (c) 2018 Jorge Aparicio; Copyright © 2005-2020 Rich Felker, et al. (and the further holders listed in the crate's `LICENSE-LIBM-MIT`) |
| `ecolor` | 0.36.1 | MIT | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>, Andreas Reich <reichandreas@gmx.de>) |
| `egui` | 0.36.1 | MIT | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>) |
| `egui-winit` | 0.36.1 | MIT | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>) |
| `egui_commonmark` | 0.25.0 | MIT | Copyright (c) 2022-2025 Erlend Walstad |
| `egui_commonmark_backend` | 0.25.0 | MIT | Copyright (c) 2022-2025 Erlend Walstad |
| `egui_extras` | 0.36.1 | MIT | (no notice in crate; authors: Dominik Rössler <dominik@freshx.de>, Emil Ernerfeldt <emil.ernerfeldt@gmail.com>, René Rössler <rene@freshx.de>) |
| `either` | 1.18.0 | MIT | Copyright (c) 2015 |
| `emath` | 0.36.1 | MIT | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>) |
| `enum-map` | 2.7.3 | MIT | (no notice in crate; authors: Kamila Borowska <kamila@borowska.pw>) |
| `enum-map-derive` | 0.17.0 | MIT | (no notice in crate; authors: Kamila Borowska <kamila@borowska.pw>) |
| `epaint` | 0.36.1 | MIT | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>) |
| `epaint_default_fonts` | 0.36.1 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 | (no notice in crate; authors: Emil Ernerfeldt <emil.ernerfeldt@gmail.com>); bundles four fonts under their own licenses (see section 1) |
| `error-code` | 3.3.2 | BSL-1.0 | (no notice in crate; authors: Douman <douman@gmx.se>) |
| `euclid` | 0.22.14 | MIT | Copyright (c) 2012-2013 Mozilla Foundation |
| `fax` | 0.2.7 | MIT | Copyright © 2021 The pdf-rs contributers. |
| `fdeflate` | 0.3.7 | MIT | (no notice in crate; authors: The image-rs Developers) |
| `fearless_simd` | 0.4.1 | MIT | Copyright (c) 2018 Raph Levien |
| `flate2` | 1.1.9 | MIT | Copyright (c) 2014-2026 Alex Crichton |
| `fluent-bundle` | 0.16.0 | MIT | Copyright 2017 Mozilla |
| `fluent-langneg` | 0.13.1 | MIT | (no notice in crate; authors: Zibi Braniecki <gandalf@mozilla.com>) |
| `fluent-syntax` | 0.12.0 | MIT | Copyright 2017 Mozilla |
| `foldhash` | 0.2.0 | Zlib | Copyright (c) 2024 Orson Peters |
| `font-types` | 0.12.4 | MIT | Copyright (c) 2019 Fontations Developers |
| `foreign-types` | 0.5.0 | MIT | Copyright (c) 2017 The foreign-types Developers |
| `foreign-types-macros` | 0.2.3 | MIT | Copyright (c) 2017 The foreign-types Developers |
| `foreign-types-shared` | 0.3.1 | MIT | Copyright (c) 2017 The foreign-types Developers |
| `form_urlencoded` | 1.2.2 | MIT | Copyright (c) 2013-2016 The rust-url developers |
| `fsevent-sys` | 4.1.0 | MIT | Copyright (c) 2015 Pierre Baillet |
| `glam` | 0.30.10 | MIT | (no notice in crate; authors: Cameron Hart <cameron.hart@gmail.com>) |
| `guillotiere` | 0.7.0 | MIT | Copyright (c) 2019 Nicolas Silva |
| `half` | 2.7.1 | MIT | (no notice in crate; authors: Kathryn Long <squeeself@gmail.com>) |
| `harfrust` | 0.12.0 | MIT | Copyright (c) HarfBuzz developers; Copyright (c) 2020 Yevhenii Reizner |
| `hashbrown` | 0.17.1 | MIT | Copyright (c) 2016 Amanieu d'Antras |
| `icu_collections` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_locale_core` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_normalizer` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_normalizer_data` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_properties` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_properties_data` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `icu_provider` | 2.2.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `idna` | 1.1.0 | MIT | Copyright (c) 2013-2025 The rust-url developers |
| `idna_adapter` | 1.2.2 | MIT | Copyright (c) The rust-url developers |
| `image` | 0.25.10 | MIT | (no notice in crate; authors: The image-rs Developers) |
| `intl-memoizer` | 0.5.3 | MIT | Copyright 2017 Mozilla |
| `intl_pluralrules` | 7.0.2 | MIT | (no notice in crate; authors: Kekoa Riggin <kekoariggin@gmail.com>, Zibi Braniecki <zbraniecki@mozilla.com>) |
| `itertools` | 0.15.0 | MIT | Copyright (c) 2015 |
| `itoa` | 1.0.18 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `jpeg-encoder` | 0.7.0 | MIT AND IJG | Copyright (c) 2021 Volker Ströbel <volkerstroebel@mysurdity.de> |
| `keyboard-types` | 0.7.0 | MIT | Copyright (c) 2017 Pyfisch |
| `kurbo` | 0.13.1 | MIT | Copyright (c) 2018 Raph Levien |
| `libc` | 0.2.186 | MIT | Copyright (c) The Rust Project Developers |
| `linebender_resource_handle` | 0.1.1 | MIT | (no copyright notice supplied by the crate) |
| `litemap` | 0.8.2 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `lock_api` | 0.4.14 | MIT | Copyright (c) 2016 The Rust Project Developers |
| `log` | 0.4.33 | MIT | Copyright (c) 2014 The Rust Project Developers |
| `memchr` | 2.8.2 | MIT | Copyright (c) 2015 Andrew Gallant |
| `mime` | 0.3.17 | MIT | Copyright (c) 2014 Sean McArthur |
| `mime_guess2` | 2.3.1 | MIT | Copyright (c) 2015 Austin Bonander |
| `miniz_oxide` | 0.8.9 | MIT | Copyright 2013-2014 RAD Game Tools and Valve Software; Copyright 2010-2014 Rich Geldreich and Tenacious Software LLC; Copyright (c) 2017 Frommi; Copyright (c) 2017-2024 oyvindln |
| `moxcms` | 0.8.1 | BSD-3-Clause | Copyright (c) Radzivon Bartoshyk. All rights reserved. |
| `muda` | 0.19.3 | MIT | Copyright (c) 2022-2022 Tauri Programme within The Commons Conservancy |
| `nohash-hasher` | 0.2.0 | MIT | Copyright 2018 Parity Technologies (UK) Ltd. |
| `notify` | 8.2.0 | CC0-1.0 | (no notice in crate; authors: Félix Saparelli <me@passcod.name>, Daniel Faust <hessijames@gmail.com>, Aron Heinecke <Ox0p54r36@t-online.de>) |
| `notify-types` | 2.1.0 | MIT | Copyright (c) 2023 Notify Contributors |
| `num-traits` | 0.2.19 | MIT | Copyright (c) 2014 The Rust Project Developers |
| `objc-sys` | 0.3.5 | MIT | (no notice in crate; authors: Mads Marquart <mads@marquart.dk>) |
| `objc2` | 0.5.2 | MIT | (no notice in crate; authors: Steven Sheldon, Mads Marquart <mads@marquart.dk>) |
| `objc2` | 0.6.4 | MIT | (no notice in crate; authors: Mads Marquart <mads@marquart.dk>) |
| `objc2-app-kit` | 0.2.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-app-kit` | 0.3.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-core-foundation` | 0.3.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-core-graphics` | 0.3.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-encode` | 4.1.0 | MIT | (no notice in crate; authors: Mads Marquart <mads@marquart.dk>) |
| `objc2-foundation` | 0.2.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-foundation` | 0.3.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-metal` | 0.2.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `objc2-quartz-core` | 0.2.2 | MIT | (no notice in crate; part of the objc2 project - Mads Marquart and contributors) |
| `once_cell` | 1.21.4 | MIT | (no notice in crate; authors: Aleksey Kladov <aleksey.kladov@gmail.com>) |
| `option-ext` | 0.2.0 | MPL-2.0 | (no notice in crate; authors: Simon Ochsenreither <simon@ochsenreither.de>) |
| `parking_lot` | 0.12.5 | MIT | Copyright (c) 2016 The Rust Project Developers |
| `parking_lot_core` | 0.9.12 | MIT | Copyright (c) 2016 The Rust Project Developers |
| `peniko` | 0.6.1 | MIT | Copyright (c) 2018 Raph Levien |
| `percent-encoding` | 2.3.2 | MIT | Copyright (c) 2013-2025 The rust-url developers |
| `pin-project-lite` | 0.2.17 | MIT | (no copyright notice supplied by the crate) |
| `png` | 0.18.1 | MIT | Copyright (c) 2015 nwin |
| `polycool` | 0.4.0 | MIT | Copyright (c) 2018 Raph Levien |
| `potential_utf` | 0.1.5 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `proc-macro2` | 1.0.106 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>) |
| `profiling` | 1.0.18 | MIT | (no notice in crate; authors: Philip Degarmo <aclysma@gmail.com>) |
| `pulldown-cmark` | 0.13.4 | MIT | Copyright 2015 Google Inc. All rights reserved. |
| `pxfm` | 0.1.29 | BSD-3-Clause | Copyright (c) Radzivon Bartoshyk. All rights reserved. |
| `quick-error` | 2.0.1 | MIT | Copyright (c) 2015 The quick-error Developers |
| `quote` | 1.0.46 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `raw-window-handle` | 0.6.2 | MIT | Copyright (c) 2019 Osspial |
| `read-fonts` | 0.41.0 | MIT | Copyright (c) 2019 Fontations Developers |
| `rfd` | 0.15.4 | MIT | Copyright (c) 2022 Bartłomiej Maryńczak |
| `rustc-hash` | 2.1.3 | MIT | (no notice in crate; authors: The Rust Project Developers) |
| `same-file` | 1.0.6 | MIT | Copyright (c) 2017 Andrew Gallant |
| `scopeguard` | 1.2.0 | MIT | Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers |
| `self_cell` | 1.3.0 | Apache-2.0 | (no notice in crate; authors: Lukas Bergdoll <lukas.bergdoll@gmail.com>) |
| `serde` | 1.0.228 | MIT | (no notice in crate; authors: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>) |
| `serde_core` | 1.0.228 | MIT | (no notice in crate; authors: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>) |
| `serde_derive` | 1.0.228 | MIT | (no notice in crate; authors: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>) |
| `serde_json` | 1.0.150 | MIT | (no notice in crate; authors: Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>) |
| `simd-adler32` | 0.3.9 | MIT | Copyright (c) [2021] [Marvin Countryman] |
| `skrifa` | 0.44.0 | MIT | Copyright (c) 2019 Fontations Developers |
| `smallvec` | 1.15.2 | MIT | Copyright (c) 2018 The Servo Project Developers |
| `smol_str` | 0.2.2 | MIT | (no notice in crate; authors: Aleksey Kladov <aleksey.kladov@gmail.com>) |
| `stable_deref_trait` | 1.2.1 | MIT | Copyright (c) 2017 Robert Grosse |
| `syn` | 2.0.118 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `synstructure` | 0.13.2 | MIT | Copyright 2016 Nika Layzell |
| `sys-locale` | 0.3.2 | MIT | Copyright (c) 2021 1Password |
| `thiserror` | 2.0.18 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `thiserror-impl` | 2.0.18 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `tiff` | 0.11.3 | MIT | Copyright (c) 2018 PistonDevelopers |
| `tinystr` | 0.8.3 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `tracing` | 0.1.44 | MIT | Copyright (c) 2019 Tokio Contributors |
| `tracing-core` | 0.1.36 | MIT | Copyright (c) 2019 Tokio Contributors |
| `tracy-client` | 0.19.0 | MIT | (no notice in crate; authors: Simonas Kazlauskas <tracy-client@kazlauskas.me>) |
| `tracy-client-sys` | 0.30.0 | MIT AND BSD-3-Clause | (no notice in crate; authors: Simonas Kazlauskas <tracy-client-sys@kazlauskas.me>); bundles the Tracy client, Copyright (c) 2017-2026 Bartosz Taudul (see section 1) |
| `type-map` | 0.5.1 | MIT | Copyright (c) 2022 Jacob Brown; Copyright (c) 2017-NOW Actix Team |
| `unic-langid` | 0.9.6 | MIT | (no notice in crate; authors: Zibi Braniecki <gandalf@mozilla.com>) |
| `unic-langid-impl` | 0.9.6 | MIT | (no notice in crate; authors: Zibi Braniecki <gandalf@mozilla.com>) |
| `unicase` | 2.9.0 | MIT | Copyright (c) 2014-2026 Sean McArthur |
| `unicode-general-category` | 1.1.0 | Apache-2.0 | (no notice in crate; authors: YesLogic Pty. Ltd. <info@yeslogic.com>) |
| `unicode-ident` | 1.0.24 | MIT AND Unicode-3.0 | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `unicode-segmentation` | 1.13.3 | MIT | Copyright (c) 2015 The Rust Project Developers |
| `url` | 2.5.8 | MIT | Copyright (c) 2013-2025 The rust-url developers |
| `utf8_iter` | 1.0.4 | MIT | Copyright Mozilla Foundation |
| `uuid` | 1.23.4 | MIT | Copyright (c) 2014 The Rust Project Developers; Copyright (c) 2018 Ashley Mannix, Christopher Armstrong, Dylan DPC, Hunar Roop Kahlon |
| `vello_common` | 0.1.0 | MIT | Copyright 2020 the Vello Authors |
| `vello_cpu` | 0.1.0 | MIT | Copyright 2020 the Vello Authors |
| `walkdir` | 2.5.0 | MIT | Copyright (c) 2015 Andrew Gallant |
| `web-time` | 1.1.0 | MIT | Copyright (c) 2023 dAxpeDDa |
| `webbrowser` | 1.2.1 | MIT | Copyright (c) 2015-2022 Amod Malviya |
| `weezl` | 0.1.12 | MIT | Copyright (c) HeroicKatora 2020 |
| `winapi-util` | 0.1.11 | MIT | Copyright (c) 2017 Andrew Gallant |
| `windows` | 0.62.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-collections` | 0.3.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-core` | 0.62.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-future` | 0.3.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-implement` | 0.60.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-interface` | 0.59.3 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-link` | 0.2.1 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-numerics` | 0.3.1 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-result` | 0.4.1 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-strings` | 0.5.1 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-sys` | 0.52.0 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-sys` | 0.59.0 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-sys` | 0.60.2 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-targets` | 0.52.6 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-targets` | 0.53.5 | MIT | Copyright (c) Microsoft Corporation. |
| `windows-threading` | 0.2.1 | MIT | Copyright (c) Microsoft Corporation. |
| `windows_x86_64_msvc` | 0.52.6 | MIT | Copyright (c) Microsoft Corporation. |
| `windows_x86_64_msvc` | 0.53.1 | MIT | Copyright (c) Microsoft Corporation. |
| `winit` | 0.30.13 | Apache-2.0 | (no notice in crate; authors: The winit contributors, Pierre Krieger <pierre.krieger1708@gmail.com>) |
| `writeable` | 0.6.3 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `yoke` | 0.8.3 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `yoke-derive` | 0.8.2 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zerocopy` | 0.8.52 | MIT | Copyright 2023 The Fuchsia Authors |
| `zerocopy-derive` | 0.8.52 | MIT | Copyright 2023 The Fuchsia Authors |
| `zerofrom` | 0.1.8 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zerofrom-derive` | 0.1.7 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zerotrie` | 0.2.4 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zerovec` | 0.11.6 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zerovec-derive` | 0.11.3 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. |
| `zmij` | 1.0.21 | MIT | (no notice in crate; authors: David Tolnay <dtolnay@gmail.com>) |
| `zune-core` | 0.5.1 | MIT | Copyright (c) zune-image developers |
| `zune-image` | 0.5.0 | MIT | Copyright (c) zune-image developers |
| `zune-jpeg` | 0.5.15 | MIT | Copyright (c) zune-image developers |

### Note on `epaint_default_fonts` (OFL-1.1, Ubuntu-font-1.0)

`epaint_default_fonts` is licensed `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`: the code
is MIT, and the four fonts it embeds keep their own licenses. They are listed, with their
copyright lines, in section 1's typeface table, and what their licenses ask is set out there.

### Note on `tracy-client-sys` (BSD-3-Clause)

`tracy-client-sys` is licensed `(MIT OR Apache-2.0) AND BSD-3-Clause`: the Rust bindings are MIT,
and the Tracy C++ client they compile is BSD-3-Clause. Tracy and the libraries it bundles are
listed in section 1.

### Note on `jpeg-encoder` (IJG)

`jpeg-encoder` is licensed `(MIT OR Apache-2.0) AND IJG` - the second term covers code derived
from the Independent JPEG Group's libjpeg. The crate ships no separate IJG text; the applicable
terms are those of the IJG distribution, which permit use, modification and redistribution
provided the origin is not misrepresented and any changes are marked. The crate is pulled in by
`image`'s `jpeg` feature. 3D Review only ever decodes JPEG (through `zune-jpeg`) and never encodes
one, so this code is linked but not reached.

### Note on the Boost-licensed crates (BSL-1.0)

`clipboard-win` and `error-code` - Windows only, reached through `arboard`, the clipboard behind
egui's copy and paste - are under the Boost Software License 1.0. It requires its notice to be
kept with source copies and waives that requirement for copies "solely in the form of
machine-executable object code". They are listed here for completeness. Full text:
[`licenses/BSL-1.0.txt`](../licenses/BSL-1.0.txt).

### Note on the Apache-2.0 crates

`winit`, `self_cell` and `unicode-general-category` are Apache-2.0 only, and `dpi` is Apache-2.0
together with MIT. Apache-2.0 section 4 asks that the license text accompany the binary (it does:
[`licenses/Apache-2.0.txt`](../licenses/Apache-2.0.txt)) and that any `NOTICE` file the work
carries be reproduced; none of these four crates ships one. The ported Khronos PBR Neutral
tone mapper in section 1 is under the same license and terms, and its repository ships no `NOTICE`
either. `self_cell` is offered as
`Apache-2.0 OR GPL-2.0-only`; 3D Review elects Apache-2.0, so no GPL term applies to it.

### Note on `notify` (CC0-1.0)

CC0-1.0 is a public-domain dedication and imposes no attribution requirement. `notify` is the file
watcher that reloads a texture when it is saved; it is listed here for completeness. Full text:
[`licenses/CC0-1.0.txt`](../licenses/CC0-1.0.txt).

### Note on `option-ext` (MPL-2.0)

`option-ext` - a small `Option` extension trait, reached through `dirs` -> `dirs-sys` - is the
one Rust crate under a copyleft license. MPL-2.0 is *file-level* weak copyleft: it obliges
whoever distributes a binary containing those files to make **their source** available, and it
explicitly permits linking them into a larger work under any license (section 3.3). 3D Review
uses the crate unmodified, so the corresponding source is the published crate itself
(<https://github.com/soc/option-ext>, `option-ext 0.2.0` on crates.io). Nothing else in 3D Review
is affected. Full text: [`licenses/MPL-2.0.txt`](../licenses/MPL-2.0.txt).

---

## 3. What is not in this document

* **Test fixtures and sample data.** The FBX models under `assets/test_models/` and the images
  under `assets/test_textures/` are used by the test suites and for manual checks. They are not
  compiled into the binary and are not installed.
* **The source HDR images.** `assets/textures/T_HDR_*.hdr` are inputs to the offline IBL bake
  and are not shipped; what *is* shipped from them - the baked cube maps and the thumbnails - is
  covered in section 1.
* **Icons, artwork and the manual.** The toolbar and node icons in `assets/icons/`, the
  application logo, the message catalogs under `crates/localization/locales/` and the manual under
  `docs/book/` (whose text is embedded in the binary and whose screenshots are installed beside
  it) are original to 3D Review and covered by its own MIT license.
* **The operating systems' own APIs.** On Windows, 3D Review links `d3d11`, `dxgi`, `user32`,
  `kernel32`, `ole32`, `shell32`, `dbghelp` and friends from the Windows SDK. On macOS it links the
  `AppKit`, `Foundation`, `QuartzCore` and `Metal` frameworks. These are operating-system
  components licensed to the user by Microsoft and by Apple as part of the OS, not redistributed
  by 3D Review.
* **Build-time tooling.** `cc` (compiles the vendored C/C++), `sha2` (the shader-bytecode
  manifest), `winresource` (the Windows version resource and icon), `sokol-shdc` (generates the
  per-backend shader sources), `fxc.exe` and `xcrun metal` (compile them to bytecode offline) run
  during the build and contribute no code to the binary. The offline IBL bake (`bake_ibl`, behind
  `review-render`'s `bake` feature) additionally uses `intel_tex_2` and `ispc_rt` - the Intel ISPC
  Texture Compressor's BC6H encoder - on a developer machine; its output is the `.bin` maps, and
  none of its code is in the shipped binary. ImageMagick (vendored as a binary under
  `packaging/magick/`, with its own `LICENSE.txt` and `NOTICE.txt`) generates the HDR thumbnails
  and the application icons at packaging time and is not installed. The manual's website is built
  with mdBook. Inno Setup builds the Windows installer; its license permits distributing the
  installers it produces without attribution. The macOS `.dmg` is built with Apple's own
  `codesign` / `notarytool` / `hdiutil`.
* **Rust and its standard library** (MIT OR Apache-2.0, (c) The Rust Project Developers),
  portions of which are linked into every Rust binary.

---

## Regenerating this document

The crate table above is derived from `Cargo.lock` plus the license files in the local cargo
registry. Re-derive it after a dependency change with:

```sh
cargo tree -p review-app -e normal --target x86_64-pc-windows-msvc --prefix none --format "{p}"
cargo tree -p review-app -e normal --target aarch64-apple-darwin   --prefix none --format "{p}"
```

Take the **union** of the two - both targets ship - and cross-reference
`cargo metadata --format-version 1` for each package's `license` field, then read the copyright
line out of each package's own `LICENSE*` file in the local cargo registry
(`~/.cargo/registry/src/*/<name>-<version>/`). The workspace's own crates (`review-app`,
`review-import`, `review-localization`, `review-model`, `review-optimize`, `review-prof`,
`review-psd`, `review-render`, `review-shell-macos`, `review-ui`) and the vendored `sokol` path
dependency are excluded from the table: the first ten are 3D Review, and sokol is section 1. Note
that the macOS target pulls in crates the Windows one does not (the `objc2` family, `muda`,
`core-foundation`, `fsevent-sys`, ...) and vice versa (the `windows` family, `clipboard-win`,
`winapi-util`), which is why both trees are needed.

The native libraries in section 1 change only when `third_party/`, `crates/psd/vendor/`,
`vendor/sokol-rust/` or the `tracy-client` pin changes; their versions and commits are recorded in
the `NOTICE.txt` beside each. The typeface table changes when `assets/fonts/` or the egui version
changes (`epaint_default_fonts`). The `licenses/` directory holds one text per license referenced
here; when a new license appears in the table, add its text there.
