- compression option at export time?
- generate UVs - xatlas?
- generate colliders?
- UV view during opt mode?
- viewport presets
- Finder thumbnails + Quick Look preview (the macOS half of the Explorer
  extension, `docs/ARCHITECTURE.md` D23). The shared core is done
  (`crates/preview`: load from a path or bytes, `Host`/`Viewport`, thumbnail,
  `Interaction`); what is left is Mac-only and needs the Mac to build and test:
  1. `rhi/backend/metal.rs`: `Swapchain::offscreen` + `read_back` (both stubbed
     with "not implemented on Metal yet") - a private render-target texture and
     the same blit-to-shared-buffer `read_image_subresource` already does.
     `Gpu::read_back` promises RGBA rows, so swizzle if the target is BGRA.
  2. Rebuild the Metal bytecode: `fs_post` changed (transparent background), and
     only a Mac can compile the `.metallib`; commit it with `bytecode.manifest`.
  3. Declare an FBX UTI in the app's Info.plist (`UTImportedTypeDeclarations`,
     e.g. `com.autodesk.fbx`, extension `fbx`, conforming to `public.data` /
     `public.3d-content`) and name it in `CFBundleDocumentTypes` - Quick Look
     extensions register by UTI, and there is none today.
  4. A thin C-ABI or objc2 layer over `review-preview` (a thumbnail call; a
     preview session: open, size, pointer events, render, close), mirroring
     `crates/shell-windows` without COM.
  5. Two app extensions in `Contents/PlugIns/`: a `QLThumbnailProvider` and a
     view-based `QLPreviewingController` (an NSView blitting the read-back
     frames, mouse and scroll into `Interaction`). Built by `scripts/build-mac.sh`
     with `swiftc`/`clang` (no Xcode project), sandboxed
     (`com.apple.security.app-sandbox`), signed with the Developer ID before the
     app is re-signed and notarized.
  6. Sandbox and sibling textures: an extension can read the file it was handed
     and nothing beside it. Decide on the Mac whether to add a read-only
     temporary-exception entitlement (allowed for Developer ID distribution, not
     the App Store) or ship embedded-and-absolute-path textures only, as the
     Windows thumbnail does.
