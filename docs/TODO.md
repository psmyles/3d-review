- compression option at export time?
- generate UVs - xatlas?
- generate colliders?
- UV view during opt mode?
- viewport presets
-
- Finder thumbnails + Quick Look preview (the macOS half of D23). Built:
  Metal offscreen + readback, the Metal `fs_post` bytecode, the FBX UTI and
  document type in both bundles, `crates/shell-quicklook` (C ABI over
  `review_preview::{shared, Session}`) and its two Swift extensions, bundled and
  signed by `scripts/build-quicklook.sh`. `scripts/quicklook-check.sh` passes, and
  `qlmanage -p` shows our preview. Left:
  1. Thumbnails through Finder: `ThumbnailsAgent` still launches SceneKit's
     thumbnailer for `.fbx` (GOTCHAS, "keeps the extension list it started
     with"). Log out and in, then `qlmanage -t`; if it still picks SceneKit,
     find out why it prefers a `public.3d-content` extension over an exact type.
  2. Sibling textures: sandboxed, the extensions read only the file Quick Look
     hands over (measured: `EPERM` on the texture). Decide whether to add the
     read-only temporary exception (`QuickLook.entitlements`; Developer ID only);
     the manual's "What Finder can show" says what happens without it.
  3. A signed run of `scripts/build-mac.sh` (Developer ID + notarization) with
     the extensions inside: not run yet.
  4. Move `shell-windows` onto `review_preview::shared` / `Session` (its
     `host.rs` and `preview/pane.rs` are the same logic in `HRESULT` shape), and
     test it on Windows.
