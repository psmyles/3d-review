// The Quick Look thumbnail extension: Finder's icon-view, column and Get Info
// thumbnails of an .fbx, drawn by libreview_quicklook (crates/shell-quicklook).
//
// All this file does is the AppKit/Quick Look half: turn the request's size
// into pixels, ask the dylib for a straight-alpha RGBA image, and draw it into
// the reply's context. The scene, the textures and the GPU are the dylib's.

import CoreGraphics
import Foundation
import QuickLookThumbnailing

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
        // The dylib caps the side, so the image can be smaller than asked for;
        // the reply's size is the image's, which is never larger than the maximum.
        let size = CGSize(width: CGFloat(image.width) / scale, height: CGFloat(image.height) / scale)
        let reply = QLThumbnailReply(contextSize: size) { context -> Bool in
            context.interpolationQuality = .high
            context.draw(image, in: CGRect(origin: .zero, size: size))
            return true
        }
        handler(reply, nil)
    }
}

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
