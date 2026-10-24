// The Quick Look thumbnail extension: Finder's icon-view, column and Get Info
// thumbnails of an .fbx, drawn by libreview_quicklook (crates/shell-quicklook).
//
// All this file does is the AppKit/Quick Look half: turn the request's size
// into pixels, ask the dylib for a straight-alpha RGBA image, and draw it into
// the reply's context. The scene, the textures and the GPU are the dylib's.

import AppKit
import QuickLookThumbnailing
import os

// Named for the Objective-C runtime, so the extension's Info.plist can name the
// principal class without a Swift module prefix.
@objc(ReviewThumbnailProvider)
final class ThumbnailProvider: QLThumbnailProvider {
    override func provideThumbnail(
        for request: QLFileThumbnailRequest,
        _ handler: @escaping (QLThumbnailReply?, Error?) -> Void
    ) {
        // `maximumSize` is in points; the image is drawn at the display's scale.
        let scale = max(request.scale, 1)
        let side = UInt32(
            (max(request.maximumSize.width, request.maximumSize.height) * scale).rounded(.up))
        guard side > 0, let image = thumbnail(of: request.fileURL, side: side) else {
            // Finder keeps the file's ordinary icon.
            handler(nil, CocoaError(.fileReadCorruptFile))
            return
        }
        // The whole square tile Quick Look asked for, as its header recommends, drawn
        // through AppKit's current context: the variant Xcode's own template uses,
        // set up in points at the request's scale. The CGContext variant was not:
        // drawn into its `contextSize` the model came out a quarter of the tile, in
        // its bottom-left corner, and sized from its bitmap's extent instead it came
        // out empty. The dylib caps the side, so the image is scaled up to fit.
        let edge = min(request.maximumSize.width, request.maximumSize.height)
        let tile = NSRect(x: 0, y: 0, width: edge, height: edge)
        let reply = QLThumbnailReply(contextSize: tile.size) { () -> Bool in
            guard let context = NSGraphicsContext.current else { return false }
            log.debug(
                "thumbnail context: \(context.cgContext.width)x\(context.cgContext.height) px, ctm \(String(describing: context.cgContext.ctm)), clip \(String(describing: context.cgContext.boundingBoxOfClipPath)), tile \(edge) pt")
            context.imageInterpolation = .high
            context.cgContext.draw(image, in: tile)
            return true
        }
        // Plain, so Finder lays the model over its own background as the viewer
        // draws it, rather than on the white page it gives a document. Private
        // (`iconFlavor`, Quick Look's legacy flavor numbering, 0 = plain), so set
        // only when the reply answers to it; without it the page simply returns.
        if reply.responds(to: Selector(("setIconFlavor:"))) {
            reply.setValue(0, forKey: "iconFlavor")
        }
        handler(reply, nil)
    }
}

/// The unified log (`/usr/bin/log show --predicate 'subsystem == "com.psmyles.3d-review"'`):
/// unlike the dylib's file, readable without opening the sandbox container.
private let log = Logger(subsystem: "com.psmyles.3d-review", category: "thumbnail")

/// `url`'s thumbnail, `side` pixels square, or nil when there is none.
private func thumbnail(of url: URL, side: UInt32) -> CGImage? {
    var image = ReviewQlImage()
    let drawn = url.withUnsafeFileSystemRepresentation { path -> Bool in
        guard let path else { return false }
        return review_ql_thumbnail(path, side, &image)
    }
    defer { review_ql_image_free(&image) }
    guard drawn else { return nil }
    // Straight alpha: the colour is the surface's own, the alpha its coverage.
    return makeImage(image, alpha: .last)
}
